//! TF2's respawn after a loadout change: `CTFPlayer::CheckInstantLoadoutRespawn`
//! (`game/server/tf/tf_player.cpp`).
//!
//! A player's loadout lives in their Steam inventory, so the server hears of a
//! change from the game coordinator, not from the player: its
//! `k_EMsgGCRespawnPostLoadoutChange` message runs the job
//! `CGC_RespawnPostLoadoutChange`, which calls `CheckInstantLoadoutRespawn` on
//! the player who made the change. The function respawns a living player with
//! their new loadout (`ForceRegenerateAndRespawn`) only while they stand in a
//! respawn room of their own team, outside a stalemate's and arena's limits,
//! and if their class's loadout changed; otherwise the new loadout waits for
//! their next respawn or resupply.
//!
//! The function is not virtual, and the game exports it on neither target, so
//! [`LoadoutRespawn::resolve`] finds it in the game server module: on Windows
//! through a signature, and on Linux through its mangled symbol, which needs an
//! unstripped `server_srv.so`. A missing or ambiguous signature, a stripped
//! symbol, or a file that differs from the loaded module fail closed. On
//! Windows, an entry a KHook detour replaced with a jump is found too, as KHook
//! keeps its detour after the last hook goes.
//!
//! `CheckInstantLoadoutRespawn` is a member function with no arguments that
//! returns nothing, called as `extern "C"` with the player as its first
//! argument, the calling convention on both supported x86-64 targets.
//!
//! # Caching
//!
//! [`LoadoutRespawn::cached`] keeps the last successful resolution for the
//! rest of the process in a [`ModuleCache`], as
//! [`SentryTargets::cached`](crate::tf2::sentry_targets::SentryTargets::cached)
//! does. Failures are not kept.

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

/// The signature of `CTFPlayer::CheckInstantLoadoutRespawn`, `void ()`, with
/// the player as its receiver.
///
/// The generated method takes a `CTFPlayer` receiver, whose entity base
/// [`crate::tf2`] asserts is at offset zero, so the function can be hooked with
/// an entity receiver.
#[doc(alias("CheckInstantLoadoutRespawn"))]
pub type CheckInstantLoadoutRespawnFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity);

/// The last resolution [`LoadoutRespawn::cached`] kept, with the module it was
/// resolved in.
static CACHE: ModuleCache<usize> = ModuleCache::new();

/// TF2's `CTFPlayer::CheckInstantLoadoutRespawn`, resolved in a game server
/// module.
///
/// It is a plain function address the resolver verified, so it is `Copy`,
/// `Send`, and `Sync`. Holding one does not keep that module loaded: the
/// function may only be hooked or called while the module it was resolved in
/// stays loaded.
#[doc(alias("CheckInstantLoadoutRespawn"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LoadoutRespawn {
	check: NonNull<c_void>,
}

impl LoadoutRespawn {
	/// Finds `CheckInstantLoadoutRespawn` in the module whose `CreateInterface`
	/// export is `factory`, as [`Self::resolve`] does, but inspects each module
	/// only once, as the [module documentation](self#caching) describes.
	///
	/// # Safety
	///
	/// As for
	/// [`SentryTargets::cached`](crate::tf2::sentry_targets::SentryTargets::cached).
	pub unsafe fn cached(factory: CreateInterfaceFn) -> Result<Self, LoadoutRespawnError> {
		let factory = factory as usize;

		// SAFETY: The caller keeps the factory's module loaded for this call.
		let key = unsafe { ModuleKey::of(factory) }.map_err(|_| LoadoutRespawnError::Unresolved)?;

		let address = CACHE.get_or_resolve(key, || {
			// SAFETY: The caller keeps the factory's module loaded, with its image
			// mappings unchanged, for this call.
			unsafe { platform::resolve(factory) }.ok_or(LoadoutRespawnError::Unresolved)
		})?;

		Self::from_address(address)
	}

	fn from_address(address: usize) -> Result<Self, LoadoutRespawnError> {
		NonNull::new(ptr::with_exposed_provenance_mut(address))
			.map(|check| Self { check })
			.ok_or(LoadoutRespawnError::Unresolved)
	}

	/// Finds `CheckInstantLoadoutRespawn` in the module whose `CreateInterface`
	/// export is `factory`. Nothing is cached.
	///
	/// # Safety
	///
	/// `factory` must be the `CreateInterface` export of a module that stays
	/// loaded, with its image mappings unchanged, for the whole call.
	pub unsafe fn resolve(factory: CreateInterfaceFn) -> Result<Self, LoadoutRespawnError> {
		// SAFETY: The caller keeps the factory's module loaded, with its image
		// mappings unchanged, for this call.
		let address = unsafe { platform::resolve(factory as usize) }
			.ok_or(LoadoutRespawnError::Unresolved)?;

		Self::from_address(address)
	}

	/// The entry of `CheckInstantLoadoutRespawn`, a
	/// [`CheckInstantLoadoutRespawnFn`], in the module it was resolved in.
	pub const fn check_instant_loadout_respawn(self) -> NonNull<c_void> {
		self.check
	}
}

// SAFETY: A plain function address, which nothing here dereferences.
unsafe impl Send for LoadoutRespawn {}

// SAFETY: As above.
unsafe impl Sync for LoadoutRespawn {}

/// Why TF2's `CTFPlayer::CheckInstantLoadoutRespawn` could not be found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum LoadoutRespawnError {
	/// The game server module's `CheckInstantLoadoutRespawn` was not found, or
	/// did not match retail TF2's, as the [module documentation](self)
	/// describes.
	#[error("the game's CTFPlayer::CheckInstantLoadoutRespawn could not be found")]
	Unresolved,
}
