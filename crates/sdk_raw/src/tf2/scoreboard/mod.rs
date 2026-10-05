//! Hand-written ABI of TF2's player resource and scoreboard statistics.
//!
//! TF2's `CTFPlayerResource` (`game/server/tf/tf_player_resource.h`) and its
//! base `CPlayerResource` (`game/server/player_resource.h`) network one array
//! per scoreboard column, which the generated bindings do not lay out.
//!
//! # Scores
//!
//! The game stores no player's score. Its game statistics singleton,
//! `CTF_GameStats` (`game/server/tf/tf_gamestats.h`), keeps a
//! [`PlayerStats`] block per entity index, whose [`RoundStats`] count each
//! [`stat`] for the current life, the current round, and the session.
//! `CTFPlayerResource::UpdateConnectedPlayer` recomputes the scoreboard's
//! Score from the session block with `CTFGameRules::CalcPlayerScore` every
//! time the resource thinks, and the round's score is computed from the round
//! block the same way. Changing the statistics therefore changes the scores
//! the game computes, sends, and acts on.
//!
//! [`GameStats`] finds the singleton and the two functions in the game server
//! module, which exports none of them. On Windows they are found through
//! signatures whose operands, calls, string reference, and run-time type
//! information must all agree, and on Linux through the symbols of an
//! unstripped `server_srv.so`, which are inferred from the SDK's source and
//! have not been checked against a retail Linux build. Every inspection uses
//! owned snapshots or copies, and any disagreement fails closed. Before a
//! resolution is returned, `CalcPlayerScore` must score a known set of
//! statistics as the SDK's source does.
//!
//! The functions are a static member function and a member function, called
//! as `extern "C"`: on both supported x86-64 targets that is their calling
//! convention, with `this` passed as the first argument of a member function.
//!
//! # Caching
//!
//! [`GameStats::cached`] keeps the last successful resolution for the rest of
//! the process in a [`ModuleCache`], keyed by the address of the factory and
//! the base address of the module containing it, as `ItemGeneration::cached` in
//! [`tf2::item_generation`](crate::tf2::item_generation#caching) does, under
//! the same assumption: Source never unloads the game server module while
//! plugins are loaded. Failures are not kept.

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;

#[cfg(test)]
#[path = "../../tests/tf2/scoreboard.rs"]
mod tests;

use crate::interfaces::CreateInterfaceFn;
use crate::players::FIRST_GAME_TEAM;
use crate::util::{ModuleCache, ModuleKey};
use crate::vtable_slot;
use std::ffi::{c_int, c_void};
use std::mem::transmute;
use std::num::NonZeroUsize;
use std::ptr::{self, NonNull};

/// `CTFGameRules::CalcPlayerScore(RoundStats_t *, CTFPlayer *)`, a static
/// member function, which scores `stats`, adding the `scoreboard_minigame`
/// attribute's terms if `player` is not null, and clamps the score at 0.
#[doc(alias("CalcPlayerScore"))]
pub type CalcPlayerScoreFn =
	unsafe extern "C" fn(stats: *mut RoundStats, player: *mut sys::CTFPlayer) -> c_int;

/// `CTFGameStats::FindPlayerStats(CBasePlayer *)`, which returns the
/// [`PlayerStats`] block of the player's entity index without checking it, or
/// null for a null player.
#[doc(alias("FindPlayerStats"))]
pub type FindPlayerStatsFn =
	unsafe extern "C" fn(this: *mut c_void, player: *mut sys::CBasePlayer) -> *mut PlayerStats;

/// `CTFPlayer::ResetScores`, which resets the player's scoring data and
/// statistics, and with them their Score, frags and deaths.
///
/// The generated method takes a `CTFPlayer` receiver. It is the player's
/// primary base, `CBaseEntity`, at the same address, so the method can be
/// called and hooked with an entity receiver.
#[doc(alias("ResetScores"))]
pub type ResetScoresFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity);

// The generated method takes no argument and returns nothing.
const _: fn(&sys::CTFPlayer__bindgen_vtable) -> unsafe extern "C" fn(*mut sys::CTFPlayer) =
	|vtable| vtable.CTFPlayer_ResetScores;

// SourceMod's `sdktools.games/game.tf.txt` gamedata lists
// `CTFPlayer::CommitSuicide(bool, bool)` at 454 under both ABIs, where the
// generated vtable has it too, a few slots after `ResetScores`.
const _: () = {
	let commit_suicide = vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_CommitSuicide);

	assert!(commit_suicide == 454 && RESET_SCORES_SLOT < commit_suicide);
};

