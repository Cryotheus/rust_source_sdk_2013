//! Tests of round timers, King of the Hill's logic and round wins, through
//! fake entities of their classes.

use super::*;
use crate::Game;
use crate::test_support::entities::take_inputs;
use crate::test_support::server::{mock_server, null_server};

use crate::test_support::tf2::objectives::{
	FIELDS_OFFSET, FakeClass, FakeObjective, bool_prop, expected, float_prop, input, int_prop,
	key_field, received, register_name, take_key_values,
};

use std::ffi::CStr;

const AUTO_COUNTDOWN: usize = FIELDS_OFFSET + 18;
const CAPTURE_WATCH: usize = FIELDS_OFFSET + 37;
const DISABLED: usize = FIELDS_OFFSET + 16;
const END_TIME: usize = FIELDS_OFFSET + 8;
const FORCE_MAP_RESET: usize = FIELDS_OFFSET + 96;
const INITIAL_LENGTH: usize = FIELDS_OFFSET + 24;
const KOTH_INITIAL_LENGTH: usize = FIELDS_OFFSET + 64;
const KOTH_UNLOCK: usize = FIELDS_OFFSET + 68;
const LENGTH: usize = FIELDS_OFFSET + 20;
const MAX_LENGTH: usize = FIELDS_OFFSET + 12;
const PAUSED: usize = FIELDS_OFFSET;
const SETUP_LENGTH: usize = FIELDS_OFFSET + 28;
const SHOW_IN_HUD: usize = FIELDS_OFFSET + 17;
const SHOW_TIME_REMAINING: usize = FIELDS_OFFSET + 19;
const STATE: usize = FIELDS_OFFSET + 32;
const STOPWATCH: usize = FIELDS_OFFSET + 36;
const SWITCH_TEAMS: usize = FIELDS_OFFSET + 97;
const TEAM_NUMBER: usize = FIELDS_OFFSET + 104;
const TIME_REMAINING: usize = FIELDS_OFFSET + 4;
const TOTAL_TIME: usize = FIELDS_OFFSET + 40;
const WIN_REASON: usize = FIELDS_OFFSET + 100;

#[test]
fn koth_logic_sets_each_teams_timer_and_finds_it() {
	use sys::_fieldtypes_FIELD_INTEGER as INTEGER;

	let mut fake = FakeObjective::new(FakeClass {
		maps: vec![
			(
				c"CKothLogic",
				vec![
					key_field(
						c"m_nTimerInitialLength",
						c"timer_length",
						INTEGER,
						KOTH_INITIAL_LENGTH,
						4,
					),
					key_field(
						c"m_nTimeToUnlockPoint",
						c"unlock_point",
						INTEGER,
						KOTH_UNLOCK,
						4,
					),
					input(c"SetRedTimer", INTEGER),
					input(c"SetBlueTimer", INTEGER),
					input(c"AddRedTimer", INTEGER),
					input(c"AddBlueTimer", INTEGER),
				],
			),
			// Mock entities share one class, so the logic's timers are found
			// as entities of the same chain.
			timer_map(),
		],
		base_fields: vec![],
		table: None,
	});
	let scope = ();
	let server = mock_server(&scope);
	let logic = KothLogic::new(server, fake.entity()).unwrap();

	fake.set_int(KOTH_INITIAL_LENGTH, 180);
	fake.set_int(KOTH_UNLOCK, 30);
	assert_eq!(logic.initial_timer_length().unwrap(), 180);
	assert_eq!(logic.unlock_delay().unwrap(), 30);

	take_inputs();
	logic.set_timer(ScoringTeam::Red, 90).unwrap();
	logic.set_timer(ScoringTeam::Blue, 80).unwrap();
	logic.add_timer(ScoringTeam::Red, 5).unwrap();
	logic.add_timer(ScoringTeam::Blue, -5).unwrap();

	assert_eq!(
		received(),
		expected(&[
			(c"SetRedTimer", 90),
			(c"SetBlueTimer", 80),
			(c"AddRedTimer", 5),
			(c"AddBlueTimer", -5),
		])
	);

	// The timers are found by the names the logic gives them.
	assert!(logic.timer(ScoringTeam::Red).unwrap().is_none());
	register_name(fake.world.as_ptr(), c"ZZ_RED_KOTH_TIMER");
	register_name(fake.as_ptr(), c"zz_blue_koth_timer");

	let red = logic.timer(ScoringTeam::Red).unwrap().unwrap();
	let blue = logic.timer(ScoringTeam::Blue).unwrap().unwrap();

	assert_eq!(red.entity().as_ptr(), fake.world.as_ptr());
	assert_eq!(blue.entity().as_ptr(), fake.as_ptr());
}

