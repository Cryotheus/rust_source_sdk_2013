//! Resolution in retail TF2's 64-bit Linux `server_srv.so`, through the
//! symbols of an unstripped build.
//!
//! The symbols and the layout are inferred from the SDK's source and the
//! Itanium ABI, not checked against a retail build: a module that lacks any of
//! them, or whose singleton is too small or not a `CTFGameStats`, is refused.

use super::{Addresses, MAX_PLAYERS_ARRAY_SAFE, PLAYER_STATS_SIZE};
use crate::util::elf::LoadedElf;
use crate::util::word_at;
use std::num::NonZeroUsize;

/// `CTFGameRules::CalcPlayerScore(RoundStats_t *, CTFPlayer *)`.
const CALC_PLAYER_SCORE: &[u8] = b"_ZN12CTFGameRules15CalcPlayerScoreEP12RoundStats_tP9CTFPlayer";

/// `CTFGameStats::FindPlayerStats(CBasePlayer *)`.
const FIND_PLAYER_STATS: &[u8] = b"_ZN12CTFGameStats15FindPlayerStatsEP11CBasePlayer";

/// The game statistics singleton, a global variable, whose name the Itanium
/// ABI does not mangle.
const GAME_STATS: &[u8] = b"CTF_GameStats";

/// The name `CTFGameStats`'s `std::type_info` holds, with its terminator.
const GAME_STATS_TYPE_NAME: &[u8] = b"12CTFGameStats\0";

/// Whether the object at `instance` is a complete `CTFGameStats`, by the
/// Itanium run-time type information its vtable refers to: the offset to its
/// top, two slots before the vtable's first, is 0, and the `std::type_info`
/// one slot before names the class. Every read is a bounded copy of the
/// module's current memory.
fn is_game_stats(elf: &LoadedElf, instance: usize) -> bool {
	let word = |address: usize| {
		elf.read(address, size_of::<usize>())
			.and_then(|bytes| word_at(&bytes, 0))
	};

	let name = (|| {
		let vtable = word(instance)?;

		if word(vtable.checked_sub(2 * size_of::<usize>())?)? != 0 {
			return None;
		}

		let type_info = word(vtable.checked_sub(size_of::<usize>())?)?;

		elf.read(
			word(type_info.checked_add(size_of::<usize>())?)?,
			GAME_STATS_TYPE_NAME.len(),
		)
	})();

	name.as_deref() == Some(GAME_STATS_TYPE_NAME)
}

/// Resolves the game statistics from the symbols of the module containing
/// `address`.
///
/// # Safety
///
/// The module containing `address` stays loaded, with its image mappings
/// unchanged, throughout this call.
pub(super) unsafe fn resolve(address: usize) -> Option<Addresses> {
	// SAFETY: The caller guarantees the module remains loaded and its image
	// mappings remain valid throughout this snapshot.
	let elf = unsafe { LoadedElf::at(address) }.ok()?;
	let (calc_player_score, _) = elf.resolve(CALC_PLAYER_SCORE)?;
	let (find_player_stats, _) = elf.resolve(FIND_PLAYER_STATS)?;
	let (instance, size) = elf.resolve_data(GAME_STATS)?;

	if size < MAX_PLAYERS_ARRAY_SAFE * PLAYER_STATS_SIZE || !is_game_stats(&elf, instance) {
		return None;
	}

	Some(Addresses {
		calc_player_score,
		find_player_stats,
		instance: NonZeroUsize::new(instance)?,
		player_stats: None,
		size,
	})
}
