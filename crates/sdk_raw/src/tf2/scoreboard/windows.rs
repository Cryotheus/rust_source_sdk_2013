//! Resolution in retail TF2's 64-bit Windows `server.dll`, through signatures
//! of `CalcPlayerScore`, of `IncrementStat`, and of
//! `CTFPlayerResource::UpdateConnectedPlayer`, which finds a player's
//! statistics and scores them. Their operands must agree with the layout this
//! module declares, the calls between them must agree with each other, and
//! the singleton they reference must hold `CTFGameStats`'s vtables.
//!
//! Wildcards are relative operands and offsets of other classes' members.

#[cfg(test)]
#[path = "../../tests/tf2/scoreboard/windows.rs"]
mod tests;

use super::{
	Addresses, GAME_STATS_PLAYER_STATS, MAX_PLAYERS_ARRAY_SAFE, PLAYER_STATS_SIZE,
	STATS_ACCUMULATED, STATS_CURRENT_ROUND, stat,
};

use crate::sig;
use crate::util::{Image, SignaturePattern, exact_u32, pattern, relative, u32_at, word_at};
use std::ffi::c_int;
use std::num::NonZeroUsize;

// The operands the signatures fix are the layout's: `UpdateConnectedPlayer`
// scores and copies the session and round blocks, and `CalcPlayerScore`
// starts by reading the healing statistic.
const _: () = {
	assert!(matches!(
		exact_u32(CALC_ACCUMULATED_SCORE, CALC_ACCUMULATED_SCORE_BLOCK),
		Some(offset) if offset as usize == STATS_ACCUMULATED
	));

	assert!(matches!(
		exact_u32(FIND_PLAYER_STATS_CALL, FIND_PLAYER_STATS_CALL_ACCUMULATED),
		Some(offset) if offset as usize == STATS_ACCUMULATED
	));

	assert!(matches!(
		exact_u32(FIND_PLAYER_STATS_CALL, FIND_PLAYER_STATS_CALL_CURRENT_ROUND),
		Some(offset) if offset as usize == STATS_CURRENT_ROUND
	));

	assert!(crate::util::is_exact(
		CALC_PLAYER_SCORE,
		CALC_PLAYER_SCORE_HEALING,
		(stat::HEALING * size_of::<c_int>()) as u8
	));
};

/// `CTFPlayerResource::UpdateConnectedPlayer`'s call of `CalcPlayerScore` for
/// a player's session statistics: `lea rcx, [rbx + statsAccumulated]; mov
/// rdx, r14; call CalcPlayerScore`.
const CALC_ACCUMULATED_SCORE: &[SignaturePattern] =
	&sig![0x48 0x8d 0x8b 0x68 0x01 0 0 0x49 0x8b 0xd6 0xe8 ? ? ? ?];

/// Where [`CALC_ACCUMULATED_SCORE`] encodes [`STATS_ACCUMULATED`].
const CALC_ACCUMULATED_SCORE_BLOCK: usize = 3;

/// Where [`CALC_ACCUMULATED_SCORE`] calls `CalcPlayerScore`.
const CALC_ACCUMULATED_SCORE_CALL: usize = 10;

/// The prologue of `CTFGameRules::CalcPlayerScore`, up to its check of the
/// healing statistic against 10,000,000.
const CALC_PLAYER_SCORE: &[SignaturePattern] = &sig![0x40 0x53 0x48 0x83 0xec 0x30 0x4c 0x8b 0xd2 0x48 0x8b 0xd9 0x48 0x85 0xc9 0x75 0x08 0x33 0xc0 0x48 0x83 0xc4 0x30 0x5b 0xc3 0x44 0x8b 0x41 0x38 0x48 0x8b 0x05 ? ? ? ? 0x48 0x89 0x6c 0x24 0x40 0x33 0xed 0x41 0x81 0xf8 0x80 0x96 0x98 0x00];

/// Where [`CALC_PLAYER_SCORE`] reads the healing statistic.
const CALC_PLAYER_SCORE_HEALING: usize = 28;

/// Where `CalcPlayerScore` loads the name of the `scoreboard_minigame`
/// attribute: `lea rdx, [rip + name]`, see [`LEA_RDX`].
const CALC_PLAYER_SCORE_MINIGAME: usize = 0x10e;

/// The prologue of `CTFGameStats::FindPlayerStats`, up to its return of the
/// block of the player's entity index: `this + base + index * stride`.
const FIND_PLAYER_STATS: &[SignaturePattern] = &sig![0x4c 0x8b 0xc1 0x48 0x85 0xd2 0x75 0x03 0x33 0xc0 0xc3 0x48 0x8b 0x42 ? 0x48 0x85 0xc0 0x74 ? 0x0f 0xbf 0x48 ? 0x48 0x63 0xc1 0x48 0x69 0xc0 ? ? ? ? 0x48 0x05 ? ? ? ? 0x49 0x03 0xc0 0xc3];

