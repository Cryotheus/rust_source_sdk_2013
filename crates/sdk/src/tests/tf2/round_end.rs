//! Tests of `crate::tf2::round_end`: ending fake game rules' rounds through
//! their vtable, and the checks made first.

use super::*;
use crate::test_support::entities::MockEntity;
use crate::test_support::server::mock_server;
use crate::test_support::server_tools::MockTools;
use crate::test_support::tf2::game_rules::round_rules_proxy;
use crate::test_support::tf2::game_rules::{FLAGS, KOTH_TIMERS, ROUND_STATE, World};
use crate::tf2::game_mode::Holiday;
use sdk_raw::entities::NUM_SERIAL_NUM_SHIFT_BITS;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use sdk_raw::tf2::game_mode::HOLIDAY_HALLOWEEN;
use sdk_raw::tf2::game_rules::{GR_STATE_RND_RUNNING, GR_STATE_TEAM_WIN};

use sdk_raw::tf2::objectives::{
	WINREASON_ALL_POINTS_CAPTURED, WINREASON_DEFEND_UNTIL_TIME_LIMIT, WINREASON_TIMELIMIT,
};

use sdk_raw::tf2::scoreboard::{TF_TEAM_BLUE, TF_TEAM_RED};
use std::cell::RefCell;
use std::ptr::null_mut;

/// A call of the fake game rules' methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Call {
	/// `SetWinningTeam`, with its receiver and arguments.
	Win(*mut sys::CTFGameRules, c_int, c_int, bool, bool, bool, bool),

	/// `SetStalemate`, with its receiver and arguments.
	Stalemate(*mut sys::CTFGameRules, c_int, bool, bool),
}

thread_local! {
	/// The calls of the fake game rules' methods, in order.
	static CALLS: RefCell<Vec<Call>> = const { RefCell::new(Vec::new()) };
}

/// `CTFGameRules::IsHolidayActive`, for which only Halloween is active.
unsafe extern "C" fn is_holiday_active(_: *const sys::CTFGameRules, holiday: c_int) -> bool {
	holiday == HOLIDAY_HALLOWEEN
}

/// `CTFGameRules::SetStalemate`, which records the call.
unsafe extern "C" fn set_stalemate(
	this: *mut sys::CTFGameRules,
	reason: c_int,
	force_map_reset: bool,
	switch_teams: bool,
) {
	CALLS.with_borrow_mut(|calls| {
		calls.push(Call::Stalemate(this, reason, force_map_reset, switch_teams));
	});
}

/// `CTFGameRules::SetWinningTeam`, which records the call.
unsafe extern "C" fn set_winning_team(
	this: *mut sys::CTFGameRules,
	team: c_int,
	reason: c_int,
	force_map_reset: bool,
	switch_teams: bool,
	dont_add_score: bool,
	final_round: bool,
) {
	CALLS.with_borrow_mut(|calls| {
		calls.push(Call::Win(
			this,
			team,
			reason,
			force_map_reset,
			switch_teams,
			dont_add_score,
			final_round,
		));
	});
}

/// Fake game rules, whose vtable records round ends, with mock tools whose
/// entity list holds `timers` under their handles, and the rules' object.
///
/// Mock entities reset the networking the world sets up, so the timers are
/// built before.
fn rules_with_vtable(
	timers: &mut [(MockEntity, EntityHandle)],
) -> (World, MockTools, *mut sys::CTFGameRules) {
	let tools = MockTools::new(null_mut());

	for (timer, handle) in timers {
		tools.list(timer.as_ptr(), *handle);
	}

	let world = World::new(Some(round_rules_proxy));

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the vtable
	// being built.
	let vtable = unsafe {
		mock_vtable::<sys::CTFGameRules__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).CTFGameRules_IsHolidayActive).write(is_holiday_active);
			(&raw mut (*vtable).CTFGameRules_SetStalemate).write(set_stalemate);
			(&raw mut (*vtable).CTFGameRules_SetWinningTeam).write(set_winning_team);
		})
	};

	world.set_vtable(Box::leak(vtable));
	world.put_int(true, ROUND_STATE, GR_STATE_RND_RUNNING);
	CALLS.take();

	let rules = world.rules.cast();

	(world, tools, rules)
}

