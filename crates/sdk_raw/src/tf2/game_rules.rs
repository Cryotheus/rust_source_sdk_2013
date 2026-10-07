//! ABI of TF2's game rules (`CTFGameRules`): the vtable slots and signatures of
//! the methods that clean up the map, end and set up rounds, and decide
//! captures and holidays, taken from the generated bindings, with calls of
//! some; the round states of `gamerules_roundstate_t`, the team roles and the
//! stalemate reasons; and the search for the game rules' vtable.

#[cfg(test)]
#[path = "../tests/tf2/game_rules.rs"]
mod tests;

use crate::abi::CppDestructors;
use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, Image};
use crate::{vcall, vtable_slot};

#[cfg(target_os = "linux")]
use crate::util::elf::LoadedElf;

use std::ffi::{c_char, c_int, c_void};
use std::ptr::NonNull;

/// The signature of `CTFGameRules::CleanUpMap`, `void ()`, with the game
/// rules as its receiver, which removes every entity a round's restart does
/// not keep, then creates the map's entities anew.
#[doc(alias("CleanUpMap"))]
pub type CleanUpMapFn = unsafe extern "C" fn(this: *mut c_void);

/// The signature of `CTFGameRules::IsHolidayActive`, `bool (int) const`,
/// with the game rules as its receiver: whether the `EHoliday` is active.
#[doc(alias("IsHolidayActive"))]
pub type IsHolidayActiveFn = unsafe extern "C" fn(this: *mut c_void, holiday: c_int) -> bool;

/// The signature of `CTFGameRules::PlayerMayCapturePoint`,
/// `bool (CBasePlayer *, int, char *, int)`, with the game rules as its
/// receiver: the player, the control point's index, and a buffer of the given
/// size for the reason the player may not, or null.
#[doc(alias("PlayerMayCapturePoint"))]
pub type PlayerMayCapturePointFn = unsafe extern "C" fn(
	this: *mut c_void,
	player: *mut sys::CBasePlayer,
	point: c_int,
	reason: *mut c_char,
	reason_size: c_int,
) -> bool;

/// The signature of `CTFGameRules::PointsMayBeCaptured`, `bool ()`, with the
/// game rules as its receiver: whether any control point may be captured.
#[doc(alias("PointsMayBeCaptured"))]
pub type PointsMayBeCapturedFn = unsafe extern "C" fn(this: *mut c_void) -> bool;

/// The signature of `CTFGameRules::SetupOnRoundStart` and
/// `CTFGameRules::SetupOnRoundRunning`, `void ()`, with the game rules as
/// their receiver.
#[doc(alias("SetupOnRoundStart", "SetupOnRoundRunning"))]
pub type RoundSetupFn = unsafe extern "C" fn(this: *mut c_void);

/// The signature of `CTFGameRules::SetStalemate`, `void (int, bool, bool)`,
/// with the game rules as its receiver: the `STALEMATE_` reason, whether to
/// reset the map after it, and whether the teams switch sides.
#[doc(alias("SetStalemate"))]
pub type SetStalemateFn = unsafe extern "C" fn(
	this: *mut c_void,
	reason: c_int,
	force_map_reset: bool,
	switch_teams: bool,
);

/// The signature of `CTFGameRules::SetWinningTeam`,
/// `void (int, int, bool, bool, bool, bool)`, with the game rules as its
/// receiver: the winning team, or `TEAM_UNASSIGNED` for none, the `WINREASON_`
/// reason, whether to reset the map after it, whether the teams switch sides,
/// whether to keep the winners' score, and whether it is the game's final
/// round.
#[doc(alias("SetWinningTeam"))]
pub type SetWinningTeamFn = unsafe extern "C" fn(
	this: *mut c_void,
	team: c_int,
	reason: c_int,
	force_map_reset: bool,
	switch_teams: bool,
	dont_add_score: bool,
	final_round: bool,
);

