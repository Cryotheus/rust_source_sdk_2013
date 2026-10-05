//! Hand-written ABI of TF2's buildings (`CBaseObject`): the vtable slot of
//! `CBaseObject::IsPlacementPosValid`, which decides whether an Engineer's
//! blueprint may be built where it is, its signature, and the search for each
//! building class's vtable.

use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, Image};
use crate::vtable_slot;
use std::ffi::c_void;
use std::ptr::NonNull;

/// The signature of `CBaseObject::IsPlacementPosValid`, `bool ()`, with the
/// building as its receiver, which returns whether the building may be built
/// where its blueprint is.
///
/// The generated method takes a `CBaseObject` receiver. It is the building's
/// primary base, `CBaseEntity`, at the same address, as
/// [`entities::health`](crate::entities::health) asserts, so the method can
/// be called and hooked with an entity receiver.
#[doc(alias("IsPlacementPosValid"))]
pub type IsPlacementPosValidFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity) -> bool;

// The generated method takes no argument and returns a `bool`.
const _: fn(
	&sys::CBaseObject__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CBaseObject) -> bool =
	|vtable| vtable.CBaseObject_IsPlacementPosValid;

/// The slot of `CBaseObject::IsPlacementPosValid` in a TF2 building's primary
/// vtable, from the generated binding. The building classes that override it,
/// such as the teleporter's, keep the slot.
#[doc(alias("IsPlacementPosValid"))]
pub const IS_PLACEMENT_POS_VALID_SLOT: usize = vtable_slot!(
	sys::CBaseObject__bindgen_vtable,
	CBaseObject_IsPlacementPosValid
);

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its building classes.
#[derive(Debug, Clone)]
pub struct ObjectVtables(Image);

impl ObjectVtables {
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
	/// as `CObjectSentrygun`, whose [`IS_PLACEMENT_POS_VALID_SLOT`] entry is
	/// executable, from its run-time type information. Returns `None` if there
	/// is no such table or more than one.
	///
	/// The address is metadata from the snapshot: it does not keep the module
	/// loaded, and the table is the class's only while the module that
	/// [`Self::load`] snapshot stays loaded.
	///
	/// Each search reads the whole snapshot a few times, so find the classes
	/// needed together with [`Self::find_all`].
	pub fn find(&self, class: &str) -> Option<NonNull<*mut c_void>> {
		NonNull::new(self.0.primary_vtable(class, IS_PLACEMENT_POS_VALID_SLOT)? as *mut *mut c_void)
	}

	/// The tables [`Self::find`] finds for each of `classes`, in their order.
	/// The snapshot is read as often for every class as [`Self::find`] reads it
	/// for one.
	pub fn find_all(&self, classes: &[&str]) -> Vec<Option<NonNull<*mut c_void>>> {
		self.0
			.primary_vtables(classes, IS_PLACEMENT_POS_VALID_SLOT)
			.into_iter()
			.map(|table| NonNull::new(table? as *mut *mut c_void))
			.collect()
	}
}
