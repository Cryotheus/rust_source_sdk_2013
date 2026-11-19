//! Hand-written ABI of `IStaticPropMgrServer`, which the generated bindings do
//! not describe: the engine's manager of the level's static props, from
//! `public/engine/IStaticPropMgr.h`.
//!
//! # The order of the vtable
//!
//! `IStaticPropMgr` declares no destructor and has no base, so its vtable
//! starts with its first method under both the MSVC and Itanium ABIs.
//! `IStaticPropMgrServer` derives from it alone and adds its own methods
//! after those of `IStaticPropMgr`, as both ABIs place the new virtual
//! methods of a class after those of its primary base.
//!
//! Itanium gives each virtual method the slot of its declaration, in order.
//! MSVC does too, except that it gathers the overloads of one name together at
//! the place of the first of them and lays them out in the reverse of the
//! order they are declared in. The header declares two overloads of one name,
//! `IsStaticProp`, one right after the other, so the two ABIs differ only in
//! which of the third and fourth slots holds which. Every other method keeps
//! its slot under both:
//!
//! | Slot | Method |
//! |---|---|
//! | 0 | `CreateVPhysicsRepresentations` |
//! | 1 | `TraceRayAgainstStaticProp` |
//! | 2, 3 | the two `IsStaticProp`s, in either order |
//! | 4 | `GetStaticPropByIndex` |
//! | 5 | `GetAllStaticProps` |
//! | 6 | `GetAllStaticPropsInAABB` |
//! | 7 | `GetAllStaticPropsInOBB` |
//!
//! Only slots 4 and 5 have signatures here. The others are opaque, so no call
//! depends on the order of the overloads.

use crate::vtable_slot;
use std::ffi::{CStr, c_int, c_void};

/// The list that [`GetAllStaticPropsFn`] fills: a `CUtlVector<ICollideable *>`.
pub type CollideableList =
	sys::CUtlVector<*mut sys::ICollideable, sys::CUtlMemory<*mut sys::ICollideable>>;

/// `void IStaticPropMgrServer::GetAllStaticProps(CUtlVector<ICollideable *>
/// *)`: adds the collideable of each of the level's static props to the end of
/// a list, growing its memory as it needs to with the engine's own allocator.
#[doc(alias("GetAllStaticProps"))]
pub type GetAllStaticPropsFn =
	unsafe extern "C" fn(this: *mut IStaticPropMgrServer, list: *mut CollideableList);

/// `ICollideable *IStaticPropMgr::GetStaticPropByIndex(int)`: the collideable
/// of the level's static prop at an index. The header promises nothing for
/// an index past the last prop.
#[doc(alias("GetStaticPropByIndex"))]
pub type GetStaticPropByIndexFn =
	unsafe extern "C" fn(this: *mut IStaticPropMgrServer, index: c_int) -> *mut sys::ICollideable;

const _: () = {
	assert!(
		vtable_slot!(IStaticPropMgrServerVtable, get_static_prop_by_index)
			== GET_STATIC_PROP_BY_INDEX_SLOT
	);
	assert!(
		vtable_slot!(IStaticPropMgrServerVtable, get_all_static_props) == GET_ALL_STATIC_PROPS_SLOT
	);
	assert!(size_of::<IStaticPropMgrServerVtable>() == 8 * size_of::<*const c_void>());
};

/// The vtable slot of `IStaticPropMgrServer::GetAllStaticProps`, the same
/// under both ABIs (see the [module documentation](self)).
///
/// The header calls `GetAllStaticProps` a testing function that may be
/// removed. Removed without a new version of the interface, it would leave
/// `GetAllStaticPropsInAABB` in this slot, so the slot is only trusted for the
/// interface asked for by [`VERSION`].
#[doc(alias("GetAllStaticProps"))]
pub const GET_ALL_STATIC_PROPS_SLOT: usize = 5;

/// The vtable slot of `IStaticPropMgr::GetStaticPropByIndex`, the same under
/// both ABIs (see the [module documentation](self)).
#[doc(alias("GetStaticPropByIndex"))]
pub const GET_STATIC_PROP_BY_INDEX_SLOT: usize = 4;

/// The version string `IStaticPropMgrServer` is exported and requested
/// under, by the engine's factory.
///
/// This is `INTERFACEVERSION_STATICPROPMGR_SERVER` from
/// `public/engine/IStaticPropMgr.h`.
#[doc(alias("INTERFACEVERSION_STATICPROPMGR_SERVER"))]
pub const VERSION: &CStr = c"StaticPropMgrServer002";

/// `IStaticPropMgrServer`, the engine's manager of the level's static props,
/// as the game server sees it.
#[repr(C)]
pub struct IStaticPropMgrServer {
	/// The pointer to the object's vtable.
	pub vtable_: *const IStaticPropMgrServerVtable,
}

/// The vtable of `IStaticPropMgrServer`. Only the methods this crate calls
/// have their signatures; the others are opaque.
#[repr(C)]
pub struct IStaticPropMgrServerVtable {
	/// `CreateVPhysicsRepresentations`.
	pub create_vphysics_representations: *const c_void,

	/// `TraceRayAgainstStaticProp`.
	pub trace_ray_against_static_prop: *const c_void,

	/// The two overloads of `IsStaticProp`, whose order differs between the
	/// ABIs.
	pub is_static_prop: [*const c_void; 2],

	/// `GetStaticPropByIndex`.
	pub get_static_prop_by_index: GetStaticPropByIndexFn,

	/// `GetAllStaticProps`.
	pub get_all_static_props: GetAllStaticPropsFn,

	/// `GetAllStaticPropsInAABB`.
	pub get_all_static_props_in_aabb: *const c_void,

	/// `GetAllStaticPropsInOBB`.
	pub get_all_static_props_in_obb: *const c_void,
}
