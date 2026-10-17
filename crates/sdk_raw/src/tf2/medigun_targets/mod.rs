//! TF2's Medi Gun targeting: `CWeaponMedigun::AllowedToHealTarget`
//! (`game/shared/tf/tf_weapon_medigun.cpp`), which a Medi Gun asks of each
//! entity it might heal, as it looks for a target and while it heals one.
//!
//! The game allows a living player on the Medi Gun owner's team, a Spy of the
//! other team disguised as the owner's, and a revive marker of the owner's
//! team; never a cloaked player, unless the owner is cloaked too, nor one whose
//! active weapon blocks healing (`weapon_blocks_healing`). The Medi Gun's
//! search traces from its owner's eyes without a team filter, so what this
//! function allows is all that decides whom it heals; a target it no longer
//! allows is dropped. The function is not virtual, and the game exports it
//! from neither module, so [`MedigunTargets::resolve`] finds it in the game
//! server module: on Windows through a signature, and on Linux through its
//! mangled symbol, which needs an unstripped `server_srv.so`. A missing or
//! ambiguous signature, a stripped symbol, or a file that differs from the
//! loaded module fail closed. On Windows, a function whose start a detour
//! replaced with a jump is found too, as for
//! [`SentryTargets`](crate::tf2::sentry_targets::SentryTargets).
//!
//! `AllowedToHealTarget` is a member function that takes the candidate entity
//! and returns a C++ `bool`, and is called as `extern "C"` with the Medi Gun as
//! its first argument, its calling convention on both supported x86-64
//! targets.
//!
//! # Caching
//!
//! [`MedigunTargets::cached`] keeps the last successful resolution for the rest
//! of the process in a [`ModuleCache`], as
//! [`SentryTargets::cached`](crate::tf2::sentry_targets::SentryTargets::cached)
//! does, under the same assumption. Failures are not kept.

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;

use crate::interfaces::CreateInterfaceFn;
use crate::util::{ModuleCache, ModuleKey};
use std::ffi::c_void;
use std::ptr::{self, NonNull};

/// The signature of `CWeaponMedigun::AllowedToHealTarget`,
/// `bool (CBaseEntity *pTarget)`, with the Medi Gun as its receiver, which
/// returns whether the Medi Gun may heal `target`.
///
/// The Medi Gun's primary base, `CBaseEntity`, is at the same address, so the
/// function can be hooked with an entity receiver.
#[doc(alias("AllowedToHealTarget"))]
pub type AllowedToHealTargetFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, target: *mut sys::CBaseEntity) -> bool;

/// The last resolution [`MedigunTargets::cached`] kept, with the module it was
/// resolved in.
static CACHE: ModuleCache<usize> = ModuleCache::new();

/// TF2's `CWeaponMedigun::AllowedToHealTarget`, resolved in a game server
/// module.
///
/// It is a plain function address the resolver verified, so it is `Copy`,
/// `Send`, and `Sync`. Holding one does not keep that module loaded: the
/// function may only be hooked or called while the module it was resolved in
/// stays loaded.
#[doc(alias("AllowedToHealTarget"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MedigunTargets {
	allowed_to_heal_target: NonNull<c_void>,
}

impl MedigunTargets {
	/// Finds `AllowedToHealTarget` in the module whose `CreateInterface` export
	/// is `factory`, as [`Self::resolve`] does, but inspects each module only
	/// once, as the [module documentation](self#caching) describes.
	///
	/// Fails with [`MedigunTargetsError::Unresolved`] if no loaded module
	/// contains `factory`, or if resolution fails.
	///
	/// # Safety
	///
	/// - `factory` must be the `CreateInterface` export of a module that stays
	///   loaded, with its image mappings unchanged, for the whole call.
	/// - If an earlier call resolved a module mapped at the same base, with its
	///   `CreateInterface` at the same address, the module containing `factory`
	///   must be that same image, with the code that call found unchanged. This
	///   holds under the assumption [`ModuleCache`] describes.
	pub unsafe fn cached(factory: CreateInterfaceFn) -> Result<Self, MedigunTargetsError> {
		let factory = factory as usize;

		// SAFETY: The caller keeps the factory's module loaded for this call.
		let key = unsafe { ModuleKey::of(factory) }.map_err(|_| MedigunTargetsError::Unresolved)?;

		let address = CACHE.get_or_resolve(key, || {
			// SAFETY: The caller keeps the factory's module loaded, with its image
			// mappings unchanged, for this call.
			unsafe { platform::resolve(factory) }.ok_or(MedigunTargetsError::Unresolved)
		})?;

		Self::from_address(address)
	}

	fn from_address(address: usize) -> Result<Self, MedigunTargetsError> {
		NonNull::new(ptr::with_exposed_provenance_mut(address))
			.map(|allowed_to_heal_target| Self {
				allowed_to_heal_target,
			})
			.ok_or(MedigunTargetsError::Unresolved)
	}

	/// Finds `AllowedToHealTarget` in the module whose `CreateInterface` export
	/// is `factory`.
	///
	/// Fails with [`MedigunTargetsError::Unresolved`] unless that module is a
	/// game server module whose function matches retail TF2's. Nothing is
	/// cached.
	///
	/// # Safety
	///
	/// `factory` must be the `CreateInterface` export of a module that stays
	/// loaded, with its image mappings unchanged, for the whole call.
	pub unsafe fn resolve(factory: CreateInterfaceFn) -> Result<Self, MedigunTargetsError> {
		// SAFETY: The caller keeps the factory's module loaded, with its image
		// mappings unchanged, for this call.
		let address = unsafe { platform::resolve(factory as usize) }
			.ok_or(MedigunTargetsError::Unresolved)?;

		Self::from_address(address)
	}

	/// The entry of `AllowedToHealTarget`, an [`AllowedToHealTargetFn`], in the
	/// module it was resolved in.
	pub const fn allowed_to_heal_target(self) -> NonNull<c_void> {
		self.allowed_to_heal_target
	}
}

// SAFETY: A plain function address, which nothing here dereferences.
unsafe impl Send for MedigunTargets {}

// SAFETY: As above.
unsafe impl Sync for MedigunTargets {}

/// Why TF2's `CWeaponMedigun::AllowedToHealTarget` could not be found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum MedigunTargetsError {
	/// The game server module's `AllowedToHealTarget` was not found, or did not
	/// match retail TF2's, as the [module documentation](self) describes.
	#[error("the game's CWeaponMedigun::AllowedToHealTarget could not be found")]
	Unresolved,
}
