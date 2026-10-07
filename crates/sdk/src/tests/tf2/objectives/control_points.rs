//! Tests of control points, their master and their mini-rounds, through fake
//! entities of their classes.

use super::*;
use crate::test_support::entities::{set_networking, take_inputs};
use crate::test_support::server::mock_server;
use crate::test_support::tf2::game_rules::{ROUND_STATE, World, round_rules_proxy};
use crate::tf2::objectives::RoundWin;
use crate::tf2::round_end::RoundEndError;
use sdk_raw::tf2::game_rules::{GR_STATE_RND_RUNNING, GR_STATE_TEAM_WIN};

use crate::test_support::tf2::objectives::{
	FIELDS_OFFSET, FakeClass, FakeObjective, expected, input, key_field, received,
	register_class_name, take_key_values,
};

const CAP_LAYOUT: usize = FIELDS_OFFSET + 8;
const DEFAULT_OWNER: usize = FIELDS_OFFSET + 16;
const DISABLED: usize = FIELDS_OFFSET;
const GROUP: usize = FIELDS_OFFSET + 20;
const INVALID_CAP_WINNER: usize = FIELDS_OFFSET + 24;
const LOCKED: usize = FIELDS_OFFSET + 1;
const NAMES: usize = FIELDS_OFFSET + 32;
const POINT_INDEX: usize = FIELDS_OFFSET + 28;
const PRINT_NAME: usize = FIELDS_OFFSET + 40;
const PRIORITY: usize = FIELDS_OFFSET + 48;
const SWITCH_TEAMS: usize = FIELDS_OFFSET + 2;
const TEAM_NUMBER: usize = FIELDS_OFFSET + 52;

#[test]
fn capture_wins_round_trip() {
	for (raw, wins) in [
		(0, CaptureWins::Either),
		(1, CaptureWins::Neither),
		(2, CaptureWins::Except(ScoringTeam::Red)),
		(3, CaptureWins::Except(ScoringTeam::Blue)),
	] {
		assert_eq!(wins.to_raw(), raw);
		assert_eq!(CaptureWins::from_raw(raw), Some(wins));
	}

	assert_eq!(CaptureWins::from_raw(4), None);
	assert_eq!(CaptureWins::from_raw(-1), None);
}