/// Where [`FIND_PLAYER_STATS`] encodes the array's offset in the singleton.
const FIND_PLAYER_STATS_BASE: usize = 36;

/// `CTFPlayerResource::UpdateConnectedPlayer`'s call of `FindPlayerStats` on
/// `CTF_GameStats`, then of `UpdateStats` with the block's session and round
/// statistics.
const FIND_PLAYER_STATS_CALL: &[SignaturePattern] = &sig![0x49 0x8b 0xd6 0x48 0x8d 0x0d ? ? ? ? 0xe8 ? ? ? ? 0x48 0x8b 0xd8 0x48 0x85 0xc0 0x0f 0x84 ? ? ? ? 0x48 0x8d 0x90 0x68 0x01 0 0 0x45 0x33 0xc9 0x49 0x8d 0x8e ? ? ? ? 0x4d 0x8b 0xc6 0xe8 ? ? ? ? 0x48 0x8d 0x93 0xb4 0 0 0];

/// Where [`FIND_PLAYER_STATS_CALL`] encodes [`STATS_ACCUMULATED`].
const FIND_PLAYER_STATS_CALL_ACCUMULATED: usize = 30;

/// Where [`FIND_PLAYER_STATS_CALL`] calls `FindPlayerStats`.
const FIND_PLAYER_STATS_CALL_AT: usize = 10;

/// Where [`FIND_PLAYER_STATS_CALL`] encodes [`STATS_CURRENT_ROUND`].
const FIND_PLAYER_STATS_CALL_CURRENT_ROUND: usize = 55;

/// Where the `rip`-relative displacement of [`FIND_PLAYER_STATS_CALL`]'s
/// `lea rcx, [rip + CTF_GameStats]` starts.
const FIND_PLAYER_STATS_CALL_INSTANCE: usize = 6;

/// Where [`FIND_PLAYER_STATS`] encodes the stride of the array.
const FIND_PLAYER_STATS_STRIDE: usize = 30;

/// The alignment of function entries in the module.
const FUNCTION_ALIGNMENT: usize = 16;

/// The bytes of `CTF_GameStats` that resolution requires in a writable
/// section: up to the end of its array of [`PlayerStats`](super::PlayerStats)
/// blocks.
const GAME_STATS_SIZE: usize = GAME_STATS_PLAYER_STATS + MAX_PLAYERS_ARRAY_SAFE * PLAYER_STATS_SIZE;

/// The offsets of the subobjects of `CTFGameStats` whose vtable pointers
/// `CTF_GameStats` must hold: its own, then those of its bases
/// `CGameEventListener` and `CAutoGameSystem`.
const GAME_STATS_VTABLES: [usize; 3] = [0, 0x78, 0x88];

/// `CTFGameStats::IncrementStat`'s addition of a statistic to a player's
/// block: `imul rdx, rax, stride; add rdx, base; add rdx, rbp`, then
/// additions to the life, round, three map, and session blocks, in that order.
const INCREMENT_STAT: &[SignaturePattern] = &sig![0x48 0x69 0xd0 ? ? ? ? 0x48 0x81 0xc2 ? ? ? ? 0x48 0x03 0xd5 0x01 0x1c 0xb2 0x01 0x9c 0xb2 ? ? ? ? 0x01 0x9c 0xb2 ? ? ? ? 0x01 0x9c 0xb2 ? ? ? ? 0x01 0x9c 0xb2 ? ? ? ? 0x01 0x9c 0xb2 ? ? ? ?];

/// Where [`INCREMENT_STAT`] encodes the session block's offset, the fourth of
/// its five block displacements.
const INCREMENT_STAT_ACCUMULATED: usize = 44;

/// Where [`INCREMENT_STAT`] encodes the array's offset in the singleton.
const INCREMENT_STAT_BASE: usize = 10;

/// Where [`INCREMENT_STAT`] encodes the round block's offset, the first of its
/// five block displacements.
const INCREMENT_STAT_CURRENT_ROUND: usize = 23;

/// Where [`INCREMENT_STAT`] encodes the stride of the array.
const INCREMENT_STAT_STRIDE: usize = 3;

/// `lea rdx, [rip + displacement]`, with the displacement at
/// [`LEA_RDX_OPERAND`].
const LEA_RDX: &[SignaturePattern] = &sig![0x48 0x8d 0x15 ? ? ? ?];

/// Where [`LEA_RDX`]'s displacement starts.
const LEA_RDX_OPERAND: usize = 3;

/// The name of the attribute `CalcPlayerScore` reads, with its terminator.
const MINIGAME_ATTRIBUTE: &[u8] = b"scoreboard_minigame\0";