#[test]
fn objectives_need_their_class_in_tf2() {
	let mut fake = timer();
	let scope = ();

	assert!(RoundTimer::new(mock_server(&scope), fake.entity()).is_ok());
	assert!(matches!(
		RoundWin::new(mock_server(&scope), fake.entity()),
		Err(ObjectiveError::WrongClass {
			expected: "game_round_win"
		})
	));
	assert!(matches!(
		RoundTimer::new(null_server(Game::SourceSdk2013, &scope), fake.entity()),
		Err(ObjectiveError::WrongClass { .. })
	));

	// An entity pending deletion is neither read nor sent inputs.
	let timer = RoundTimer::new(mock_server(&scope), fake.entity()).unwrap();

	fake.mock.set_eflags(sdk_raw::entities::EFL_KILLME);
	assert!(matches!(
		timer.is_paused(),
		Err(ObjectiveError::MarkedForDeletion)
	));
	assert!(matches!(
		timer.pause(),
		Err(ObjectiveError::Input(
			crate::inputs::InputError::MarkedForDeletion
		))
	));
}

#[test]
fn round_wins_read_and_set_their_win() {
	use sys::{
		_fieldtypes_FIELD_BOOLEAN as BOOLEAN, _fieldtypes_FIELD_INTEGER as INTEGER,
		_fieldtypes_FIELD_VOID as VOID,
	};

	let mut fake = FakeObjective::new(FakeClass {
		maps: vec![(
			c"CTeamplayRoundWin",
			vec![
				key_field(
					c"m_bForceMapReset",
					c"force_map_reset",
					BOOLEAN,
					FORCE_MAP_RESET,
					1,
				),
				key_field(
					c"m_bSwitchTeamsOnWin",
					c"switch_teams",
					BOOLEAN,
					SWITCH_TEAMS,
					1,
				),
				key_field(c"m_iWinReason", c"win_reason", INTEGER, WIN_REASON, 4),
				input(c"SetTeam", INTEGER),
				input(c"RoundWin", VOID),
			],
		)],
		base_fields: vec![key_field(
			c"m_iTeamNum",
			c"TeamNum",
			INTEGER,
			TEAM_NUMBER,
			4,
		)],
		table: None,
	});
	let scope = ();
	let win = RoundWin::new(mock_server(&scope), fake.entity()).unwrap();

	fake.set_int(WIN_REASON, raw::WINREASON_DEFEND_UNTIL_TIME_LIMIT);
	fake.set_int(TEAM_NUMBER, 2);
	fake.set_bool(FORCE_MAP_RESET, true);

	assert_eq!(win.win_reason().unwrap(), WinReason::DefendedUntilTimeLimit);
	assert_eq!(win.team().unwrap(), Some(ScoringTeam::Red));
	assert!(win.resets_map().unwrap());
	assert!(!win.switches_teams().unwrap());

	fake.set_int(TEAM_NUMBER, 1);
	assert_eq!(win.team().unwrap(), None);
	fake.set_int(WIN_REASON, 99);
	assert!(matches!(
		win.win_reason(),
		Err(ObjectiveError::UnknownValue { value: 99, .. })
	));

	take_inputs();
	win.set_team(Some(ScoringTeam::Blue)).unwrap();
	win.set_team(None).unwrap();

	assert_eq!(received(), expected(&[(c"SetTeam", 3), (c"SetTeam", 0)]));

	win.set_win_reason(WinReason::PlayerDestructionPoints)
		.unwrap();
	win.set_resets_map(false).unwrap();
	win.set_switches_teams(true).unwrap();

	let set: Vec<(CString, CString)> = take_key_values()
		.into_iter()
		.map(|(entity, key, value)| {
			assert_eq!(entity, fake.as_ptr());
			(key, value)
		})
		.collect();

	assert_eq!(
		set,
		[
			(c"win_reason".to_owned(), c"12".to_owned()),
			(c"force_map_reset".to_owned(), c"0".to_owned()),
			(c"switch_teams".to_owned(), c"1".to_owned()),
		]
	);
}

