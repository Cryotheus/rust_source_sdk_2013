//! Builders of TF2's resolved game statistics.

use crate::tf2::scoreboard::{CalcPlayerScoreFn, FindPlayerStatsFn, GameStats};
use std::ffi::c_void;
use std::ptr::NonNull;

/// A [`GameStats`] over fakes: its singleton is `instance`, whose first
/// `size` bytes hold `m_aPlayerStats` at `player_stats`, and its functions are
/// `calc_player_score` and `find_player_stats`.
///
/// For tests only.
pub fn game_stats(
	instance: NonNull<c_void>,
	size: usize,
	player_stats: usize,
	calc_player_score: CalcPlayerScoreFn,
	find_player_stats: FindPlayerStatsFn,
) -> GameStats {
	GameStats {
		calc_player_score,
		find_player_stats,
		instance: instance.addr(),
		player_stats: Some(player_stats),
		size,
	}
}
