//! Hand-written ABI of TF2's entity teams: the vtable slot of
//! `CBaseEntity::ChangeTeam`, through which every change of an entity's team
//! passes, its signature, and the search for an entity class's vtable.

use crate::tf2::class_targets::SlotVtables;
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

/// The signature of `CTFPlayer::ChangeTeam`, `void (int iTeamNum, bool
/// bAutoTeam, bool bSilent, bool bAutoBalance)`, which puts a player on the
/// team numbered `team`, as the `jointeam`, `autoteam` and `spectate`
/// commands ask, and as TF2's autobalance moves them.
///
/// Before the team changes, it removes the player's projectiles and
/// detonates their buildings (`RemoveAllOwnedEntitiesFromWorld`), and after,
/// kills a player who was alive on RED or BLU. It refuses a change the game
/// rules or the player's state forbid, and one to the team the player is on,
/// before any of that.
///
/// The generated method takes a `CTFPlayer` receiver, whose primary base,
/// `CBaseEntity`, is at the same address, so it can be hooked with an entity
/// receiver.
#[doc(alias("ChangeTeam"))]
pub type PlayerChangeTeamFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	team: c_int,
	auto_team: bool,
	silent: bool,
	auto_balance: bool,
);

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its entity classes, through which the game calls
/// `ChangeTeam` on their entities: those whose [`CHANGE_TEAM_SLOT`] entries
/// are executable.
pub type TeamVtables = SlotVtables<CHANGE_TEAM_SLOT>;

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

// The generated overload with four parameters has the hand-written
// signature, but for its receiver.
const _: fn(
	&sys::CTFPlayer__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CTFPlayer, c_int, bool, bool, bool) =
	|vtable| vtable.CTFPlayer_ChangeTeam1;

/// The slot of `CBaseEntity::ChangeTeam` in a TF2 entity's primary vtable,
/// from the generated binding. The classes that override it keep the slot.
///
/// Methods `TF_DLL` and `NEXT_BOT` declare come before it, so other games'
/// entities have it elsewhere.
#[doc(alias("ChangeTeam"))]
pub const CHANGE_TEAM_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_ChangeTeam);

/// The slot of `CTFPlayer::ChangeTeam(int, bool, bool, bool)` in a TF2
/// player's primary vtable, from the generated binding. `CTFBot` keeps
/// `CTFPlayer`'s.
///
/// `CTFPlayer::ForceChangeTeam`, through which the game moves players itself,
/// such as as it scrambles the teams, is not virtual, and does not call it.
#[doc(alias("ChangeTeam"))]
pub const PLAYER_CHANGE_TEAM_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_ChangeTeam1);