// `PlayerStats_t` starts with its life, round, and session `RoundStats_t`,
// each `TFSTAT_TOTAL` `int`s, and is made of `int`s only.
const _: () = {
	assert!(size_of::<RoundStats>() == TFSTAT_TOTAL * size_of::<c_int>());
	assert!(size_of::<RoundStats>() == 180);
	assert!(align_of::<RoundStats>() == align_of::<c_int>());
	assert!(STATS_CURRENT_LIFE == 0);
	assert!(STATS_CURRENT_ROUND == size_of::<RoundStats>());
	assert!(STATS_ACCUMULATED == 2 * size_of::<RoundStats>());
	assert!(STATS_ACCUMULATED + size_of::<RoundStats>() <= PLAYER_STATS_SIZE);
	assert!(PLAYER_STATS_SIZE.is_multiple_of(align_of::<c_int>()));
	assert!(stat::FLAGRETURNS + 1 == TFSTAT_TOTAL);
};

// `GameStats` is a plain bundle of addresses, which only its unsafe methods
// use, so it can be copied and shared between threads.
const _: () = {
	const fn assert_plain<T: Copy + Send + Sync>() {}

	assert_plain::<GameStats>();
};

// Each player slot's group of streaks includes its kill streak.
const _: () = assert!(KILL_STREAK < STREAKS_PER_SLOT);

// The test's expected score follows from the header's weights.
const _: () = assert!(SELF_TEST_SCORE == 7 * TF_SCORE_KILL_RUNECARRIER + 2 * TF_SCORE_KILL);

/// Bytes between the elements of the player resource's arrays of `int`s
/// (`CNetworkArray( int, ... )`), which include every scoreboard column.
pub const ELEMENT_SIZE: usize = size_of::<c_int>();

/// Where `CTFGameStats::m_aPlayerStats`, the array of [`PlayerStats`] blocks,
/// lies in `CTF_GameStats` in the 64-bit Windows `server.dll`.
///
/// Resolution on Windows checks that the game's code agrees. On Linux, the
/// array's place is left to `FindPlayerStats`.
#[cfg(target_os = "windows")]
#[cfg_attr(docsrs, doc(cfg(target_os = "windows")))]
#[doc(alias("m_aPlayerStats"))]
pub const GAME_STATS_PLAYER_STATS: usize = 0xd8;

/// The element of a player slot's `m_iStreaks` group the scoreboard shows:
/// `CTFPlayerShared::kTFStreak_Kills`.
#[doc(alias("kTFStreak_Kills"))]
pub const KILL_STREAK: usize = sys::CTFPlayerShared_ETFStreak_kTFStreak_Kills as usize;

/// The [`PlayerStats`] blocks of `CTFGameStats::m_aPlayerStats`, one per
/// entity index from 0, which no player has, to TF2's `MAX_PLAYERS`, 101.
///
/// This is `MAX_PLAYERS_ARRAY_SAFE` from `game/shared/shareddefs.h`, as TF2
/// builds it. The 64-bit Windows `server.dll` constructs this many blocks.
pub const MAX_PLAYERS_ARRAY_SAFE: usize = 102;

/// The size of a [`PlayerStats`] block, `sizeof(PlayerStats_t)`.
///
/// Verified in the 64-bit Windows `server.dll`, whose `FindPlayerStats` and
/// `IncrementStat` resolution checks. `PlayerStats_t` holds only `int`s, so
/// the Linux size is inferred to be the same; [`GameStats`] checks there that
/// the blocks fit `CTF_GameStats`.
#[doc(alias("PlayerStats_t"))]
pub const PLAYER_STATS_SIZE: usize = 0x794;

/// The slot of `CTFPlayer::ResetScores` in a TF2 player's primary vtable,
/// from the generated binding.
///
/// The game calls it as a player is first spawned (`CTFPlayer::InitialSpawn`),
/// for each player as `mp_restartgame`, a tournament restart, or the end of
/// the wait for players resets every player's scores, and in Mann vs.
/// Machine, from its population manager. `CTFBot` keeps the same function at
/// the slot.
#[doc(alias("ResetScores"))]
pub const RESET_SCORES_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_ResetScores);

/// The score `CalcPlayerScore` gives [`self_test_stats`] without a player.
const SELF_TEST_SCORE: c_int = 9;