/// The signature of `CTFGameRules::TeamMayCapturePoint`, `bool (int, int)`,
/// with the game rules as its receiver: the team and the control point's
/// index.
#[doc(alias("TeamMayCapturePoint"))]
pub type TeamMayCapturePointFn =
	unsafe extern "C" fn(this: *mut c_void, team: c_int, point: c_int) -> bool;

// The generated methods have the signatures above, with a `CTFGameRules`
// receiver, the object at the game rules' address.
const _: fn(&sys::CTFGameRules__bindgen_vtable) -> unsafe extern "C" fn(*mut sys::CTFGameRules) =
	|vtable| vtable.CTFGameRules_CleanUpMap;

const _: fn(
	&sys::CTFGameRules__bindgen_vtable,
) -> unsafe extern "C" fn(*const sys::CTFGameRules, c_int) -> bool =
	|vtable| vtable.CTFGameRules_IsHolidayActive;

const _: fn(
	&sys::CTFGameRules__bindgen_vtable,
) -> unsafe extern "C" fn(
	*mut sys::CTFGameRules,
	*mut sys::CBasePlayer,
	c_int,
	*mut c_char,
	c_int,
) -> bool = |vtable| vtable.CTFGameRules_PlayerMayCapturePoint;

const _: fn(
	&sys::CTFGameRules__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CTFGameRules) -> bool =
	|vtable| vtable.CTFGameRules_PointsMayBeCaptured;

const _: fn(&sys::CTFGameRules__bindgen_vtable) -> unsafe extern "C" fn(*mut sys::CTFGameRules) =
	|vtable| vtable.CTFGameRules_SetupOnRoundRunning;

const _: fn(&sys::CTFGameRules__bindgen_vtable) -> unsafe extern "C" fn(*mut sys::CTFGameRules) =
	|vtable| vtable.CTFGameRules_SetupOnRoundStart;

const _: fn(
	&sys::CTFGameRules__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CTFGameRules, c_int, bool, bool) =
	|vtable| vtable.CTFGameRules_SetStalemate;

const _: fn(
	&sys::CTFGameRules__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CTFGameRules, c_int, c_int, bool, bool, bool, bool) =
	|vtable| vtable.CTFGameRules_SetWinningTeam;

const _: fn(
	&sys::CTFGameRules__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CTFGameRules, c_int, c_int) -> bool =
	|vtable| vtable.CTFGameRules_TeamMayCapturePoint;

// The generated slots match those found before `CTFGameRules` was generated.
// `CleanUpMap` is slot 231 of TF2's 64-bit Windows `server.dll`, whose
// `CTFGameRules` vtable, found through its run-time type information, holds
// there the function that removes every player's conditions and then calls
// the base class's, the only function referencing
// `"CleanUpMap\n===============\n"`, which it prints under
// `mp_showcleanedupents`. On Linux it is 233, the slot of the function the
// unstripped `server_srv.so` names `CTFGameRules::CleanUpMap`, which the
// search still requires. Up to slot 190, Linux is one past Windows, for the
// Itanium ABI's second destructor; from there on it is two past, because the
// Itanium ABI also gives `FireGameEvent`, which overrides a method of the
// secondary base `IGameEventListener2`, a slot of the primary vtable.
//
// The 64-bit gamedata of public SourceMod plugins agrees for other methods:
// `IsHolidayActive` at 139 and 140, and `RoundRespawn` at 230 and 232.
const _: () = {
	assert!(
		CLEAN_UP_MAP_SLOT
			== cfg_select! {
				target_os = "windows" => 231,
				target_os = "linux" => 233,
			}
	);
	assert!(IS_HOLIDAY_ACTIVE_SLOT == 138 + CppDestructors::VTABLE_SLOTS);
	assert!(
		vtable_slot!(sys::CTFGameRules__bindgen_vtable, CTFGameRules_RoundRespawn)
			== cfg_select! {
				target_os = "windows" => 230,
				target_os = "linux" => 232,
			}
	);
};

/// The slot of `CleanUpMap` in `CTFGameRules`' primary vtable, from the
/// generated binding.
#[doc(alias("CleanUpMap"))]
pub const CLEAN_UP_MAP_SLOT: usize =
	vtable_slot!(sys::CTFGameRules__bindgen_vtable, CTFGameRules_CleanUpMap);

