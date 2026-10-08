//! `CBaseEntity`'s virtual methods that finish bringing an entity into the
//! level after `DispatchSpawn`, and that give it a model.
//!
//! Every game declares them before the virtual methods its own defines add,
//! as the assertions on `IsNextBot` in [`crate::entities`] describe, so their
//! slots are the generated vtable's on each ABI, as [`SPAWN_SLOT`]'s is.

use crate::entities::SPAWN_SLOT;
use crate::{vcall, vtable_slot};
use std::ffi::c_char;

// The slots match the generated vtable on each ABI, and precede the methods
// the game's defines add.
const _: () = {
	use sys::CBaseEntity__bindgen_vtable as Vtable;

	assert!(ACTIVATE_SLOT == vtable_slot!(Vtable, CBaseEntity_Activate));
	assert!(SET_MODEL_SLOT == vtable_slot!(Vtable, CBaseEntity_SetModel));
	assert!(ACTIVATE_SLOT < vtable_slot!(Vtable, CBaseEntity_IsNextBot));
	assert!(SET_MODEL_SLOT < vtable_slot!(Vtable, CBaseEntity_IsNextBot));

	// `Precache` lies between `Spawn` and `SetModel` on both ABIs.
	assert!(SET_MODEL_SLOT == SPAWN_SLOT + 2);
};

/// `CBaseEntity::Activate` in the primary vtable, the same for every game.
/// Derived from `game/server/baseentity.h` with the MSVC ABI model on Windows
/// and the Itanium ABI model on Linux.
#[doc(alias("Activate"))]
pub const ACTIVATE_SLOT: usize = cfg_select! {
	target_os = "windows" => 35,
	target_os = "linux" => 36,
};

/// `CBaseEntity::SetModel` in the primary vtable, the same for every game, two
/// slots past [`SPAWN_SLOT`]. Derived from `game/server/baseentity.h` with the
/// MSVC ABI model on Windows and the Itanium ABI model on Linux.
#[doc(alias("SetModel"))]
pub const SET_MODEL_SLOT: usize = cfg_select! {
	target_os = "windows" => 26,
	target_os = "linux" => 27,
};

/// Calls `CBaseEntity::Activate`, which the game calls on each of a level's
/// entities once they have all spawned, and `ent_create` on the entity it
/// spawns. The base class's changes the entity to its initial team, and finds
/// its damage filter by name; derived classes find the entities they refer
/// to by name, and start what they do.
///
/// # Safety
///
/// `entity` must point to a live `CBaseEntity` of the loaded game DLL, which
/// has spawned and has not been activated since, and the call must be made on
/// the server's main thread. Everything its `Activate` runs must free entities
/// only through Source's deferred deletion.
#[doc(alias("Activate"))]
pub unsafe fn activate(entity: *mut sys::CBaseEntity) {
	// SAFETY: The entity is live, and every game DLL's vtable has `Activate`
	// where the generated one does, as asserted above. The caller vouches for
	// the rest.
	unsafe { vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_Activate()) }
}

/// Calls `CBaseEntity::SetModel`, which classes override, through which the
/// entity takes a model, its index, and the collision bounds the model gives.
///
/// # Safety
///
/// `entity` must point to a live `CBaseEntity` of the loaded game DLL, and the
/// call must be made on the server's main thread. `model` must point to a
/// string naming a model the server has precached: the base class's
/// `UTIL_SetModel` stops the server with an error for any other.
#[doc(alias("SetModel", "SetEntityModel"))]
pub unsafe fn set_model(entity: *mut sys::CBaseEntity, model: *const c_char) {
	// SAFETY: As for `activate`, with `SetModel`, and the caller vouches for
	// the model.
	unsafe { vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_SetModel(model)) }
}