/// Where a [`PlayerStats`] block holds its session statistics,
/// `statsAccumulated`, which the scoreboard's Score is computed from.
#[doc(alias("statsAccumulated"))]
pub const STATS_ACCUMULATED: usize = 0x168;

/// Where a [`PlayerStats`] block holds the statistics of the player's current
/// life, `statsCurrentLife`.
#[doc(alias("statsCurrentLife"))]
pub const STATS_CURRENT_LIFE: usize = 0;

/// Where a [`PlayerStats`] block holds the statistics of the current round,
/// `statsCurrentRound`, which the round's score is computed from.
#[doc(alias("statsCurrentRound"))]
pub const STATS_CURRENT_ROUND: usize = 0xb4;

/// `m_iStreaks` elements per player slot: `CTFPlayerShared::kTFStreak_COUNT`.
///
/// `CTFPlayerResource` keeps each slot's streaks together, the slot's group
/// starting at its index times this (`tf_player_resource.cpp:282`).
#[doc(alias("kTFStreak_COUNT"))]
pub const STREAKS_PER_SLOT: usize = sys::CTFPlayerShared_ETFStreak_kTFStreak_COUNT as usize;

/// Score per backstab, `TF_SCORE_BACKSTAB` from
/// `game/shared/tf/tf_shareddefs.h`, as are the other `TF_SCORE_` weights.
pub const TF_SCORE_BACKSTAB: c_int = 1;

/// Bonus points per point of score, `TF_SCORE_BONUS_POINT_DIVISOR`.
pub const TF_SCORE_BONUS_POINT_DIVISOR: c_int = 10;

/// Score per capture, `TF_SCORE_CAPTURE`.
pub const TF_SCORE_CAPTURE: c_int = 2;

/// Score per capture in Mannpower, `TF_SCORE_CAPTURE_POWERUPMODE`.
pub const TF_SCORE_CAPTURE_POWERUPMODE: c_int = 5;

/// Credits collected per point of score, `TF_SCORE_CURRENCY_COLLECTED`.
pub const TF_SCORE_CURRENCY_COLLECTED: c_int = 20;

/// Healing, damage, damage assist, boss damage, healing assist, and damage
/// blocked per point of score in Mann vs. Machine, `TF_SCORE_DAMAGE`.
pub const TF_SCORE_DAMAGE: c_int = 250;

/// Score per death, `TF_SCORE_DEATH`.
pub const TF_SCORE_DEATH: c_int = 0;

/// Score per defense, `TF_SCORE_DEFEND`.
pub const TF_SCORE_DEFEND: c_int = 1;

/// Score per building destroyed, `TF_SCORE_DESTROY_BUILDING`.
pub const TF_SCORE_DESTROY_BUILDING: c_int = 1;

/// Score per flag returned, `TF_SCORE_FLAG_RETURN`.
pub const TF_SCORE_FLAG_RETURN: c_int = 4;

/// Headshots per point of score, `TF_SCORE_HEADSHOT_DIVISOR`.
pub const TF_SCORE_HEADSHOT_DIVISOR: c_int = 2;

/// Healing, damage, damage assist, boss damage, healing assist, and damage
/// blocked per point of score outside Mann vs. Machine,
/// `TF_SCORE_HEAL_HEALTHUNITS_PER_POINT`.
pub const TF_SCORE_HEAL_HEALTHUNITS_PER_POINT: c_int = 600;

/// Score per invulnerability, `TF_SCORE_INVULN`.
pub const TF_SCORE_INVULN: c_int = 1;

/// Score per kill, `TF_SCORE_KILL`.
pub const TF_SCORE_KILL: c_int = 1;

/// Kill assists per point of score, `TF_SCORE_KILL_ASSISTS_PER_POINT`.
pub const TF_SCORE_KILL_ASSISTS_PER_POINT: c_int = 2;

/// Score per kill of a player carrying a Mannpower rune,
/// `TF_SCORE_KILL_RUNECARRIER`.
pub const TF_SCORE_KILL_RUNECARRIER: c_int = 1;

/// Score per revenge, `TF_SCORE_REVENGE`.
pub const TF_SCORE_REVENGE: c_int = 1;

/// Teleports per point of score, `TF_SCORE_TELEPORTS_PER_POINT`.
pub const TF_SCORE_TELEPORTS_PER_POINT: c_int = 2;

/// BLU's team number.
///
/// This is `TF_TEAM_BLUE` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_TEAM_BLUE: c_int = TF_TEAM_RED + 1;

/// RED's team number, the first of the game's own teams.
///
/// This is `TF_TEAM_RED` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_TEAM_RED: c_int = FIRST_GAME_TEAM;