/// How far after [`FIND_PLAYER_STATS_CALL`] [`CALC_ACCUMULATED_SCORE`] may
/// lie, so that both are in `UpdateConnectedPlayer`.
const UPDATE_CONNECTED_PLAYER_SPAN: usize = 0x400;

/// Whether the snapshot of the object at `instance` holds, at each of
/// [`GAME_STATS_VTABLES`], the only one of `tables` for that subobject offset.
fn holds_game_stats_vtables(image: &Image, tables: &[(usize, usize)], instance: usize) -> bool {
	GAME_STATS_VTABLES.iter().all(|&offset| {
		let mut candidates = tables.iter().filter(|(found, _)| *found == offset);

		let (Some(&(_, table)), None) = (candidates.next(), candidates.next()) else {
			return false;
		};

		instance
			.checked_add(offset)
			.and_then(|pointer| image.read(pointer, size_of::<usize>()))
			.and_then(|bytes| word_at(bytes, 0))
			== Some(table)
	})
}

/// Whether `bytes` holds `value` as a little-endian `u32` at `at`.
fn is(bytes: &[u8], at: usize, value: usize) -> bool {
	u32_at(bytes, at).is_some_and(|found| found as usize == value)
}

/// Resolves the game statistics in the module containing `address`.
///
/// # Safety
///
/// The module containing `address` stays loaded throughout this call.
pub(super) unsafe fn resolve(address: usize) -> Option<Addresses> {
	// SAFETY: The caller keeps the module loaded throughout this call.
	let image = unsafe { Image::load(address) }.ok()?;

	resolve_image(&image)
}

/// Resolves the game statistics in a snapshot of the module, which must hold
/// the singleton as the game constructed it.
fn resolve_image(image: &Image) -> Option<Addresses> {
	let calc_player_score = image.unique(CALC_PLAYER_SCORE, FUNCTION_ALIGNMENT)?;

	// The function must read the attribute that identifies it.
	let reference = calc_player_score + CALC_PLAYER_SCORE_MINIGAME;
	let lea = image.read(reference, LEA_RDX.len())?;

	if !pattern(lea, LEA_RDX) {
		return None;
	}

	let name = relative(reference, lea, LEA_RDX_OPERAND)?;

	if image.executable(name) || image.read(name, MINIGAME_ATTRIBUTE.len())? != MINIGAME_ATTRIBUTE {
		return None;
	}

	// `UpdateConnectedPlayer` must call it with a player's session block, after
	// finding that block in the singleton.
	let score = image.unique(CALC_ACCUMULATED_SCORE, 1)?;
	let find = image.unique(FIND_PLAYER_STATS_CALL, 1)?;

	if image.call(score + CALC_ACCUMULATED_SCORE_CALL)? != calc_player_score
		|| score.checked_sub(find)? > UPDATE_CONNECTED_PLAYER_SPAN
	{
		return None;
	}

	let call = image.read(find, FIND_PLAYER_STATS_CALL.len())?;
	let instance = relative(find, call, FIND_PLAYER_STATS_CALL_INSTANCE)?;
	let find_player_stats = image.call(find + FIND_PLAYER_STATS_CALL_AT)?;

	// `FindPlayerStats` and `IncrementStat` must both index the array this
	// module declares, and `IncrementStat` must add to its round and session
	// blocks where this module declares them.
	let body = image.read(find_player_stats, FIND_PLAYER_STATS.len())?;
	let increment = image.read(image.unique(INCREMENT_STAT, 1)?, INCREMENT_STAT.len())?;

	if !pattern(body, FIND_PLAYER_STATS)
		|| !is(body, FIND_PLAYER_STATS_STRIDE, PLAYER_STATS_SIZE)
		|| !is(body, FIND_PLAYER_STATS_BASE, GAME_STATS_PLAYER_STATS)
		|| !is(increment, INCREMENT_STAT_STRIDE, PLAYER_STATS_SIZE)
		|| !is(increment, INCREMENT_STAT_BASE, GAME_STATS_PLAYER_STATS)
		|| !is(increment, INCREMENT_STAT_CURRENT_ROUND, STATS_CURRENT_ROUND)
		|| !is(increment, INCREMENT_STAT_ACCUMULATED, STATS_ACCUMULATED)
	{
		return None;
	}

	// The singleton's array must be writable, and the singleton must hold the
	// vtables of a `CTFGameStats`.
	if !image.contains(instance, GAME_STATS_SIZE, false, true)
		|| !holds_game_stats_vtables(image, &image.vtables("CTFGameStats", 0), instance)
	{
		return None;
	}

	Some(Addresses {
		calc_player_score,
		find_player_stats,
		instance: NonZeroUsize::new(instance)?,
		player_stats: Some(GAME_STATS_PLAYER_STATS),
		size: GAME_STATS_SIZE,
	})
}
