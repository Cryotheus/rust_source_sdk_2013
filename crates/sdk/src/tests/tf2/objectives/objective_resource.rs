//! Tests of the objective resource, through a fake entity of its class.

use super::*;
use crate::test_support::server::mock_server;

use crate::test_support::tf2::objectives::{
	FIELDS_OFFSET, FakeClass, FakeObjective, array_field, array_props, array3_prop, bool_prop,
	float_prop, int_prop, register_class_name, vector_prop,
};

const BLOCKED: usize = FIELDS_OFFSET + 1216;
const CAN_CAPTURE: usize = FIELDS_OFFSET + 688;
const CAPPING: usize = FIELDS_OFFSET + 1152;
const CAPTURE_LEFT: usize = FIELDS_OFFSET + 1256;
const CAPTURE_TIME: usize = FIELDS_OFFSET + 432;
const GROUP: usize = FIELDS_OFFSET + 792;
const HUD_TIMER: usize = FIELDS_OFFSET;
const IN_MINI_ROUND: usize = FIELDS_OFFSET + 752;
const IN_ZONE: usize = FIELDS_OFFSET + 1184;
const LAZY_CAPTURE_LEFT: usize = FIELDS_OFFSET + 144;
const LOCKED: usize = FIELDS_OFFSET + 824;
const MEMBERS: usize = FIELDS_OFFSET + 896;
const MINI_ROUNDS: usize = FIELDS_OFFSET + 12;
const OWNER: usize = FIELDS_OFFSET + 1224;
const PATH_DISTANCE: usize = FIELDS_OFFSET + 760;
const POINT_COUNT: usize = FIELDS_OFFSET + 8;
const POSITIONS: usize = FIELDS_OFFSET + 16;
const REQUIRED_CAPPERS: usize = FIELDS_OFFSET + 176;
const STOPWATCH_TIMER: usize = FIELDS_OFFSET + 4;
const TIMER_ENDS: usize = FIELDS_OFFSET + 864;
const UNLOCK_TIMES: usize = FIELDS_OFFSET + 832;
const VISIBLE: usize = FIELDS_OFFSET + 112;

#[test]
fn points_are_read_by_index() {
	let mut fake = resource(8);
	let scope = ();
	let resource = ObjectiveResource::new(mock_server(&scope), fake.entity()).unwrap();

	fake.set_int(POINT_COUNT, 3);
	fake.set_bool(MINI_ROUNDS, true);
	fake.set_int(OWNER + 2 * 4, 3);
	fake.set_int(VISIBLE + 2 * 4, 1);
	fake.set_bool(LOCKED + 2, true);
	fake.set_float(UNLOCK_TIMES + 2 * 4, 90.0);
	fake.set_float(TIMER_ENDS + 2 * 4, -1.0);
	fake.set_int(CAPPING + 2 * 4, 2);
	fake.set_int(IN_ZONE + 2 * 4, 2);
	fake.set_bool(BLOCKED + 2, true);
	fake.set_float(LAZY_CAPTURE_LEFT + 2 * 4, 0.75);
	fake.set_float(CAPTURE_LEFT + 2 * 4, 0.5);
	fake.set_bool(IN_MINI_ROUND + 2, true);
	fake.set_int(GROUP + 2 * 4, 1);
	fake.set_float(PATH_DISTANCE + 2 * 4, 0.25);

	for (axis, value) in [1.0, -2.0, 3.5].into_iter().enumerate() {
		fake.set_float(POSITIONS + 2 * 12 + axis * 4, value);
	}

	assert_eq!(resource.point_count().unwrap(), 3);
	assert!(resource.plays_mini_rounds().unwrap());
	assert_eq!(resource.owner(2).unwrap(), Some(ScoringTeam::Blue));
	assert!(resource.is_visible(2).unwrap());
	assert!(resource.is_locked(2).unwrap());
	assert_eq!(resource.unlock_time(2).unwrap(), 90.0);
	assert_eq!(resource.timer_end(2).unwrap(), -1.0);
	assert_eq!(resource.capping_team(2).unwrap(), Some(ScoringTeam::Red));
	assert_eq!(resource.team_in_zone(2).unwrap(), Some(ScoringTeam::Red));
	assert!(resource.is_blocked(2).unwrap());
	assert_eq!(resource.lazy_capture_left(2).unwrap(), 0.75);
	assert_eq!(resource.capture_left(2).unwrap(), 0.5);
	assert!(resource.is_in_mini_round(2).unwrap());
	assert_eq!(resource.group(2).unwrap(), 1);
	assert_eq!(resource.path_distance(2).unwrap(), 0.25);
	assert_eq!(resource.position(2).unwrap(), Vector::new(1.0, -2.0, 3.5));

	// The neighbouring points are untouched.
	assert_eq!(resource.owner(1).unwrap(), None);
	assert!(!resource.is_visible(3).unwrap());
	assert!(!resource.is_blocked(1).unwrap());
	assert_eq!(resource.position(1).unwrap(), Vector::new(0.0, 0.0, 0.0));

	// The game never counts more points than it has room for.
	fake.set_int(POINT_COUNT, 12);
	assert_eq!(resource.point_count().unwrap(), 8);
	fake.set_int(POINT_COUNT, -1);
	assert_eq!(resource.point_count().unwrap(), 0);

	for result in [
		resource.owner(8).map(drop),
		resource.position(8).map(drop),
		resource.capture_left(8).map(drop),
		resource.required_cappers(8, ScoringTeam::Red).map(drop),
	] {
		assert!(matches!(
			result,
			Err(ObjectiveError::PointOutOfRange { index: 8 })
		));
	}
}

