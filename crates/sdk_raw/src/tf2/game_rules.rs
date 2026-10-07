//! Hand-written ABI of TF2's game rules (`CTFGameRules`), which the generated
//! bindings do not describe: the vtable slot and signature of `CleanUpMap`,
//! the round states of `gamerules_roundstate_t`, and the search for the game
//! rules' vtable.

#[cfg(test)]
#[path = "../tests/tf2/game_rules.rs"]
mod tests;

use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, Image};

#[cfg(target_os = "linux")]
use crate::util::elf::LoadedElf;

use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

/// The signature of `CTFGameRules::CleanUpMap`, `void ()`, with the game
/// rules as its receiver, which removes every entity a round's restart does
/// not keep, then creates the map's entities anew.
#[doc(alias("CleanUpMap"))]
pub type CleanUpMapFn = unsafe extern "C" fn(this: *mut c_void);

// `CleanUpMap` is slot 231 of TF2's 64-bit Windows `server.dll`, whose
// `CTFGameRules` vtable, found through its run-time type information, holds
// there the function that removes every player's conditions and then calls
// the base class's, the only function referencing
// `"CleanUpMap\n===============\n"`, which it prints under
// `mp_showcleanedupents`. `CTeamplayRoundBasedRules::RoundRespawn` calls the
// slot through the vtable. `CTeamplayRoundBasedRules`' own vtable has its
// function at the same slot.
//
// On Linux, Clang's `-fdump-vtable-layouts` of Valve's `tf_gamerules.h`, under
// the defines of `server_tf.vpc`, gives 233. Up to slot 190, Linux is one past
// Windows, for the Itanium ABI's second destructor; from there on it is two
// past, because the Itanium ABI also gives `FireGameEvent`, which overrides a
// method of the secondary base `IGameEventListener2`, a slot of the primary
// vtable. The layouts match the 64-bit gamedata of public SourceMod plugins
// for the methods around it: `RoundRespawn` at 230 and 232, and
// `CheckRespawnWaves` at 232 and 234. No Linux binary confirmed it, and the
// slots around it hold functions of the same signature, so on Linux the
// search also requires the slot to hold the function the unstripped
// `server_srv.so` names `CTFGameRules::CleanUpMap`.

/// The slot of `CleanUpMap` in `CTFGameRules`' primary vtable.
#[doc(alias("CleanUpMap"))]
pub const CLEAN_UP_MAP_SLOT: usize = cfg_select! {
	target_os = "windows" => 231,
	target_os = "linux" => 233,
};

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