#[test]
fn time_remaining_follows_the_game_time() {
	let mut fake = timer();
	let scope = ();
	let timer = RoundTimer::new(mock_server(&scope), fake.entity()).unwrap();

	// A running timer counts down to its end time.
	fake.set_float(END_TIME, 100.0);
	fake.set_time(40.0);
	assert_eq!(timer.time_remaining().unwrap(), 60.0);

	fake.set_time(150.0);
	assert_eq!(timer.time_remaining().unwrap(), 0.0);

	// A paused timer holds its time.
	fake.set_bool(PAUSED, true);
	fake.set_float(TIME_REMAINING, 25.0);
	assert!(timer.is_paused().unwrap());
	assert_eq!(timer.time_remaining().unwrap(), 25.0);

	// A stopwatch watching captures gives the time it counted.
	fake.set_bool(STOPWATCH, true);
	fake.set_float(TOTAL_TIME, 33.0);
	assert_eq!(timer.time_remaining().unwrap(), 25.0);
	fake.set_bool(CAPTURE_WATCH, true);
	assert_eq!(timer.time_remaining().unwrap(), 33.0);
}

/// A networked round timer.
fn timer() -> FakeObjective {
	FakeObjective::new(FakeClass {
		maps: vec![timer_map()],
		base_fields: vec![],
		table: Some((
			c"DT_TeamRoundTimer",
			vec![
				bool_prop(c"m_bTimerPaused", PAUSED),
				float_prop(c"m_flTimeRemaining", TIME_REMAINING),
				float_prop(c"m_flTimerEndTime", END_TIME),
				int_prop(c"m_nTimerMaxLength", MAX_LENGTH),
				bool_prop(c"m_bIsDisabled", DISABLED),
				bool_prop(c"m_bShowInHUD", SHOW_IN_HUD),
				int_prop(c"m_nTimerLength", LENGTH),
				int_prop(c"m_nTimerInitialLength", INITIAL_LENGTH),
				bool_prop(c"m_bAutoCountdown", AUTO_COUNTDOWN),
				int_prop(c"m_nSetupTimeLength", SETUP_LENGTH),
				int_prop(c"m_nState", STATE),
				bool_prop(c"m_bShowTimeRemaining", SHOW_TIME_REMAINING),
				bool_prop(c"m_bStopWatchTimer", STOPWATCH),
				bool_prop(c"m_bInCaptureWatchState", CAPTURE_WATCH),
				float_prop(c"m_flTotalTime", TOTAL_TIME),
			],
		)),
	})
}

/// The data description of `CTeamRoundTimer`, with its inputs.
fn timer_map() -> (&'static CStr, Vec<sys::typedescription_t>) {
	use sys::{
		_fieldtypes_FIELD_INTEGER as INTEGER, _fieldtypes_FIELD_STRING as STRING,
		_fieldtypes_FIELD_VOID as VOID,
	};

	(
		c"CTeamRoundTimer",
		vec![
			input(c"Enable", VOID),
			input(c"Disable", VOID),
			input(c"Pause", VOID),
			input(c"Resume", VOID),
			input(c"Restart", VOID),
			input(c"SetTime", INTEGER),
			input(c"AddTime", INTEGER),
			input(c"AddTeamTime", STRING),
			input(c"ShowInHUD", INTEGER),
			input(c"SetMaxTime", INTEGER),
			input(c"AutoCountdown", INTEGER),
			input(c"SetSetupTime", INTEGER),
		],
	)
}

#[test]
fn timer_outputs_are_named_as_declared() {
	assert_eq!(RoundTimerOutput::Finished.name(), c"OnFinished");
	assert_eq!(RoundTimerOutput::SetupFinished.name(), c"OnSetupFinished");
	assert_eq!(RoundTimerOutput::FiveMinutesLeft.name(), c"On5MinRemain");
	assert_eq!(RoundTimerOutput::OneSecondLeft.name(), c"On1SecRemain");
}