/// The mangled symbol of `CTFGameRules::CleanUpMap()`, whose function
/// [`CLEAN_UP_MAP_SLOT`] must hold on Linux.
#[cfg(target_os = "linux")]
const CLEAN_UP_MAP_SYMBOL: &[u8] = b"_ZN12CTFGameRules10CleanUpMapEv";

/// The name of TF2's game rules class in its run-time type information.
pub const GAME_RULES_CLASS: &str = "CTFGameRules";

/// `GR_STATE_INIT`: the game rules were just created.
pub const GR_STATE_INIT: c_int = 0;

/// `GR_STATE_PREGAME`: before players are ready, before the game starts.
pub const GR_STATE_PREGAME: c_int = 1;

/// `GR_STATE_STARTGAME`: players are ready, and the first round is about to
/// be set up, a tick later.
pub const GR_STATE_STARTGAME: c_int = 2;

/// `GR_STATE_PREROUND`: a round was set up, and players wait to move.
pub const GR_STATE_PREROUND: c_int = 3;

/// `GR_STATE_RND_RUNNING`: a round is being played.
pub const GR_STATE_RND_RUNNING: c_int = 4;

/// `GR_STATE_TEAM_WIN`: a team won the round.
pub const GR_STATE_TEAM_WIN: c_int = 5;

/// `GR_STATE_RESTART`: the round is restarting.
pub const GR_STATE_RESTART: c_int = 6;

/// `GR_STATE_STALEMATE`: sudden death, or an arena round.
pub const GR_STATE_STALEMATE: c_int = 7;

/// `GR_STATE_GAME_OVER`: the game ended, as the map is about to change.
pub const GR_STATE_GAME_OVER: c_int = 8;

/// `GR_STATE_BONUS`: a bonus round.
pub const GR_STATE_BONUS: c_int = 9;

/// `GR_STATE_BETWEEN_RNDS`: players ready up, between Mann vs. Machine's waves
/// or before a matchmade game (`UsePlayerReadyStatusMode`).
pub const GR_STATE_BETWEEN_RNDS: c_int = 10;

/// The slot of `IsHolidayActive` in `CTFGameRules`' primary vtable, from the
/// generated binding.
#[doc(alias("IsHolidayActive"))]
pub const IS_HOLIDAY_ACTIVE_SLOT: usize = vtable_slot!(
	sys::CTFGameRules__bindgen_vtable,
	CTFGameRules_IsHolidayActive
);

/// The slot of `PlayerMayCapturePoint` in `CTFGameRules`' primary vtable,
/// from the generated binding.
#[doc(alias("PlayerMayCapturePoint"))]
pub const PLAYER_MAY_CAPTURE_POINT_SLOT: usize = vtable_slot!(
	sys::CTFGameRules__bindgen_vtable,
	CTFGameRules_PlayerMayCapturePoint
);

/// The slot of `PointsMayBeCaptured` in `CTFGameRules`' primary vtable, from
/// the generated binding.
#[doc(alias("PointsMayBeCaptured"))]
pub const POINTS_MAY_BE_CAPTURED_SLOT: usize = vtable_slot!(
	sys::CTFGameRules__bindgen_vtable,
	CTFGameRules_PointsMayBeCaptured
);

/// The slot of `SetStalemate` in `CTFGameRules`' primary vtable, from the
/// generated binding.
#[doc(alias("SetStalemate"))]
pub const SET_STALEMATE_SLOT: usize =
	vtable_slot!(sys::CTFGameRules__bindgen_vtable, CTFGameRules_SetStalemate);

/// The slot of `SetWinningTeam` in `CTFGameRules`' primary vtable, from the
/// generated binding.
#[doc(alias("SetWinningTeam"))]
pub const SET_WINNING_TEAM_SLOT: usize = vtable_slot!(
	sys::CTFGameRules__bindgen_vtable,
	CTFGameRules_SetWinningTeam
);

