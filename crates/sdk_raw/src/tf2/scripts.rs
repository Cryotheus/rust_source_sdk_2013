//! Hand-written ABI of the VScripts TF2's entities run: the vtable slot of
//! `CBaseEntity::RunVScripts`, through which the game runs the scripts an
//! entity's `vscripts` key value names, its signature, and the search for an
//! entity class's vtable.
//!
//! The game calls `RunVScripts` as it spawns an entity (`DispatchSpawn`),
//! before the entity's own `Spawn`: for the map's entities as a level loads,
//! again for those a round's restart creates, and for those spawned later.
//! Unless the entity names no script, it gives the entity its script scope,
//! runs each script in it, and starts the entity's `thinkfunction`. The
//! scripts' `Precache` and `OnPostSpawn` run later in the spawn, also only for
//! an entity that names a script. `logic_script`'s `CLogicScript` overrides
//! it, to give its scripts the entities its group key values name first.

use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, Image};
use crate::vtable_slot;
use std::ffi::c_void;
use std::ptr::NonNull;

/// The signature of `CBaseEntity::RunVScripts`, `void ()`, which runs the
/// scripts the entity's `vscripts` key value names.
#[doc(alias("RunVScripts"))]
pub type RunVScriptsFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity);

// `RunVScripts` follows `ChangeTeam`, which follows the methods `TF_DLL` and
// `NEXT_BOT` declare, so its slot is TF2's alone.
const _: () = assert!(
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_ChangeTeam) < RUN_VSCRIPTS_SLOT
);

// The generated method has the hand-written signature.
const _: fn(&sys::CBaseEntity__bindgen_vtable) -> RunVScriptsFn =
	|vtable| vtable.CBaseEntity_RunVScripts;

/// The slot of `CBaseEntity::RunVScripts` in a TF2 entity's primary vtable,
/// from the generated binding. The classes that override it keep the slot.
///
/// Methods `TF_DLL` and `NEXT_BOT` declare come before it, so other games'
/// entities have it elsewhere.
#[doc(alias("RunVScripts"))]
pub const RUN_VSCRIPTS_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_RunVScripts);

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its entity classes, through which the game calls
/// `RunVScripts` on their entities.
#[derive(Debug, Clone)]
pub struct ScriptVtables(Image);

impl ScriptVtables {
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
	/// as `CLogicScript`, whose [`RUN_VSCRIPTS_SLOT`] entry is executable,
	/// from its run-time type information. Returns `None` if there is no such
	/// table or more than one.
	///
	/// The search does not check that the class derives from `CBaseEntity`,
	/// so that the slot holds `RunVScripts`: any class with that many virtual
	/// methods passes. The table is only the class's own: classes deriving
	/// from it have tables of their own. The address is metadata from the
	/// snapshot: it does not keep the module loaded, and the table is the
	/// class's only while the module that [`Self::load`] snapshot stays loaded.
	///
	/// Each search reads the whole snapshot a few times, so find the classes
	/// needed together with [`Self::find_all`].
	pub fn find(&self, class: &str) -> Option<NonNull<*mut c_void>> {
		NonNull::new(self.0.primary_vtable(class, RUN_VSCRIPTS_SLOT)? as *mut *mut c_void)
	}

	/// The tables [`Self::find`] finds for each of `classes`, in their order.
	/// The snapshot is read as often for every class as [`Self::find`] reads
	/// it for one.
	pub fn find_all(&self, classes: &[&str]) -> Vec<Option<NonNull<*mut c_void>>> {
		self.0
			.primary_vtables(classes, RUN_VSCRIPTS_SLOT)
			.into_iter()
			.map(|table| NonNull::new(table? as *mut *mut c_void))
			.collect()
	}
}
