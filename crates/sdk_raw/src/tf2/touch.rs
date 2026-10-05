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
use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, Image};
use crate::vtable_slot;
use std::ffi::c_void;
use std::ptr::NonNull;

/// The signature of `CBaseEntity::Touch`, `void (CBaseEntity *)`, with the
/// touched entity as its receiver and the entity touching it as `other`.
#[doc(alias("Touch"))]
pub type TouchFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity, other: *mut sys::CBaseEntity);

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

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its entity classes.
#[derive(Debug, Clone)]
pub struct TouchVtables(Image);

impl TouchVtables {
	/// Snapshots the module whose `CreateInterface` export is `factory`, such
	/// as the game server module.
	///
	/// # Safety
	///
	/// `factory` must be the `CreateInterface` export of a module that stays
	/// loaded throughout this call.
	pub unsafe fn load(factory: CreateInterfaceFn) -> Result<Self, util::Error> {
		// SAFETY: The factory is an executable address in its module, which the
		// caller keeps loaded while it is inspected.
		unsafe { Image::load(factory as usize) }.map(Self)
	}

	/// The unique primary vtable of the global C++ class named `class`, such
	/// as `CTFAmmoPack`, whose [`TOUCH_SLOT`] entry is executable, from its
	/// run-time type information. Returns `None` if there is no such table or
	/// more than one.
	///
	/// The table is only the class's own: classes deriving from it have
	/// tables of their own. The address is metadata from the snapshot: it does
	/// not keep the module loaded, and the table is the class's only while the
	/// module that [`Self::load`] snapshot stays loaded.
	pub fn find(&self, class: &str) -> Option<NonNull<*mut c_void>> {
		NonNull::new(self.0.primary_vtable(class, TOUCH_SLOT)? as *mut *mut c_void)
	}

	/// The tables [`Self::find`] finds for each of `classes`, in their order.
	/// Each search reads the whole snapshot a few times, and this reads it as
	/// often for every class as [`Self::find`] does for one.
	pub fn find_all(&self, classes: &[&str]) -> Vec<Option<NonNull<*mut c_void>>> {
		self.0
			.primary_vtables(classes, TOUCH_SLOT)
			.into_iter()
			.map(|table| NonNull::new(table? as *mut *mut c_void))
			.collect()
	}
}
