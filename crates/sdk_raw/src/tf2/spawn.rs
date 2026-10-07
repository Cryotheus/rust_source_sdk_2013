//! Hand-written ABI of entities spawning: the search for an entity class's
//! vtable, through which the game calls `CBaseEntity::Spawn` on the class's
//! entities.
//!
//! The game spawns an entity once it created it and gave it its keys, through
//! `DispatchSpawn`, which calls `Spawn` through the entity's vtable. Every
//! game has the method at the same slot, [`SPAWN_SLOT`], with the signature
//! [`SpawnFn`](crate::entities::SpawnFn).

use crate::entities::SPAWN_SLOT;
use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, Image};
use std::ffi::c_void;
use std::ptr::NonNull;

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its entity classes, through which the game calls
/// `Spawn` on their entities.
#[derive(Debug, Clone)]
pub struct SpawnVtables(Image);

impl SpawnVtables {
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
	/// as `CTFFlameManager`, whose [`SPAWN_SLOT`] entry is executable, from its
	/// run-time type information. Returns `None` if there is no such table or
	/// more than one.
	///
	/// The search does not check that the class derives from `CBaseEntity`,
	/// so that the slot holds `Spawn`: any class with that many virtual
	/// methods passes, which most polymorphic classes do, as the slot is an
	/// early one. The table is only the class's own: classes deriving from it
	/// have tables of their own. The address is metadata from the snapshot: it
	/// does not keep the module loaded, and the table is the class's only
	/// while the module that [`Self::load`] snapshot stays loaded.
	///
	/// Each search reads the whole snapshot a few times, so find the classes
	/// needed together with [`Self::find_all`].
	pub fn find(&self, class: &str) -> Option<NonNull<*mut c_void>> {
		NonNull::new(self.0.primary_vtable(class, SPAWN_SLOT)? as *mut *mut c_void)
	}

	/// The tables [`Self::find`] finds for each of `classes`, in their order.
	/// The snapshot is read as often for every class as [`Self::find`] reads
	/// it for one.
	pub fn find_all(&self, classes: &[&str]) -> Vec<Option<NonNull<*mut c_void>>> {
		self.0
			.primary_vtables(classes, SPAWN_SLOT)
			.into_iter()
			.map(|table| NonNull::new(table? as *mut *mut c_void))
			.collect()
	}
}