#[test]
fn rounds_end_through_the_game_rules_vtable() {
	let (world, tools, object) = rules_with_vtable(&mut []);
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();
	let tools = tools.tools();

	rules
		.set_winning_team(
			tools,
			Some(ScoringTeam::Blue),
			WinReason::AllPointsCaptured,
			WinOptions::default(),
		)
		.unwrap();
	rules
		.set_winning_team(
			tools,
			None,
			WinReason::TimeLimit,
			WinOptions {
				reset_map: false,
				switch_teams: true,
				add_score: false,
				final_round: true,
			},
		)
		.unwrap();
	rules
		.set_stalemate(tools, StalemateReason::Timer, StalemateOptions::default())
		.unwrap();
	rules
		.set_stalemate(
			tools,
			StalemateReason::JoinedMidRound,
			StalemateOptions {
				reset_map: false,
				switch_teams: true,
			},
		)
		.unwrap();

	assert_eq!(
		CALLS.take(),
		[
			Call::Win(
				object,
				TF_TEAM_BLUE,
				WINREASON_ALL_POINTS_CAPTURED,
				true,
				false,
				false,
				false
			),
			Call::Win(object, 0, WINREASON_TIMELIMIT, false, true, true, true),
			Call::Stalemate(object, raw::STALEMATE_TIMER, true, false),
			Call::Stalemate(object, raw::STALEMATE_JOIN_MID, false, true),
		]
	);

	// A round a team already won is not ended again.
	world.put_int(true, ROUND_STATE, GR_STATE_TEAM_WIN);

	assert_eq!(
		rules.set_winning_team(
			tools,
			Some(ScoringTeam::Red),
			WinReason::OpponentsDead,
			WinOptions::default(),
		),
		Err(RoundEndError::AlreadyWon)
	);
	assert_eq!(
		rules.set_stalemate(tools, StalemateReason::Timer, StalemateOptions::default()),
		Err(RoundEndError::AlreadyWon)
	);
	assert_eq!(CALLS.take(), []);
}

#[test]
fn king_of_the_hill_rounds_end_only_with_both_timers() {
	let red = EntityHandle::from_raw(1 << NUM_SERIAL_NUM_SHIFT_BITS | 0x40);
	let blue = EntityHandle::from_raw(3 << NUM_SERIAL_NUM_SHIFT_BITS | 0x41);

	let mut timers = [
		(MockEntity::new(red.to_raw()), red),
		(MockEntity::new(blue.to_raw()), blue),
	];

	let (world, tools, object) = rules_with_vtable(&mut timers);
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();
	let tools = tools.tools();
	let win = |rules: GameRules<'_>| {
		rules.set_winning_team(
			tools,
			Some(ScoringTeam::Red),
			WinReason::DefendedUntilTimeLimit,
			WinOptions::default(),
		)
	};

	// `m_bPlayingKoth` is the first flag.
	world.put_byte(false, FLAGS, 1);
	world.put_int(false, KOTH_TIMERS, EntityHandle::INVALID.to_raw() as c_int);
	world.put_int(
		false,
		KOTH_TIMERS + 4,
		EntityHandle::INVALID.to_raw() as c_int,
	);

	assert_eq!(rules.koth_timer(ScoringTeam::Red), Ok(None));
	assert_eq!(
		win(rules),
		Err(RoundEndError::NoKothTimer(ScoringTeam::Red))
	);

	// A handle whose entity was removed does not count.
	world.put_int(false, KOTH_TIMERS, red.to_raw() as c_int);
	world.put_int(
		false,
		KOTH_TIMERS + 4,
		(blue.to_raw() + (1 << NUM_SERIAL_NUM_SHIFT_BITS)) as c_int,
	);

	assert_eq!(rules.koth_timer(ScoringTeam::Red), Ok(Some(red)));
	assert_eq!(
		win(rules),
		Err(RoundEndError::NoKothTimer(ScoringTeam::Blue))
	);
	assert_eq!(
		rules.set_stalemate(tools, StalemateReason::Timer, StalemateOptions::default()),
		Err(RoundEndError::NoKothTimer(ScoringTeam::Blue))
	);
	assert_eq!(CALLS.take(), []);

	world.put_int(false, KOTH_TIMERS + 4, blue.to_raw() as c_int);

	assert_eq!(rules.koth_timer(ScoringTeam::Blue), Ok(Some(blue)));
	assert_eq!(win(rules), Ok(()));
	assert_eq!(
		CALLS.take(),
		[Call::Win(
			object,
			TF_TEAM_RED,
			WINREASON_DEFEND_UNTIL_TIME_LIMIT,
			true,
			false,
			false,
			false
		)]
	);

	// Levels that do not play King of the Hill need no timers.
	world.put_byte(false, FLAGS, 0);
	world.put_int(false, KOTH_TIMERS, EntityHandle::INVALID.to_raw() as c_int);

	assert_eq!(win(rules), Ok(()));
	assert_eq!(CALLS.take().len(), 1);
}

#[test]
fn holidays_are_asked_of_the_game_rules() {
	let (_world, _tools, _) = rules_with_vtable(&mut []);
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();

	assert!(rules.is_holiday_active(Holiday::Halloween));
	assert!(!rules.is_holiday_active(Holiday::FullMoon));
}

#[test]
fn stalemate_reasons_round_trip_through_their_raw_values() {
	for reason in StalemateReason::ALL {
		assert_eq!(StalemateReason::from_raw(reason.to_raw()), Some(reason));
	}

	assert_eq!(StalemateReason::from_raw(3), None);
	assert_eq!(StalemateReason::Timer.to_raw(), raw::STALEMATE_TIMER);
}