/// The slot of `SetupOnRoundRunning` in `CTFGameRules`' primary vtable, from
/// the generated binding.
#[doc(alias("SetupOnRoundRunning"))]
pub const SETUP_ON_ROUND_RUNNING_SLOT: usize = vtable_slot!(
	sys::CTFGameRules__bindgen_vtable,
	CTFGameRules_SetupOnRoundRunning
);

/// The slot of `SetupOnRoundStart` in `CTFGameRules`' primary vtable, from
/// the generated binding.
#[doc(alias("SetupOnRoundStart"))]
pub const SETUP_ON_ROUND_START_SLOT: usize = vtable_slot!(
	sys::CTFGameRules__bindgen_vtable,
	CTFGameRules_SetupOnRoundStart
);

/// `STALEMATE_JOIN_MID` from `game/shared/teamplayroundbased_gamerules.h`: a
/// stalemate because players joined mid-round.
pub const STALEMATE_JOIN_MID: c_int = 0;

/// `STALEMATE_TIMER`: a stalemate because the round timer ran out.
pub const STALEMATE_TIMER: c_int = 1;

/// `STALEMATE_SERVER_TIMELIMIT`: a stalemate because the map's time limit ran
/// out.
pub const STALEMATE_SERVER_TIMELIMIT: c_int = 2;

/// The slot of `TeamMayCapturePoint` in `CTFGameRules`' primary vtable, from
/// the generated binding.
#[doc(alias("TeamMayCapturePoint"))]
pub const TEAM_MAY_CAPTURE_POINT_SLOT: usize = vtable_slot!(
	sys::CTFGameRules__bindgen_vtable,
	CTFGameRules_TeamMayCapturePoint
);

/// `TEAM_ROLE_NONE` from `game/shared/tf/tf_shareddefs.h`: a team neither
/// attacking nor defending.
pub const TEAM_ROLE_NONE: c_int = 0;

/// `TEAM_ROLE_DEFENDERS`: a team defending the objectives.
pub const TEAM_ROLE_DEFENDERS: c_int = 1;

/// `TEAM_ROLE_ATTACKERS`: a team attacking the objectives.
pub const TEAM_ROLE_ATTACKERS: c_int = 2;

/// Finds the unique primary vtable of `CTFGameRules` whose
/// [`CLEAN_UP_MAP_SLOT`] entry is executable, from the run-time type
/// information of the module whose `CreateInterface` export is `factory`,
/// such as the game server module. On Linux, the entry must also be the
/// function the module's symbols name `CTFGameRules::CleanUpMap`. Returns
/// `Ok(None)` if there is no such table, or more than one.
///
/// The address is metadata from a snapshot of the module: it does not keep
/// the module loaded, and the table is the class's only while the module
/// stays loaded.
///
/// # Safety
///
/// `factory` must be the `CreateInterface` export of a module that stays
/// loaded throughout this call.
pub unsafe fn find_game_rules_vtable(
	factory: CreateInterfaceFn,
) -> Result<Option<NonNull<*mut c_void>>, util::Error> {
	// SAFETY: The factory is an executable address in its module, which the
	// caller keeps loaded while it is inspected.
	let image = unsafe { Image::load(factory as usize) }?;

	let Some(vtable) = image.primary_vtable(GAME_RULES_CLASS, CLEAN_UP_MAP_SLOT) else {
		return Ok(None);
	};

	#[cfg(target_os = "linux")]
	{
		let entry = CLEAN_UP_MAP_SLOT
			.checked_mul(size_of::<usize>())
			.and_then(|offset| vtable.checked_add(offset))
			.and_then(|address| image.read(address, size_of::<usize>()))
			.and_then(|bytes| util::word_at(bytes, 0));

		// SAFETY: As for the image above.
		let elf = unsafe { LoadedElf::at(factory as usize) }?;

		if entry.is_none() || entry != elf.function_address(CLEAN_UP_MAP_SYMBOL) {
			return Ok(None);
		}
	}

	Ok(NonNull::new(vtable as *mut *mut c_void))
}