/// The number of statistics a [`RoundStats`] counts, `TFSTAT_TOTAL` from
/// `game/shared/tf/tf_gamestats_shared.h`.
pub const TFSTAT_TOTAL: usize = 45;

/// The last resolution [`GameStats::cached`] kept, with the module it was
/// resolved in.
static CACHE: ModuleCache<Addresses> = ModuleCache::new();

/// The addresses of the game statistics singleton and its functions in a
/// module, as the platform's resolver verified them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Addresses {
	/// `CTFGameRules::CalcPlayerScore`, a [`CalcPlayerScoreFn`].
	calc_player_score: usize,

	/// `CTFGameStats::FindPlayerStats`, a [`FindPlayerStatsFn`].
	find_player_stats: usize,

	/// `CTF_GameStats`.
	instance: NonZeroUsize,

	/// Where `m_aPlayerStats` lies in `CTF_GameStats`, if the resolver
	/// verified it.
	player_stats: Option<usize>,

	/// How many bytes from `instance` belong to `CTF_GameStats`.
	size: usize,
}

/// TF2's game statistics singleton, `CTF_GameStats`, and the functions that
/// find a player's statistics in it and score them, resolved in a game server
/// module.
///
/// It is a plain bundle of the addresses the resolver verified, so it is
/// `Copy`, `Send`, and `Sync`: only its unsafe methods, which must be called
/// on the server's main thread, use them. Holding one does not keep that
/// module loaded.
#[doc(alias("CTFGameStats", "CTF_GameStats"))]
#[derive(Debug, Clone, Copy)]
pub struct GameStats {
	pub(crate) calc_player_score: CalcPlayerScoreFn,
	pub(crate) find_player_stats: FindPlayerStatsFn,
	pub(crate) instance: NonZeroUsize,
	pub(crate) player_stats: Option<usize>,
	pub(crate) size: usize,
}

impl GameStats {
	/// Finds TF2's game statistics in the module whose `CreateInterface`
	/// export is `factory`, as [`Self::resolve`] does, but inspects and tests
	/// each module only once, as the
	/// [module documentation](crate::tf2::scoreboard#caching) describes.
	///
	/// Fails as [`Self::resolve`] does, or with
	/// [`GameStatsError::Unresolved`] if no loaded module contains `factory`. A
	/// failure is not kept: the next call resolves again.
	///
	/// # Safety
	///
	/// - The safety requirements of [`Self::resolve`] hold.
	/// - If an earlier call resolved a module mapped at the same base, with its
	///   `CreateInterface` at the same address, the module containing `factory`
	///   must be that same image, with the code and singleton that call found
	///   unchanged, since a cache hit returns that call's addresses without
	///   inspecting the module. This holds under the assumption
	///   [`ModuleCache`] describes.
	pub unsafe fn cached(factory: CreateInterfaceFn) -> Result<Self, GameStatsError> {
		let factory = factory as usize;

		// SAFETY: The caller keeps the factory's module loaded for this call.
		let key = unsafe { ModuleKey::of(factory) }.map_err(|_| GameStatsError::Unresolved)?;

		let addresses = CACHE.get_or_resolve(key, || {
			// SAFETY: The caller keeps the factory's module loaded, with its image
			// mappings unchanged, for this call, on the server's main thread.
			unsafe { resolve_tested(factory) }
		})?;

		// SAFETY: The resolver verified and tested the addresses in this module:
		// in this call, or, on a cache hit, in an earlier call, for a module
		// mapped at the same base with the same factory, which the caller
		// guarantees is this same, unchanged image, still mapped for this call.
		Ok(unsafe { Self::from_addresses(addresses) })
	}

	/// Types the addresses a platform resolver verified.
	///
	/// # Safety
	///
	/// A platform resolver verified `addresses` in a module whose image is
	/// still mapped at the addresses it inspected.
	unsafe fn from_addresses(addresses: Addresses) -> Self {
		// SAFETY: The resolver verified each address as the entry of the function
		// its type describes, in the module's executable code: on Windows through
		// signatures, their operands, and the native calls between them, on Linux
		// through exact mangled symbols whose live code matches the file. Neither
		// is null, since each lies in a section of the module.
		unsafe {
			Self {
				calc_player_score: transmute::<*const (), CalcPlayerScoreFn>(
					ptr::with_exposed_provenance(addresses.calc_player_score),
				),
				find_player_stats: transmute::<*const (), FindPlayerStatsFn>(
					ptr::with_exposed_provenance(addresses.find_player_stats),
				),
				instance: addresses.instance,
				player_stats: addresses.player_stats,
				size: addresses.size,
			}
		}
	}

