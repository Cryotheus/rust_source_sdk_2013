//! Hand-written ABI of TF2's entity teams: the vtable slot of
//! `CBaseEntity::ChangeTeam`, through which every change of an entity's team
//! passes, and its signature.

use crate::vtable_slot;
use std::ffi::c_int;

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