/// Calls `CTFGameRules::IsHolidayActive`, which tells whether the `EHoliday`
/// is active, as `tf_forced_holiday`, the date and the level's holiday make
/// it, and as hooks of the method decide.
///
/// The method is called through the generated vtable, at
/// [`IS_HOLIDAY_ACTIVE_SLOT`].
///
/// # Safety
///
/// `rules` must point to TF2's live game rules, a `CTFGameRules`, and the call
/// must be made on the server's main thread.
#[doc(alias("IsHolidayActive"))]
pub unsafe fn is_holiday_active(rules: NonNull<c_void>, holiday: c_int) -> bool {
	let rules = rules.as_ptr().cast::<sys::CTFGameRules>().cast_const();

	// SAFETY: As the caller promises; the game rules' primary vtable is TF2's
	// `CTFGameRules` vtable, as generated.
	unsafe {
		vcall!(rules as sys::CTFGameRules__bindgen_vtable => CTFGameRules_IsHolidayActive(holiday))
	}
}

/// Calls `CTFGameRules::SetStalemate`, which starts sudden death for the
/// `STALEMATE_` reason, or ends the round without a winner while
/// `mp_stalemate_enable` is off, unless the round is already a stalemate, or a
/// tournament has not started.
///
/// The method is called through the generated vtable, at
/// [`SET_STALEMATE_SLOT`].
///
/// # Safety
///
/// - `rules` must point to TF2's live game rules, a `CTFGameRules`, and the
///   call must be made on the server's main thread.
/// - Everything the method runs, such as the `teamplay_round_stalemate`
///   event's listeners, must free entities only through deferred deletion.
#[doc(alias("SetStalemate"))]
pub unsafe fn set_stalemate(
	rules: NonNull<c_void>,
	reason: c_int,
	force_map_reset: bool,
	switch_teams: bool,
) {
	let rules = rules.as_ptr().cast::<sys::CTFGameRules>();

	// SAFETY: As for `is_holiday_active`, and the caller promises the rest.
	unsafe {
		vcall!(rules as sys::CTFGameRules__bindgen_vtable => CTFGameRules_SetStalemate(reason, force_map_reset, switch_teams))
	}
}

/// Calls `CTFGameRules::SetWinningTeam`, which ends the round with a win for
/// `team`, or without a winner for `TEAM_UNASSIGNED`, and starts the bonus
/// round, in which the winners are crit boosted, and the losers humiliated.
///
/// TF2's part of the method runs first, and always: it crit boosts the
/// team's living players for the bonus round, and adds King of the Hill's and
/// Payload Race's progress to the teams' statistics. Only then does the base
/// class refuse a team that is neither `TEAM_UNASSIGNED` nor a playing team,
/// a round a team already won, and any win during commentary.
///
/// The method is called through the generated vtable, at
/// [`SET_WINNING_TEAM_SLOT`].
///
/// # Safety
///
/// - `rules` must point to TF2's live game rules, a `CTFGameRules`, and the
///   call must be made on the server's main thread.
/// - On a King of the Hill level, both teams' King of the Hill timers must
///   exist, as the level's `tf_logic_koth` makes them: the method reads them
///   without checking.
/// - Everything the method runs, such as the `teamplay_round_win` event's
///   listeners, must free entities only through deferred deletion.
#[doc(alias("SetWinningTeam"))]
pub unsafe fn set_winning_team(
	rules: NonNull<c_void>,
	team: c_int,
	reason: c_int,
	force_map_reset: bool,
	switch_teams: bool,
	dont_add_score: bool,
	final_round: bool,
) {
	let rules = rules.as_ptr().cast::<sys::CTFGameRules>();

	// SAFETY: As for `is_holiday_active`, and the caller promises the rest.
	unsafe {
		vcall!(rules as sys::CTFGameRules__bindgen_vtable => CTFGameRules_SetWinningTeam(
			team,
			reason,
			force_map_reset,
			switch_teams,
			dont_add_score,
			final_round,
		))
	}
}
