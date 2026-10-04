//! Tests of finding a player's statistics block and of the test of
//! `CalcPlayerScore`, against fakes of the game's functions.

use super::*;

/// Where the fake singleton holds its array, as the Windows build does.
const ARRAY: usize = 0xd8;

/// A fake player, which holds the entity index its fake edict reports.
#[repr(C)]
struct Player {
	index: c_int,
}

/// A fake singleton, large enough for the array at [`ARRAY`].
#[repr(C, align(8))]
struct Singleton([u8; ARRAY + MAX_PLAYERS_ARRAY_SAFE * PLAYER_STATS_SIZE]);

/// `CalcPlayerScore` without a player, as the SDK's source computes it, for
/// the statistics the self-test sets.
unsafe extern "C" fn calc(stats: *mut RoundStats, _: *mut sys::CTFPlayer) -> c_int {
	// SAFETY: The test passes a complete `RoundStats`.
	let stats = unsafe { stats.read() };

	(stats.stat[stat::KILLS] * TF_SCORE_KILL
		+ stats.stat[stat::KILLS_RUNECARRIER] * TF_SCORE_KILL_RUNECARRIER)
		.max(0)
}

/// `CalcPlayerScore` of a build whose statistics are shifted by one.
unsafe extern "C" fn calc_shifted(stats: *mut RoundStats, _: *mut sys::CTFPlayer) -> c_int {
	// SAFETY: The test passes a complete `RoundStats`.
	let stats = unsafe { stats.read() };

	stats.stat[stat::KILLS + 1] + stats.stat[stat::KILLS_RUNECARRIER + 1]
}

/// `CalcPlayerScore` that scores correctly, but writes to the statistics.
unsafe extern "C" fn calc_writing(stats: *mut RoundStats, player: *mut sys::CTFPlayer) -> c_int {
	// SAFETY: As for `calc`, and the statistics are writable.
	unsafe {
		let score = calc(stats, player);
		(*stats).stat[stat::DEATHS] = 1;
		score
	}
}

/// `FindPlayerStats` as the game implements it: the block of the player's
/// entity index, without checking it.
unsafe extern "C" fn find(this: *mut c_void, player: *mut sys::CBasePlayer) -> *mut PlayerStats {
	// SAFETY: Tests pass a `Player` for every player.
	let index = unsafe { player.cast::<Player>().read() }.index;

	this.wrapping_byte_add(ARRAY)
		.wrapping_byte_add(index as usize * PLAYER_STATS_SIZE)
		.cast()
}

#[test]
fn player_stats_are_the_blocks_of_their_entity_indices() {
	let mut singleton = Box::new(Singleton([0; _]));
	let instance = NonNull::from(&mut *singleton).cast::<c_void>();
	let stats = GameStats::fake(instance, size_of::<Singleton>(), ARRAY, calc, find);

	for index in [1, 24, MAX_PLAYERS_ARRAY_SAFE - 1] {
		let mut player = Player {
			index: index as c_int,
		};
		let player = NonNull::from(&mut player).cast::<sys::CBasePlayer>();

		// SAFETY: The fakes stand in for the game's functions and singleton.
		let block = unsafe { stats.player_stats(player, index) }.unwrap();

		assert_eq!(
			block.addr().get() - instance.addr().get(),
			ARRAY + index * PLAYER_STATS_SIZE
		);
		assert_eq!(
			PlayerStats::accumulated(block.as_ptr()).addr() - block.addr().get(),
			0x168
		);
		assert_eq!(
			PlayerStats::current_round(block.as_ptr()).addr() - block.addr().get(),
			0xb4
		);

		// A caller's index that is not the game's finds nothing.
		// SAFETY: As above.
		assert!(unsafe { stats.player_stats(player, index % 100 + 1) }.is_none());
	}

	// No player has the first block, which `FindPlayerStats` returns without an
	// edict, nor one past the array.
	for index in [0, MAX_PLAYERS_ARRAY_SAFE] {
		let mut player = Player {
			index: index as c_int,
		};
		let player = NonNull::from(&mut player).cast::<sys::CBasePlayer>();

		// SAFETY: As above.
		assert!(unsafe { stats.player_stats(player, index) }.is_none());
	}

	let mut player = Player { index: 1 };
	let player = NonNull::from(&mut player).cast::<sys::CBasePlayer>();

	// An array that does not fit the singleton, or that lies elsewhere than
	// the resolver verified, finds nothing.
	let short = GameStats::fake(instance, size_of::<Singleton>() - 4, ARRAY, calc, find);
	let moved = GameStats::fake(instance, size_of::<Singleton>(), ARRAY + 4, calc, find);

	// SAFETY: As above.
	unsafe {
		assert!(short.player_stats(player, 1).is_none());
		assert!(moved.player_stats(player, 1).is_none());
	}
}

#[test]
fn the_self_test_requires_the_sdk_indices_and_weights() {
	// SAFETY: The fakes only read, and write, the statistics they are given.
	unsafe {
		assert!(self_test(calc));
		assert!(!self_test(calc_shifted));
		assert!(!self_test(calc_writing));
	}
}
