//! Hand-written ABI of TF2's entity teams: the vtable slot of
//! `CBaseEntity::ChangeTeam`, through which every change of an entity's team
//! passes, its signature, and the search for an entity class's vtable.

use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, Image};
use crate::vtable_slot;
use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

/// The signature of `CBaseEntity::ChangeTeam`, `void (int)`, which puts the
/// entity on the team numbered `team`, such as
/// [`TF_TEAM_RED`](crate::tf2::scoreboard::TF_TEAM_RED).
///
/// `CBaseEntity`'s own method only stores the team. Classes that override it
/// do more: a TF2 player changes its team, and a respawn room moves its
/// visualizers to the team too.
#[doc(alias("ChangeTeam"))]
pub type ChangeTeamFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity, team: c_int);

// `ChangeTeam` follows the three methods `TF_DLL` declares after
// `IsCombatItem`, and `NEXT_BOT`'s `IsNextBot`, so its slot is TF2's alone.
const _: () = assert!(
	vtable_slot!(
		sys::CBaseEntity__bindgen_vtable,
		CBaseEntity_IsBaseCombatWeapon
	) < CHANGE_TEAM_SLOT
);

// The generated method has the hand-written signature.
const _: fn(&sys::CBaseEntity__bindgen_vtable) -> ChangeTeamFn =
	|vtable| vtable.CBaseEntity_ChangeTeam;

/// The slot of `CBaseEntity::ChangeTeam` in a TF2 entity's primary vtable,
/// from the generated binding. The classes that override it keep the slot.
///
/// Methods `TF_DLL` and `NEXT_BOT` declare come before it, so other games'
/// entities have it elsewhere.
#[doc(alias("ChangeTeam"))]
pub const CHANGE_TEAM_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_ChangeTeam);

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its entity classes, through which the game calls
/// `ChangeTeam` on their entities.
#[derive(Debug, Clone)]
pub struct TeamVtables(Image);

impl TeamVtables {
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
	/// as `CFuncRespawnRoom`, whose [`CHANGE_TEAM_SLOT`] entry is executable,
	/// from its run-time type information. Returns `None` if there is no such
	/// table or more than one.
	///
	/// The search does not check that the class derives from `CBaseEntity`,
	/// so that the slot holds `ChangeTeam`: any class with that many virtual
	/// methods passes. The table is only the class's own: classes deriving
	/// from it have tables of their own. The address is metadata from the
	/// snapshot: it does not keep the module loaded, and the table is the
	/// class's only while the module that [`Self::load`] snapshot stays loaded.
	///
	/// Each search reads the whole snapshot a few times, so find the classes
	/// needed together with [`Self::find_all`].
	pub fn find(&self, class: &str) -> Option<NonNull<*mut c_void>> {
		NonNull::new(self.0.primary_vtable(class, CHANGE_TEAM_SLOT)? as *mut *mut c_void)
	}

	/// The tables [`Self::find`] finds for each of `classes`, in their order.
	/// The snapshot is read as often for every class as [`Self::find`] reads
	/// it for one.
	pub fn find_all(&self, classes: &[&str]) -> Vec<Option<NonNull<*mut c_void>>> {
		self.0
			.primary_vtables(classes, CHANGE_TEAM_SLOT)
			.into_iter()
			.map(|table| NonNull::new(table? as *mut *mut c_void))
			.collect()
	}
}