	/// Finds TF2's game statistics in the module whose `CreateInterface`
	/// export is `factory`, then tests its `CalcPlayerScore`.
	///
	/// Fails with [`GameStatsError::Unresolved`] unless that module is a game
	/// server module whose code matches retail TF2's, as the
	/// [module documentation](crate::tf2::scoreboard) describes, and with
	/// [`GameStatsError::SelfTestFailed`] if `CalcPlayerScore` does not score a
	/// known set of statistics as the SDK's source does. Nothing is cached:
	/// each call inspects the module again, while [`Self::cached`] keeps the
	/// result.
	///
	/// # Safety
	///
	/// - `factory` is the `CreateInterface` export of a module that stays
	///   loaded, with its image mappings unchanged, for the whole call.
	/// - The call is made on the server's main thread, since its test calls
	///   `CalcPlayerScore` without a player, which reads the game rules if they
	///   exist.
	pub unsafe fn resolve(factory: CreateInterfaceFn) -> Result<Self, GameStatsError> {
		// SAFETY: As the caller promises.
		let addresses = unsafe { resolve_tested(factory as usize) }?;

		// SAFETY: The resolver just verified the addresses in this module, which
		// the caller keeps mapped.
		Ok(unsafe { Self::from_addresses(addresses) })
	}

	/// Scores `stats` through the game's `CTFGameRules::CalcPlayerScore`, as
	/// the scoreboard's Score is computed from a player's session statistics
	/// and the round's score from the round's. `CalcPlayerScore` only reads
	/// `stats`.
	///
	/// With a `player`, the score includes the terms of the
	/// `scoreboard_minigame` attribute, if the player has it. The score is
	/// never negative: the game clamps it at 0. The game sums `int`s, so
	/// statistics far from 0 can overflow it.
	///
	/// # Safety
	///
	/// - The module this was resolved in is still loaded, with its image
	///   mappings unchanged, for the whole call.
	/// - The call is made on the server's main thread.
	/// - `stats` points to a [`RoundStats`] readable for the call, such as a
	///   copy or a block of a [`PlayerStats`] from [`Self::player_stats`].
	/// - `player` is null or points to a live `CTFPlayer`, whose attribute
	///   providers, such as its items, are live, since the game reads the
	///   attribute through them.
	#[doc(alias("CalcPlayerScore"))]
	pub unsafe fn calc_player_score(
		&self,
		stats: *const RoundStats,
		player: *mut sys::CTFPlayer,
	) -> c_int {
		// SAFETY: As the caller promises. The function only reads `stats`, the
		// game rules if they exist, and, with a player, its attributes.
		unsafe { (self.calc_player_score)(stats.cast_mut(), player) }
	}

	/// The address of `CTF_GameStats`, the game's `CTFGameStats` singleton.
	pub fn instance(&self) -> NonNull<c_void> {
		NonNull::with_exposed_provenance(self.instance)
	}

	/// The [`PlayerStats`] block of `player`, whose entity index is `index`,
	/// through the game's `CTFGameStats::FindPlayerStats`.
	///
	/// `None` unless `index` is a player's, from 1 to `MAX_PLAYERS`, and the
	/// game finds the block `index` selects, in an array of
	/// [`MAX_PLAYERS_ARRAY_SAFE`] blocks that lies wholly inside the
	/// singleton. So it is `None` for a player whose entity index is not
	/// `index`, and for one without an edict, whose block `FindPlayerStats`
	/// takes to be the first, which no player has.
	///
	/// The block stays at that address until the module unloads. The game
	/// resets it when a player connects, disconnects, or has its scores reset,
	/// and resets its round statistics every round.
	///
	/// # Safety
	///
	/// - The module this was resolved in is still loaded, with its image
	///   mappings unchanged, for the whole call.
	/// - The call is made on the server's main thread.
	/// - `player` points to a live `CBasePlayer` with a live or null edict.
	#[doc(alias("FindPlayerStats"))]
	pub unsafe fn player_stats(
		&self,
		player: NonNull<sys::CBasePlayer>,
		index: usize,
	) -> Option<NonNull<PlayerStats>> {
		if !(1..MAX_PLAYERS_ARRAY_SAFE).contains(&index) {
			return None;
		}

		// SAFETY: As the caller promises; the singleton is the game's live
		// `CTFGameStats`. The function only reads the player's edict index.
		let stats = unsafe {
			(self.find_player_stats)(
				ptr::with_exposed_provenance_mut(self.instance.get()),
				player.as_ptr(),
			)
		};

		// The block `index` selects must be the one the game found, in an array
		// that lies wholly inside the singleton.
		let array = stats
			.addr()
			.checked_sub(self.instance.get())?
			.checked_sub(index * PLAYER_STATS_SIZE)?;

		if !stats.addr().is_multiple_of(align_of::<c_int>())
			|| array.checked_add(MAX_PLAYERS_ARRAY_SAFE * PLAYER_STATS_SIZE)? > self.size
			|| self.player_stats.is_some_and(|expected| expected != array)
		{
			return None;
		}

		NonNull::new(stats)
	}
}