#[test]
fn control_points_read_their_state_and_change_hands() {
	use sys::{
		_fieldtypes_FIELD_BOOLEAN as BOOLEAN, _fieldtypes_FIELD_INTEGER as INTEGER,
		_fieldtypes_FIELD_STRING as STRING, _fieldtypes_FIELD_VOID as VOID,
	};

	let mut fake = FakeObjective::new(FakeClass {
		maps: vec![(
			c"CTeamControlPoint",
			vec![
				key_field(c"m_iszPrintName", c"point_printname", STRING, PRINT_NAME, 8),
				key_field(c"m_iCPGroup", c"point_group", INTEGER, GROUP, 4),
				key_field(
					c"m_iDefaultOwner",
					c"point_default_owner",
					INTEGER,
					DEFAULT_OWNER,
					4,
				),
				key_field(c"m_iPointIndex", c"point_index", INTEGER, POINT_INDEX, 4),
				key_field(c"m_bLocked", c"point_start_locked", BOOLEAN, LOCKED, 1),
				input(c"SetOwner", INTEGER),
				input(c"ShowModel", VOID),
				input(c"HideModel", VOID),
				input(c"SetLocked", INTEGER),
				input(c"SetUnlockTime", INTEGER),
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
	let point = ControlPoint::new(mock_server(&scope), fake.entity()).unwrap();

	fake.set_string(PRINT_NAME, c"Gate A");
	fake.set_int(GROUP, 1);
	fake.set_int(DEFAULT_OWNER, 2);
	fake.set_int(POINT_INDEX, 3);
	fake.set_bool(LOCKED, true);
	fake.set_int(TEAM_NUMBER, 3);

	assert_eq!(point.print_name().unwrap().as_c_str(), c"Gate A");
	assert_eq!(point.group().unwrap(), 1);
	assert_eq!(point.default_owner().unwrap(), Some(ScoringTeam::Red));
	assert_eq!(point.index().unwrap(), 3);
	assert!(point.is_locked().unwrap());
	assert_eq!(point.owner().unwrap(), Some(ScoringTeam::Blue));

	fake.set_int(TEAM_NUMBER, 0);
	assert_eq!(point.owner().unwrap(), None);

	take_inputs();
	point.set_owner(Some(ScoringTeam::Red)).unwrap();
	point.set_owner(None).unwrap();
	point.set_locked(true).unwrap();
	point.set_locked(false).unwrap();
	point.set_unlock_time(30).unwrap();
	point.hide_model().unwrap();
	point.show_model().unwrap();

	assert_eq!(
		received(),
		expected(&[
			(c"SetOwner", 2),
			(c"SetOwner", 0),
			(c"SetLocked", 1),
			(c"SetLocked", 0),
			(c"SetUnlockTime", 30),
			(c"HideModel", 0),
			(c"ShowModel", 0),
		])
	);

	// A capture comes from the capper, whom the point credits if a player.
	point
		.capture(ScoringTeam::Blue, fake.world_entity())
		.unwrap();

	let inputs = take_inputs();

	assert_eq!(inputs.len(), 1);
	assert_eq!(inputs[0].name.as_c_str(), c"SetOwner");
	assert_eq!(inputs[0].payload[..4], 3_i32.to_ne_bytes());
	assert_eq!(inputs[0].target, fake.as_ptr());
	assert_eq!(inputs[0].activator, fake.world.as_ptr());
	assert_eq!(inputs[0].caller, fake.world.as_ptr());
}

/// The key values set on `fake` since the last call.
fn key_values(fake: &FakeObjective) -> Vec<(CString, CString)> {
	take_key_values()
		.into_iter()
		.map(|(entity, key, value)| {
			assert_eq!(entity, fake.as_ptr());
			(key, value)
		})
		.collect()
}

#[test]
fn masters_read_their_rules_and_end_rounds() {
	use sys::{
		_fieldtypes_FIELD_BOOLEAN as BOOLEAN, _fieldtypes_FIELD_FLOAT as FLOAT,
		_fieldtypes_FIELD_INTEGER as INTEGER, _fieldtypes_FIELD_STRING as STRING,
		_fieldtypes_FIELD_VOID as VOID,
	};

	let mut fake = FakeObjective::new(FakeClass {
		maps: vec![(
			c"CTeamControlPointMaster",
			vec![
				key_field(c"m_bDisabled", c"StartDisabled", BOOLEAN, DISABLED, 1),
				key_field(c"m_iszCapLayoutInHUD", c"caplayout", STRING, CAP_LAYOUT, 8),
				key_field(
					c"m_iInvalidCapWinner",
					c"cpm_restrict_team_cap_win",
					INTEGER,
					INVALID_CAP_WINNER,
					4,
				),
				key_field(
					c"m_bSwitchTeamsOnWin",
					c"switch_teams",
					BOOLEAN,
					SWITCH_TEAMS,
					1,
				),
				input(c"Enable", VOID),
				input(c"Disable", VOID),
				input(c"SetWinner", INTEGER),
				input(c"SetWinnerAndForceCaps", INTEGER),
				input(c"SetCapLayout", STRING),
				input(c"SetCapLayoutCustomPositionX", FLOAT),
				input(c"SetCapLayoutCustomPositionY", FLOAT),
			],
		)],
		base_fields: vec![],
		table: None,
	});
	let scope = ();
	let server = mock_server(&scope);

	assert!(ControlPointMaster::find(server).unwrap().is_none());
	register_class_name(fake.as_ptr(), c"team_control_point_master");

	let master = ControlPointMaster::find(server).unwrap().unwrap();

	fake.set_bool(DISABLED, true);
	fake.set_string(CAP_LAYOUT, c"2, 0 1");
	fake.set_int(INVALID_CAP_WINNER, 2);
	fake.set_bool(SWITCH_TEAMS, true);

	assert!(master.is_disabled().unwrap());
	assert_eq!(master.cap_layout().unwrap().as_c_str(), c"2, 0 1");
	assert_eq!(
		master.capture_wins().unwrap(),
		CaptureWins::Except(ScoringTeam::Red)
	);
	assert!(master.switches_teams().unwrap());

	fake.set_int(INVALID_CAP_WINNER, 9);
	assert!(matches!(
		master.capture_wins(),
		Err(ObjectiveError::UnknownValue { value: 9, .. })
	));

	take_inputs();
	master.enable().unwrap();
	master.disable().unwrap();

	assert_eq!(received(), expected(&[(c"Enable", 0), (c"Disable", 0)]));

	master.set_cap_layout(c"0 1 2").unwrap();
	master.set_cap_layout_position(0.5, -1.0).unwrap();

	let inputs = take_inputs();
	let floats: Vec<(&CStr, f32)> = inputs[1..]
		.iter()
		.map(|input| {
			assert_eq!(input.field_type, sys::_fieldtypes_FIELD_FLOAT);
			(
				input.name.as_c_str(),
				f32::from_ne_bytes(input.payload[..4].try_into().unwrap()),
			)
		})
		.collect();

	assert_eq!(inputs[0].name.as_c_str(), c"SetCapLayout");
	// SAFETY: The pool keeps its strings for the rest of the test.
	assert_eq!(unsafe { CStr::from_ptr(inputs[0].string) }, c"0 1 2");
	assert_eq!(
		floats,
		[
			(c"SetCapLayoutCustomPositionX", 0.5),
			(c"SetCapLayoutCustomPositionY", -1.0),
		]
	);

	master.set_capture_wins(CaptureWins::Neither).unwrap();
	master.set_switches_teams(false).unwrap();

	assert_eq!(
		key_values(&fake),
		[
			(c"cpm_restrict_team_cap_win".to_owned(), c"1".to_owned()),
			(c"switch_teams".to_owned(), c"0".to_owned()),
		]
	);
}

#[test]
fn mini_rounds_read_their_points_and_rules() {
	use sys::{
		_fieldtypes_FIELD_BOOLEAN as BOOLEAN, _fieldtypes_FIELD_INTEGER as INTEGER,
		_fieldtypes_FIELD_STRING as STRING, _fieldtypes_FIELD_VOID as VOID,
	};

	let mut fake = FakeObjective::new(FakeClass {
		maps: vec![(
			c"CTeamControlPointRound",
			vec![
				key_field(c"m_bDisabled", c"StartDisabled", BOOLEAN, DISABLED, 1),
				key_field(c"m_iszCPNames", c"cpr_cp_names", STRING, NAMES, 8),
				key_field(c"m_nPriority", c"cpr_priority", INTEGER, PRIORITY, 4),
				key_field(
					c"m_iInvalidCapWinner",
					c"cpr_restrict_team_cap_win",
					INTEGER,
					INVALID_CAP_WINNER,
					4,
				),
				key_field(c"m_iszPrintName", c"cpr_printname", STRING, PRINT_NAME, 8),
				input(c"Enable", VOID),
				input(c"Disable", VOID),
			],
		)],
		base_fields: vec![],
		table: None,
	});
	let scope = ();
	let round = ControlPointRound::new(mock_server(&scope), fake.entity()).unwrap();

	fake.set_string(NAMES, c"cp_a cp_b");
	fake.set_string(PRINT_NAME, c"Stage 1");
	fake.set_int(PRIORITY, 2);
	fake.set_int(INVALID_CAP_WINNER, 1);

	assert!(!round.is_disabled().unwrap());
	assert_eq!(round.point_names().unwrap().as_c_str(), c"cp_a cp_b");
	assert_eq!(round.print_name().unwrap().as_c_str(), c"Stage 1");
	assert_eq!(round.priority().unwrap(), 2);
	assert_eq!(round.capture_wins().unwrap(), CaptureWins::Neither);

	take_inputs();
	round.disable().unwrap();
	round.enable().unwrap();
	assert_eq!(received(), expected(&[(c"Disable", 0), (c"Enable", 0)]));

	round
		.set_capture_wins(CaptureWins::Except(ScoringTeam::Blue))
		.unwrap();
	assert_eq!(
		key_values(&fake),
		[(c"cpr_restrict_team_cap_win".to_owned(), c"3".to_owned())]
	);

	// A point is not a master or a mini-round.
	assert!(matches!(
		ControlPoint::new(mock_server(&scope), fake.entity()),
		Err(ObjectiveError::WrongClass {
			expected: "team_control_point"
		})
	));
}

#[test]
fn rounds_end_only_while_no_team_has_won() {
	use sys::{_fieldtypes_FIELD_INTEGER as INTEGER, _fieldtypes_FIELD_VOID as VOID};

	// The first interfaces exported are the ones found, so the game rules'
	// come first, and a new mock entity forgets their networking, so it is
	// set again. Mock entities share one class, so one entity stands in for
	// both wrappers.
	let world = World::new(Some(round_rules_proxy));
	let fake = FakeObjective::new(FakeClass {
		maps: vec![
			(
				c"CTeamControlPointMaster",
				vec![
					input(c"SetWinner", INTEGER),
					input(c"SetWinnerAndForceCaps", INTEGER),
				],
			),
			(c"CTeamplayRoundWin", vec![input(c"RoundWin", VOID)]),
		],
		base_fields: vec![],
		table: None,
	});
	let scope = ();
	let server = mock_server(&scope);

	set_networking(world.class, world.edict);

	let master = ControlPointMaster::new(server, fake.entity()).unwrap();
	let win = RoundWin::new(server, fake.entity()).unwrap();

	world.put_int(true, ROUND_STATE, GR_STATE_RND_RUNNING);
	received();
	master.set_winner(Some(ScoringTeam::Blue)).unwrap();
	master.set_winner(None).unwrap();
	master
		.set_winner_and_force_caps(Some(ScoringTeam::Red))
		.unwrap();
	win.win().unwrap();

	assert_eq!(
		received(),
		expected(&[
			(c"SetWinner", 3),
			(c"SetWinner", 0),
			(c"SetWinnerAndForceCaps", 2),
			(c"RoundWin", 0),
		])
	);

	// Once a team has won, TF2 would crit boost the winners again.
	world.put_int(true, ROUND_STATE, GR_STATE_TEAM_WIN);

	for result in [
		master.set_winner(Some(ScoringTeam::Red)),
		master.set_winner_and_force_caps(None),
		win.win(),
	] {
		assert!(matches!(
			result,
			Err(ObjectiveError::RoundEnd(RoundEndError::AlreadyWon))
		));
	}

	assert!(received().is_empty());
}
