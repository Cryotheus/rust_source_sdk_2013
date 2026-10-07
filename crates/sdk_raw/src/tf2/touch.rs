//! Hand-written ABI of entities touching one another: the vtable slot of
//! `CBaseEntity::Touch`, through which the engine tells an entity of each
//! entity that touches it, its signature, and the search for an entity
//! class's vtable.
//!
//! `CBaseEntity::Touch` runs the entity's touch function, which the game sets
//! with `SetTouch`, such as an item's `ItemTouch` or a dropped ammo pack's
//! `PackTouch`. Classes that act on touches themselves override it instead,
//! as `func_regenerate`'s `CRegenerateZone` does.

use crate::abi::CppDestructors;
use crate::tf2::class_targets::SlotVtables;
use crate::vtable_slot;

/// The signature of `CBaseEntity::Touch`, `void (CBaseEntity *)`, with the
/// touched entity as its receiver and the entity touching it as `other`.
#[doc(alias("Touch"))]
pub type TouchFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity, other: *mut sys::CBaseEntity);

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its entity classes whose [`TOUCH_SLOT`] entries are
/// executable.
pub type TouchVtables = SlotVtables<TOUCH_SLOT>;

// SourceMod's `gamedata/sdkhooks.games/engine.ep2v.txt` lists `Touch` in its
// `tf` section as slot 105 on Windows, and 106 on Linux, whose Itanium vtables
// start with two destructor slots instead of MSVC's one.
const _: () = assert!(TOUCH_SLOT == 104 + CppDestructors::VTABLE_SLOTS);

// The generated method has the signature of `TouchFn`.
const _: fn(&sys::CBaseEntity__bindgen_vtable) -> TouchFn = |vtable| vtable.CBaseEntity_Touch;

/// The slot of `CBaseEntity::Touch` in an entity's primary vtable, from the
/// generated binding.
#[doc(alias("Touch"))]
pub const TOUCH_SLOT: usize = vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_Touch);
