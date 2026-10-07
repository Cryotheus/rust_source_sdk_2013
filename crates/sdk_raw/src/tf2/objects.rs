//! Hand-written ABI of TF2's buildings (`CBaseObject`): the vtable slot of
//! `CBaseObject::IsPlacementPosValid`, which decides whether an Engineer's
//! blueprint may be built where it is, its signature, and the search for each
//! building class's vtable.

use crate::tf2::class_targets::SlotVtables;
use crate::vtable_slot;

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

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its building classes: those whose
/// [`IS_PLACEMENT_POS_VALID_SLOT`] entries are executable.
pub type ObjectVtables = SlotVtables<IS_PLACEMENT_POS_VALID_SLOT>;

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
