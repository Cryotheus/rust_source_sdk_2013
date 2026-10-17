//! The animation layers of the game's `CBaseAnimatingOverlay`s, such as its
//! flex entities, players, NextBots and buildings: gestures that play over
//! the entity's own sequence, as a player's attacks play over their run
//! (`game/server/BaseAnimatingOverlay.cpp`).
//!
//! [`AnimationLayers::restart_gesture`] calls the game's
//! `CBaseAnimatingOverlay::RestartGesture`, which plays a gesture of an
//! activity from its start: it restarts the layer that already plays that
//! activity's gesture, or adds one. A layer that plays to its end with
//! `autokill` set fades out and is removed by the entity's
//! `StudioFrameAdvance`, which also advances every layer's cycle. The server
//! networks each layer's sequence, cycle, weight and order, so clients show
//! the layers of any entity whose send table includes
//! `DT_BaseAnimatingOverlay`'s, but not TF2's players, whose clients animate
//! them themselves.
//!
//! The function is not virtual, and the game exports it from neither module,
//! so [`AnimationLayers::resolve`] finds it in the game server module: on
//! Windows through a signature, and on Linux through its mangled symbol, which
//! needs an unstripped `server_srv.so`. A missing or ambiguous signature, a
//! stripped symbol, or a file that differs from the loaded module fail closed.
//!
//! `RestartGesture` takes the activity as a C `int` and two C++ `bool`s, and
//! is called as `extern "C"` with the entity as its first argument, its
//! calling convention on both supported x86-64 targets. The entity's primary
//! base, `CBaseEntity`, is at the same address.
//!
//! # Caching
//!
//! [`AnimationLayers::cached`] keeps the last successful resolution for the
//! rest of the process in a [`ModuleCache`], as
//! [`MedigunTargets::cached`](crate::tf2::medigun_targets::MedigunTargets::cached)
//! does, under the same assumption. Failures are not kept.

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;

use crate::interfaces::CreateInterfaceFn;
use crate::util::{ModuleCache, ModuleKey};
use std::ffi::{c_int, c_void};
use std::mem::transmute;
use std::ptr::{self, NonNull};

/// `CBaseAnimatingOverlay::RestartGesture(Activity, bool addifmissing, bool
/// autokill)`, with the entity as its receiver.
#[doc(alias("RestartGesture"))]
pub type RestartGestureFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	activity: c_int,
	add_if_missing: bool,
	autokill: bool,
);

/// The last resolution [`AnimationLayers::cached`] kept, with the module it was
/// resolved in.
static CACHE: ModuleCache<usize> = ModuleCache::new();

/// TF2's `CBaseAnimatingOverlay::RestartGesture`, resolved in a game server
/// module.
///
/// It is a plain function address the resolver verified, so it is `Copy`,
/// `Send`, and `Sync`. Holding one does not keep that module loaded: the
/// function may only be called while the module it was resolved in stays
/// loaded.
#[doc(alias("CBaseAnimatingOverlay"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimationLayers {
	restart_gesture: NonNull<c_void>,
}

impl AnimationLayers {
	/// Finds `RestartGesture` in the module whose `CreateInterface` export is
	/// `factory`, as [`Self::resolve`] does, but inspects each module only
	/// once, as the [module documentation](self#caching) describes.
	///
	/// Fails with [`AnimationLayersError::Unresolved`] if no loaded module
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
	pub unsafe fn cached(factory: CreateInterfaceFn) -> Result<Self, AnimationLayersError> {
		let factory = factory as usize;

		// SAFETY: The caller keeps the factory's module loaded for this call.
		let key =
			unsafe { ModuleKey::of(factory) }.map_err(|_| AnimationLayersError::Unresolved)?;

		let address = CACHE.get_or_resolve(key, || {
			// SAFETY: The caller keeps the factory's module loaded, with its image
			// mappings unchanged, for this call.
			unsafe { platform::resolve(factory) }.ok_or(AnimationLayersError::Unresolved)
		})?;

		Self::from_address(address)
	}

	fn from_address(address: usize) -> Result<Self, AnimationLayersError> {
		NonNull::new(ptr::with_exposed_provenance_mut(address))
			.map(|restart_gesture| Self { restart_gesture })
			.ok_or(AnimationLayersError::Unresolved)
	}

	/// Finds `RestartGesture` in the module whose `CreateInterface` export is
	/// `factory`.
	///
	/// Fails with [`AnimationLayersError::Unresolved`] unless that module is a
	/// game server module whose function matches retail TF2's. Nothing is
	/// cached.
	///
	/// # Safety
	///
	/// `factory` must be the `CreateInterface` export of a module that stays
	/// loaded, with its image mappings unchanged, for the whole call.
	pub unsafe fn resolve(factory: CreateInterfaceFn) -> Result<Self, AnimationLayersError> {
		// SAFETY: The caller keeps the factory's module loaded, with its image
		// mappings unchanged, for this call.
		let address = unsafe { platform::resolve(factory as usize) }
			.ok_or(AnimationLayersError::Unresolved)?;

		Self::from_address(address)
	}

	/// Plays the gesture of `activity` on `entity` from its start: restarts the
	/// layer that plays it already, or, if none does and `add_if_missing` is
	/// set, adds a layer for the sequence the entity's model picks for the
	/// activity, which `autokill` removes once it has played. A model without a
	/// sequence for the activity gets no layer, and the game says so in the
	/// console as a developer message.
	///
	/// # Safety
	///
	/// - The module this was resolved in, by [`Self::resolve`] or
	///   [`Self::cached`], is still loaded, with its image mappings unchanged,
	///   for the whole call.
	/// - The call is made on the server's main thread.
	/// - `entity` points to a live `CBaseAnimatingOverlay` of that module, not
	///   marked for deletion.
	#[doc(alias("RestartGesture"))]
	pub unsafe fn restart_gesture(
		&self,
		entity: NonNull<sys::CBaseEntity>,
		activity: c_int,
		add_if_missing: bool,
		autokill: bool,
	) {
		// SAFETY: The resolver verified that the address is the entry of
		// `RestartGesture`, whose prototype this is.
		let restart =
			unsafe { transmute::<*mut c_void, RestartGestureFn>(self.restart_gesture.as_ptr()) };

		// SAFETY: The caller keeps the module loaded, calls on the main thread,
		// and vouches for the entity, whose layers the call reads and changes.
		unsafe { restart(entity.as_ptr(), activity, add_if_missing, autokill) };
	}
}

// SAFETY: A plain function address, which only the unsafe `restart_gesture`
// calls.
unsafe impl Send for AnimationLayers {}

// SAFETY: As above.
unsafe impl Sync for AnimationLayers {}

/// Why TF2's `CBaseAnimatingOverlay::RestartGesture` could not be found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum AnimationLayersError {
	/// The game server module's `RestartGesture` was not found, or did not
	/// match retail TF2's, as the [module documentation](self) describes.
	#[error("the game's CBaseAnimatingOverlay::RestartGesture could not be found")]
	Unresolved,
}
