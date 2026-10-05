//! Hand-written ABI of the game's entity factories, `IEntityFactory` in
//! `game/server/util.h`: the vtable slot of `Create`, which constructs an
//! entity, and its signature.
//!
//! Each class name the game DLL links to an entity class
//! (`LINK_ENTITY_TO_CLASS`) gets a factory of its own: a static
//! `CEntityFactory<T>` of the game DLL, installed in the entity factory
//! dictionary under the name. `CreateEntityByName`, through which the game
//! creates every entity it creates by name, finds the factory in the dictionary
//! and calls its `Create` through its vtable. Factories of the same entity
//! class share a vtable.

use crate::vtable_slot;
use std::ffi::c_char;

/// The signature of `IEntityFactory::Create`,
/// `IServerNetworkable *(const char *)`, with the factory as its receiver and
/// the class name the entity is being created by. It returns the new entity's
/// networkable, or null if it created nothing.
#[doc(alias("Create"))]
pub type CreateFn = unsafe extern "C" fn(
	this: *mut sys::IEntityFactory,
	class_name: *const c_char,
) -> *mut sys::IServerNetworkable;

// `IEntityFactory` declares no destructor, so `Create`, its first method, is
// its first slot under both ABIs.
const _: () = assert!(CREATE_SLOT == 0);

// The generated method has the same receiver, parameter and return type.
const _: fn(&sys::IEntityFactory__bindgen_vtable) -> CreateFn =
	|vtable| vtable.IEntityFactory_Create;

/// The slot of `IEntityFactory::Create` in a factory's vtable, from the
/// generated binding.
#[doc(alias("Create"))]
pub const CREATE_SLOT: usize =
	vtable_slot!(sys::IEntityFactory__bindgen_vtable, IEntityFactory_Create);