/// Why TF2's game statistics could not be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum GameStatsError {
	/// `CTFGameRules::CalcPlayerScore` did not score a known set of
	/// statistics as the SDK's source does.
	#[error("the game's score calculation did not pass its test")]
	SelfTestFailed,

	/// The game statistics singleton or its functions were not found, or did
	/// not match retail TF2's, as the
	/// [module documentation](crate::tf2::scoreboard) describes.
	#[error("the game's statistics could not be found")]
	Unresolved,
}

/// A player's statistics, `PlayerStats_t` from
/// `game/shared/tf/tf_gamestats_shared.h`: its current life's, current
/// round's, and session's [`RoundStats`], at [`STATS_CURRENT_LIFE`],
/// [`STATS_CURRENT_ROUND`], and [`STATS_ACCUMULATED`], then statistics the
/// scoreboard does not show, [`PLAYER_STATS_SIZE`] bytes in all.
///
/// Only pointers to it are used; the game owns every block.
#[doc(alias("PlayerStats_t"))]
#[repr(C)]
#[derive(Debug)]
pub struct PlayerStats {
	_opaque: [u8; 0],
}

impl PlayerStats {
	/// The session statistics of the block `this` points to.
	///
	/// The address is computed without being dereferenced.
	#[doc(alias("statsAccumulated"))]
	pub fn accumulated(this: *mut Self) -> *mut RoundStats {
		this.wrapping_byte_add(STATS_ACCUMULATED).cast()
	}

	/// The statistics of the current life of the block `this` points to.
	///
	/// The address is computed without being dereferenced.
	#[doc(alias("statsCurrentLife"))]
	pub fn current_life(this: *mut Self) -> *mut RoundStats {
		this.wrapping_byte_add(STATS_CURRENT_LIFE).cast()
	}

	/// The statistics of the current round of the block `this` points to.
	///
	/// The address is computed without being dereferenced.
	#[doc(alias("statsCurrentRound"))]
	pub fn current_round(this: *mut Self) -> *mut RoundStats {
		this.wrapping_byte_add(STATS_CURRENT_ROUND).cast()
	}
}

/// One count per [`stat`], `RoundStats_t` from
/// `game/shared/tf/tf_gamestats_shared.h`.
#[doc(alias("RoundStats_t"))]
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RoundStats {
	/// The counts, indexed by [`stat`], `m_iStat`.
	#[doc(alias("m_iStat"))]
	pub stat: [c_int; TFSTAT_TOTAL],
}

impl RoundStats {
	/// No statistic counted, as `RoundStats_t::Reset` leaves it.
	pub const ZERO: Self = Self {
		stat: [0; TFSTAT_TOTAL],
	};
}

/// Resolves the game statistics in the module containing `address`, then
/// tests its `CalcPlayerScore`.
///
/// # Safety
///
/// The module containing `address` stays loaded, with its image mappings
/// unchanged, throughout this call, which is made on the server's main
/// thread.
unsafe fn resolve_tested(address: usize) -> Result<Addresses, GameStatsError> {
	// SAFETY: As the caller promises.
	let addresses = unsafe { platform::resolve(address) }.ok_or(GameStatsError::Unresolved)?;

	// SAFETY: The resolver just verified the address in the module, which the
	// caller keeps mapped, as the entry of `CalcPlayerScore`.
	let calc = unsafe {
		transmute::<*const (), CalcPlayerScoreFn>(ptr::with_exposed_provenance(
			addresses.calc_player_score,
		))
	};

	// SAFETY: The caller calls on the main thread with the module loaded.
	if unsafe { self_test(calc) } {
		Ok(addresses)
	} else {
		Err(GameStatsError::SelfTestFailed)
	}
}