#[test]
fn points_are_read_per_team() {
	let mut fake = resource(8);
	let scope = ();
	let resource = ObjectiveResource::new(mock_server(&scope), fake.entity()).unwrap();

	// Each team's entries follow the eight of the team numbered before it.
	fake.set_int(REQUIRED_CAPPERS + (1 + 3 * 8) * 4, 2);
	fake.set_float(CAPTURE_TIME + (1 + 2 * 8) * 4, 6.0);
	fake.set_bool(CAN_CAPTURE + 1 + 3 * 8, true);
	fake.set_int(MEMBERS + (1 + 2 * 8) * 4, 4);

	assert_eq!(resource.required_cappers(1, ScoringTeam::Blue).unwrap(), 2);
	assert_eq!(resource.required_cappers(1, ScoringTeam::Red).unwrap(), 0);
	assert_eq!(resource.capture_time(1, ScoringTeam::Red).unwrap(), 6.0);
	assert!(resource.team_can_capture(1, ScoringTeam::Blue).unwrap());
	assert!(!resource.team_can_capture(1, ScoringTeam::Red).unwrap());
	assert_eq!(resource.cappers_in_zone(1, ScoringTeam::Red).unwrap(), 4);
	assert_eq!(resource.cappers_in_zone(0, ScoringTeam::Red).unwrap(), 0);
}

/// A networked objective resource, whose data description declares the
/// capture progress of each point with `points` elements.
fn resource(points: usize) -> FakeObjective {
	use sys::_fieldtypes_FIELD_FLOAT as FLOAT;

	let mut props = vec![
		int_prop(c"m_iTimerToShowInHUD", HUD_TIMER),
		int_prop(c"m_iStopWatchTimer", STOPWATCH_TIMER),
		int_prop(c"m_iNumControlPoints", POINT_COUNT),
		bool_prop(c"m_bPlayingMiniRounds", MINI_ROUNDS),
	];

	props.extend(array_props(
		c"m_vCPPositions",
		POSITIONS,
		8,
		12,
		vector_prop,
	));
	props.extend([
		array3_prop(c"m_bCPIsVisible", VISIBLE, 8, 4, int_prop),
		array3_prop(c"m_flLazyCapPerc", LAZY_CAPTURE_LEFT, 8, 4, float_prop),
		array3_prop(c"m_iTeamReqCappers", REQUIRED_CAPPERS, 64, 4, int_prop),
		array3_prop(c"m_flTeamCapTime", CAPTURE_TIME, 64, 4, float_prop),
		array3_prop(c"m_bTeamCanCap", CAN_CAPTURE, 64, 1, bool_prop),
		array3_prop(c"m_bInMiniRound", IN_MINI_ROUND, 8, 1, bool_prop),
		array3_prop(c"m_flPathDistance", PATH_DISTANCE, 8, 4, float_prop),
		array3_prop(c"m_iCPGroup", GROUP, 8, 4, int_prop),
		array3_prop(c"m_bCPLocked", LOCKED, 8, 1, bool_prop),
		array3_prop(c"m_flUnlockTimes", UNLOCK_TIMES, 8, 4, float_prop),
		array3_prop(c"m_flCPTimerTimes", TIMER_ENDS, 8, 4, float_prop),
		array3_prop(c"m_iNumTeamMembers", MEMBERS, 64, 4, int_prop),
		array3_prop(c"m_iCappingTeam", CAPPING, 8, 4, int_prop),
		array3_prop(c"m_iTeamInZone", IN_ZONE, 8, 4, int_prop),
		array3_prop(c"m_bBlocked", BLOCKED, 8, 1, bool_prop),
		array3_prop(c"m_iOwner", OWNER, 8, 4, int_prop),
	]);

	FakeObjective::new(FakeClass {
		maps: vec![
			(c"CTFObjectiveResource", vec![]),
			(
				c"CBaseTeamObjectiveResource",
				vec![array_field(
					c"m_flCapPercentages",
					FLOAT,
					CAPTURE_LEFT,
					points,
					4,
				)],
			),
			// Mock entities share one class, so the timers the resource refers
			// to are found as entities of the same chain.
			(c"CTeamRoundTimer", vec![]),
		],
		base_fields: vec![],
		table: Some((c"DT_TFObjectiveResource", props)),
	})
}

#[test]
fn resources_are_found_with_their_timers() {
	let mut fake = resource(8);
	let scope = ();
	let server = mock_server(&scope);

	assert!(ObjectiveResource::find(server).unwrap().is_none());
	register_class_name(fake.world.as_ptr(), c"tf_objective_resource");
	register_class_name(fake.as_ptr(), c"tf_objective_resource");

	// Entities marked for deletion are skipped.
	fake.world.set_eflags(sdk_raw::entities::EFL_KILLME);
	let resource = ObjectiveResource::find(server).unwrap().unwrap();

	assert_eq!(resource.entity().as_ptr(), fake.as_ptr());

	// Index 0, the world, means no timer, and the fake's own index, 5, holds a
	// round timer.
	assert!(resource.hud_timer().unwrap().is_none());
	fake.set_int(HUD_TIMER, 5);
	fake.set_int(STOPWATCH_TIMER, 7);

	let timer = resource.hud_timer().unwrap().unwrap();

	assert_eq!(timer.entity().as_ptr(), fake.as_ptr());
	assert!(resource.stopwatch_timer().unwrap().is_none());
}

#[test]
fn unexpected_layouts_are_refused() {
	let fake = resource(4);
	let scope = ();
	let resource = ObjectiveResource::new(mock_server(&scope), fake.entity()).unwrap();

	assert!(resource.capture_left(3).is_ok());
	assert!(matches!(
		resource.capture_left(4),
		Err(ObjectiveError::UnsupportedLayout {
			field,
			..
		}) if field == c"m_flCapPercentages"
	));
}
