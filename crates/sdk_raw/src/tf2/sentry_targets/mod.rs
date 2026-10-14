//! TF2's sentry targeting: `CObjectSentrygun::FindTarget`
//! (`game/server/tf/tf_obj_sentrygun.cpp`), which a sentry runs each time it
//! looks for something to shoot, and the values it keeps about its target.
//!
//! `FindTarget` goes through the dummies, the enemy team's players, the
//! non-player bots (`INextBot`s) and the enemy team's buildings, in that
//! order, and keeps the target it has, unless a new one is closer by a quarter.
//! It returns whether the sentry has a target, after it set `m_hEnemy` and the
//! sentry's state through `FoundTarget`. A sentry that has none goes on
//! searching; `Attack` clears `m_hEnemy` and returns to searching when
//! `FindTarget` returns `false`. The function is not virtual and, like
//! `FoundTarget`, which it calls, cannot be hooked through a vtable. The game
//! exports neither, so [`SentryTargets::resolve`] finds `FindTarget` in the game
//! server module: on Windows through a signature, and on Linux through its
//! mangled symbol, which needs an unstripped `server_srv.so`. A missing or
//! ambiguous signature, a stripped symbol, or a file that differs from the
//! loaded module fail closed. On Windows, a `FindTarget` whose start a detour
//! replaced with a jump is found too: KHook keeps its detour after the last
//! hook goes, so a plugin that hooked it and was unloaded leaves one for its
//! next load.
//!
//! `FindTarget` is a member function with no arguments that returns a C++
//! `bool`, and is called as `extern "C"` with the sentry as its first argument,
//! its calling convention on both supported x86-64 targets.
//!
//! # Caching
//!
//! [`SentryTargets::cached`] keeps the last successful resolution for the rest
//! of the process in a [`ModuleCache`], keyed as
//! [`ServerRagdolls::cached`](crate::tf2::ragdolls::ServerRagdolls::cached) is,
//! under the same assumption. Failures are not kept.

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

/// The signature of `CObjectSentrygun::FindTarget`, `bool ()`, with the sentry
/// as its receiver, which returns whether the sentry has a target, after it
/// chose one.
///
/// The generated method takes a `CObjectSentrygun` receiver. It is the
/// sentry's primary base, `CBaseEntity`, at the same address, as
/// [`buildings`](crate::tf2::buildings) describes for every building method,
/// so the function can be hooked with an entity receiver.
#[doc(alias("FindTarget"))]
pub type FindTargetFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity) -> bool;

/// `SENTRY_THINK_DELAY` from `game/server/tf/tf_obj_sentrygun.cpp`, the seconds
/// between a sentry's thinks, which `FoundTarget` adds to a new target's
/// `m_flNextAttack`.
pub const SENTRY_THINK_DELAY: f32 = 0.05;

/// The last resolution [`SentryTargets::cached`] kept, with the module it was
/// resolved in.
static CACHE: ModuleCache<usize> = ModuleCache::new();

/// TF2's `CObjectSentrygun::FindTarget`, resolved in a game server module.
///
/// It is a plain function address the resolver verified, so it is `Copy`,
/// `Send`, and `Sync`. Holding one does not keep that module loaded: the
/// function may only be hooked or called while the module it was resolved in
/// stays loaded.
#[doc(alias("FindTarget"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SentryTargets {
	find_target: NonNull<c_void>,
}

impl SentryTargets {
	/// Finds `FindTarget` in the module whose `CreateInterface` export is
	/// `factory`, as [`Self::resolve`] does, but inspects each module only once,
	/// as the [module documentation](self#caching) describes.
	///
	/// Fails with [`SentryTargetsError::Unresolved`] if no loaded module contains
	/// `factory`, or if resolution fails.
	///
	/// # Safety
	///
	/// - `factory` must be the `CreateInterface` export of a module that stays
	///   loaded, with its image mappings unchanged, for the whole call.
	/// - If an earlier call resolved a module mapped at the same base, with its
	///   `CreateInterface` at the same address, the module containing `factory`
	///   must be that same image, with the code that call found unchanged. This
	///   holds under the assumption [`ModuleCache`] describes.
	pub unsafe fn cached(factory: CreateInterfaceFn) -> Result<Self, SentryTargetsError> {
		let factory = factory as usize;

		// SAFETY: The caller keeps the factory's module loaded for this call.
		let key = unsafe { ModuleKey::of(factory) }.map_err(|_| SentryTargetsError::Unresolved)?;

		let address = CACHE.get_or_resolve(key, || {
			// SAFETY: The caller keeps the factory's module loaded, with its image
			// mappings unchanged, for this call.
			unsafe { platform::resolve(factory) }.ok_or(SentryTargetsError::Unresolved)
		})?;

		Self::from_address(address)
	}

	fn from_address(address: usize) -> Result<Self, SentryTargetsError> {
		NonNull::new(ptr::with_exposed_provenance_mut(address))
			.map(|find_target| Self { find_target })
			.ok_or(SentryTargetsError::Unresolved)
	}

	/// Finds `FindTarget` in the module whose `CreateInterface` export is
	/// `factory`.
	///
	/// Fails with [`SentryTargetsError::Unresolved`] unless that module is a game
	/// server module whose function matches retail TF2's. Nothing is cached.
	///
	/// # Safety
	///
	/// `factory` must be the `CreateInterface` export of a module that stays
	/// loaded, with its image mappings unchanged, for the whole call.
	pub unsafe fn resolve(factory: CreateInterfaceFn) -> Result<Self, SentryTargetsError> {
		// SAFETY: The caller keeps the factory's module loaded, with its image
		// mappings unchanged, for this call.
		let address =
			unsafe { platform::resolve(factory as usize) }.ok_or(SentryTargetsError::Unresolved)?;

		Self::from_address(address)
	}

	/// The entry of `FindTarget`, a [`FindTargetFn`], in the module it was
	/// resolved in.
	pub const fn find_target(self) -> NonNull<c_void> {
		self.find_target
	}
}

// SAFETY: A plain function address, which nothing here dereferences.
unsafe impl Send for SentryTargets {}

// SAFETY: As above.
unsafe impl Sync for SentryTargets {}

/// Why TF2's `CObjectSentrygun::FindTarget` could not be found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum SentryTargetsError {
	/// The game server module's `FindTarget` was not found, or did not match
	/// retail TF2's, as the [module documentation](self) describes.
	#[error("the game's CObjectSentrygun::FindTarget could not be found")]
	Unresolved,
}