/// Whether `calc` scores [`self_test_stats`] without a player as the SDK's
/// `CalcPlayerScore` does, [`SELF_TEST_SCORE`], and leaves them unchanged.
///
/// It proves the statistics' indices, that they are `int`s, and the weight of
/// [`stat::KILLS_RUNECARRIER`], which no game mode or attribute changes:
/// without a player, the `scoreboard_minigame` attribute is not read.
///
/// # Safety
///
/// `calc` is a live `CalcPlayerScore`, called on the server's main thread.
unsafe fn self_test(calc: CalcPlayerScoreFn) -> bool {
	let mut stats = self_test_stats();

	// SAFETY: As the caller promises; `stats` is a complete `RoundStats` that
	// lives through the call, and the player is null.
	let score = unsafe { calc(&mut stats, ptr::null_mut()) };

	score == SELF_TEST_SCORE && stats == self_test_stats()
}

/// The statistics [`self_test`] scores: 7 kills of rune carriers and 2 kills,
/// worth [`TF_SCORE_KILL_RUNECARRIER`] and [`TF_SCORE_KILL`] each.
fn self_test_stats() -> RoundStats {
	let mut stats = RoundStats::ZERO;

	stats.stat[stat::KILLS_RUNECARRIER] = 7;
	stats.stat[stat::KILLS] = 2;

	stats
}

/// The indices of a [`RoundStats`]'s statistics, `TFStatType_t` from
/// `game/shared/tf/tf_gamestats_shared.h`, named without their `TFSTAT_`
/// prefix.
///
/// [`TF_SCORE_KILL`] and the other `TF_SCORE_` weights
/// count some of them toward a player's score.
#[doc(alias("TFStatType_t"))]
pub mod stat {
	/// Ammunition kits picked up.
	#[doc(alias("TFSTAT_AMMOKITS"))]
	pub const AMMOKITS: usize = 27;

	/// Backstabs.
	#[doc(alias("TFSTAT_BACKSTABS"))]
	pub const BACKSTABS: usize = 17;

	/// Damage dealt by explosions.
	#[doc(alias("TFSTAT_BLASTDAMAGE"))]
	pub const BLASTDAMAGE: usize = 24;

	/// Bonus points, such as those for helping teammates.
	#[doc(alias("TFSTAT_BONUS_POINTS"))]
	pub const BONUS_POINTS: usize = 23;

	/// Buildings built.
	#[doc(alias("TFSTAT_BUILDINGSBUILT"))]
	pub const BUILDINGSBUILT: usize = 19;

	/// Enemy buildings destroyed.
	#[doc(alias("TFSTAT_BUILDINGSDESTROYED"))]
	pub const BUILDINGSDESTROYED: usize = 11;

	/// Captures of control points and flags.
	#[doc(alias("TFSTAT_CAPTURES"))]
	pub const CAPTURES: usize = 6;

	/// Changes of class.
	#[doc(alias("TFSTAT_CLASSCHANGES"))]
	pub const CLASSCHANGES: usize = 28;

	/// Critical hits.
	#[doc(alias("TFSTAT_CRITS"))]
	pub const CRITS: usize = 29;

	/// Credits collected in Mann vs. Machine.
	#[doc(alias("TFSTAT_CURRENCY_COLLECTED"))]
	pub const CURRENCY_COLLECTED: usize = 31;

	/// Damage dealt.
	#[doc(alias("TFSTAT_DAMAGE"))]
	pub const DAMAGE: usize = 5;

	/// Damage dealt by others with this player's help.
	#[doc(alias("TFSTAT_DAMAGE_ASSIST"))]
	pub const DAMAGE_ASSIST: usize = 32;

	/// Damage blocked.
	#[doc(alias("TFSTAT_DAMAGE_BLOCKED"))]
	pub const DAMAGE_BLOCKED: usize = 35;

	/// Damage dealt to bosses.
	#[doc(alias("TFSTAT_DAMAGE_BOSS"))]
	pub const DAMAGE_BOSS: usize = 34;

	/// Damage dealt at range.
	#[doc(alias("TFSTAT_DAMAGE_RANGED"))]
	pub const DAMAGE_RANGED: usize = 36;

	/// Damage dealt at range by crit-boosted hits.
	#[doc(alias("TFSTAT_DAMAGE_RANGED_CRIT_BOOSTED"))]
	pub const DAMAGE_RANGED_CRIT_BOOSTED: usize = 38;