#[test]
fn timers_are_controlled_through_their_inputs() {
	let fake = timer();
	let scope = ();
	let timer = RoundTimer::new(mock_server(&scope), fake.entity()).unwrap();

	take_inputs();
	timer.enable().unwrap();
	timer.disable().unwrap();
	timer.pause().unwrap();
	timer.resume().unwrap();
	timer.restart().unwrap();
	timer.set_time(120).unwrap();
	timer.add_time(-30).unwrap();
	timer.set_shown_in_hud(true).unwrap();
	timer.set_max_length(Some(600)).unwrap();
	timer.set_max_length(Some(-5)).unwrap();
	timer.set_max_length(None).unwrap();
	timer.set_announces_countdown(false).unwrap();
	timer.set_setup_length(45).unwrap();

	assert_eq!(
		received(),
		expected(&[
			(c"Enable", 0),
			(c"Disable", 0),
			(c"Pause", 0),
			(c"Resume", 0),
			(c"Restart", 0),
			(c"SetTime", 120),
			(c"AddTime", -30),
			(c"ShowInHUD", 1),
			(c"SetMaxTime", 600),
			(c"SetMaxTime", 0),
			(c"SetMaxTime", 0),
			(c"AutoCountdown", 0),
			(c"SetSetupTime", 45),
		])
	);

	// The team and the seconds are sent as one string, which is pooled.
	timer.add_team_time(ScoringTeam::Blue, -20).unwrap();

	let inputs = take_inputs();

	assert_eq!(inputs.len(), 1);
	assert_eq!(inputs[0].name.as_c_str(), c"AddTeamTime");
	assert_eq!(inputs[0].target, fake.as_ptr());
	assert_eq!(inputs[0].activator, fake.as_ptr());
	assert_eq!(inputs[0].caller, fake.as_ptr());
	// SAFETY: The pool keeps its strings for the rest of the test.
	assert_eq!(unsafe { CStr::from_ptr(inputs[0].string) }, c"3 -20");
}

#[test]
fn timers_read_their_networked_state() {
	let mut fake = timer();
	let scope = ();
	let timer = RoundTimer::new(mock_server(&scope), fake.entity()).unwrap();

	fake.set_int(STATE, raw::RT_STATE_SETUP);
	fake.set_int(SETUP_LENGTH, 60);
	fake.set_int(LENGTH, 300);
	fake.set_int(INITIAL_LENGTH, 600);
	fake.set_bool(SHOW_IN_HUD, true);
	fake.set_bool(AUTO_COUNTDOWN, true);

	assert_eq!(timer.state().unwrap(), TimerState::Setup);
	assert_eq!(timer.setup_length().unwrap(), 60);
	assert_eq!(timer.length().unwrap(), 300);
	assert_eq!(timer.initial_length().unwrap(), 600);
	assert_eq!(timer.max_length().unwrap(), None);
	assert!(timer.shows_in_hud().unwrap());
	assert!(timer.announces_countdown().unwrap());
	assert!(!timer.is_disabled().unwrap());
	assert!(!timer.shows_time_remaining().unwrap());

	// The HUD's bar is the setup's during setup, then the maximum, or the
	// length without one.
	assert_eq!(timer.hud_length().unwrap(), 60);
	fake.set_int(STATE, raw::RT_STATE_NORMAL);
	assert_eq!(timer.hud_length().unwrap(), 300);
	fake.set_int(MAX_LENGTH, 900);
	assert_eq!(timer.max_length().unwrap(), Some(900));
	assert_eq!(timer.hud_length().unwrap(), 900);

	fake.set_int(STATE, 7);
	assert!(matches!(
		timer.state(),
		Err(ObjectiveError::UnknownValue { value: 7, .. })
	));
}

#[test]
fn win_reasons_round_trip() {
	for (raw, reason) in WinReason::ALL.into_iter().enumerate() {
		let raw = c_int::try_from(raw).unwrap();

		assert_eq!(reason.to_raw(), raw);
		assert_eq!(WinReason::from_raw(raw), Some(reason));
	}

	assert_eq!(WinReason::from_raw(17), None);
	assert_eq!(TimerState::from_raw(2), None);
}