	/// Damage dealt at range by random critical hits.
	#[doc(alias("TFSTAT_DAMAGE_RANGED_CRIT_RANDOM"))]
	pub const DAMAGE_RANGED_CRIT_RANDOM: usize = 37;

	/// Damage taken.
	#[doc(alias("TFSTAT_DAMAGETAKEN"))]
	pub const DAMAGETAKEN: usize = 25;

	/// Deaths.
	#[doc(alias("TFSTAT_DEATHS"))]
	pub const DEATHS: usize = 4;

	/// Defenses of control points and flags.
	#[doc(alias("TFSTAT_DEFENSES"))]
	pub const DEFENSES: usize = 7;

	/// Dominations.
	#[doc(alias("TFSTAT_DOMINATIONS"))]
	pub const DOMINATIONS: usize = 8;

	/// Damage dealt by fire.
	#[doc(alias("TFSTAT_FIREDAMAGE"))]
	pub const FIREDAMAGE: usize = 22;

	/// Flags returned.
	#[doc(alias("TFSTAT_FLAGRETURNS"))]
	pub const FLAGRETURNS: usize = 44;

	/// Headshots.
	#[doc(alias("TFSTAT_HEADSHOTS"))]
	pub const HEADSHOTS: usize = 12;

	/// Healing given.
	#[doc(alias("TFSTAT_HEALING"))]
	pub const HEALING: usize = 14;

	/// Healing given by others with this player's help.
	#[doc(alias("TFSTAT_HEALING_ASSIST"))]
	pub const HEALING_ASSIST: usize = 33;

	/// Health kits picked up.
	#[doc(alias("TFSTAT_HEALTHKITS"))]
	pub const HEALTHKITS: usize = 26;

	/// Health leached.
	#[doc(alias("TFSTAT_HEALTHLEACHED"))]
	pub const HEALTHLEACHED: usize = 18;

	/// Invulnerabilities given.
	#[doc(alias("TFSTAT_INVULNS"))]
	pub const INVULNS: usize = 15;

	/// Kill assists.
	#[doc(alias("TFSTAT_KILLASSISTS"))]
	pub const KILLASSISTS: usize = 16;

	/// Kills.
	#[doc(alias("TFSTAT_KILLS"))]
	pub const KILLS: usize = 3;

	/// Kills of players carrying a Mannpower rune, which the game counts for
	/// any rune carrier, in any mode. Each scores a point and has no other
	/// effect.
	#[doc(alias("TFSTAT_KILLS_RUNECARRIER"))]
	pub const KILLS_RUNECARRIER: usize = 43;

	/// The longest kill streak.
	#[doc(alias("TFSTAT_KILLSTREAK_MAX"))]
	pub const KILLSTREAK_MAX: usize = 42;

	/// The most kills of one sentry gun.
	#[doc(alias("TFSTAT_MAXSENTRYKILLS"))]
	pub const MAXSENTRYKILLS: usize = 20;

	/// Seconds played.
	#[doc(alias("TFSTAT_PLAYTIME"))]
	pub const PLAYTIME: usize = 13;

	/// Points scored, which the game counts per life only.
	#[doc(alias("TFSTAT_POINTSSCORED"))]
	pub const POINTSSCORED: usize = 10;

	/// Revenges.
	#[doc(alias("TFSTAT_REVENGE"))]
	pub const REVENGE: usize = 9;

	/// Revivals.
	#[doc(alias("TFSTAT_REVIVED"))]
	pub const REVIVED: usize = 39;

	/// Shots fired.
	#[doc(alias("TFSTAT_SHOTS_FIRED"))]
	pub const SHOTS_FIRED: usize = 2;

	/// Shots that hit.
	#[doc(alias("TFSTAT_SHOTS_HIT"))]
	pub const SHOTS_HIT: usize = 1;

	/// Suicides.
	#[doc(alias("TFSTAT_SUICIDES"))]
	pub const SUICIDES: usize = 30;

	/// Teleports given by this player's teleporters.
	#[doc(alias("TFSTAT_TELEPORTS"))]
	pub const TELEPORTS: usize = 21;

	/// Hits with throwables.
	#[doc(alias("TFSTAT_THROWABLEHIT"))]
	pub const THROWABLEHIT: usize = 40;

	/// Kills with throwables.
	#[doc(alias("TFSTAT_THROWABLEKILL"))]
	pub const THROWABLEKILL: usize = 41;

	/// No statistic, which the game never counts.
	#[doc(alias("TFSTAT_UNDEFINED"))]
	pub const UNDEFINED: usize = 0;
}
