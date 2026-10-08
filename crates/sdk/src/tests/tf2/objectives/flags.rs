//! Tests of flags and capture zones, through fake entities of their classes.

use super::*;
use crate::test_support::server::mock_server;

use crate::test_support::tf2::objectives::{
	FIELDS_OFFSET, FakeClass, FakeObjective, bool_prop, expected, float_prop, handle_prop, input,
	int_prop, key_field, list_entity, received,
};

const DISABLED: usize = FIELDS_OFFSET;
const FLAG_TYPE: usize = FIELDS_OFFSET + 4;
const GLOW: usize = FIELDS_OFFSET + 2;
const NEUTRAL_TIME: usize = FIELDS_OFFSET + 16;
const POINT_VALUE: usize = FIELDS_OFFSET + 24;
const PREVIOUS_OWNER: usize = FIELDS_OFFSET + 20;
const RESET_TIME: usize = FIELDS_OFFSET + 12;
const RETURN_TIME: usize = FIELDS_OFFSET + 28;
const STATUS: usize = FIELDS_OFFSET + 8;
const TEAM_NUMBER: usize = FIELDS_OFFSET + 32;
const VISIBLE_WHEN_DISABLED: usize = FIELDS_OFFSET + 1;

#[test]
fn carriers_are_found_by_handle() {
	let mut fake = flag();
	let scope = ();
	let flag = CaptureFlag::new(mock_server(&scope), fake.entity()).unwrap();
	let world = fake.world.as_ptr();

	list_entity(world, 1, 3);
	fake.set_int(PREVIOUS_OWNER, 1 | 3 << 16);
	fake.set_int(STATUS, raw::TF_FLAGINFO_STOLEN);

	assert_eq!(flag.carrier().unwrap().unwrap().as_ptr(), world);
	assert_eq!(flag.last_carrier().unwrap().unwrap().as_ptr(), world);

	// A dropped flag remembers its carrier, who no longer carries it.
	fake.set_int(STATUS, raw::TF_FLAGINFO_DROPPED);
	assert!(flag.carrier().unwrap().is_none());
	assert_eq!(flag.last_carrier().unwrap().unwrap().as_ptr(), world);

	// A handle to a removed entity, whose slot has a new serial number, or to
	// no entity refers to none.
	fake.set_int(STATUS, raw::TF_FLAGINFO_STOLEN);
	fake.set_int(PREVIOUS_OWNER, 1 | 2 << 16);
	assert!(flag.carrier().unwrap().is_none());
	fake.set_int(PREVIOUS_OWNER, -1);
	assert!(flag.last_carrier().unwrap().is_none());
}

/// A networked flag.
fn flag() -> FakeObjective {
	use sys::{_fieldtypes_FIELD_INTEGER as INTEGER, _fieldtypes_FIELD_VOID as VOID};

	FakeObjective::new(FakeClass {
		maps: vec![(
			c"CCaptureFlag",
			vec![
				key_field(c"m_nReturnTime", c"ReturnTime", INTEGER, RETURN_TIME, 4),
				input(c"Enable", VOID),
				input(c"Disable", VOID),
				input(c"ForceDrop", VOID),
				input(c"ForceReset", VOID),
				input(c"ForceResetSilent", VOID),
				input(c"SetReturnTime", INTEGER),
				input(c"ShowTimer", INTEGER),
				input(c"ForceGlowDisabled", INTEGER),
			],
		)],
		base_fields: vec![key_field(
			c"m_iTeamNum",
			c"TeamNum",
			INTEGER,
			TEAM_NUMBER,
			4,
		)],
		table: Some((
			c"DT_CaptureFlag",
			vec![
				bool_prop(c"m_bDisabled", DISABLED),
				bool_prop(c"m_bVisibleWhenDisabled", VISIBLE_WHEN_DISABLED),
				bool_prop(c"m_bGlowEnabled", GLOW),
				int_prop(c"m_nType", FLAG_TYPE),
				int_prop(c"m_nFlagStatus", STATUS),
				float_prop(c"m_flResetTime", RESET_TIME),
				float_prop(c"m_flNeutralTime", NEUTRAL_TIME),
				handle_prop(c"m_hPrevOwner", PREVIOUS_OWNER),
				int_prop(c"m_nPointValue", POINT_VALUE),
			],
		)),
	})
}

#[test]
fn flag_values_convert() {
	for status in [FlagStatus::Home, FlagStatus::Stolen, FlagStatus::Dropped] {
		assert_eq!(FlagStatus::from_raw(status.to_raw()), Some(status));
	}

	for flag_type in [
		FlagType::CaptureTheFlag,
		FlagType::AttackDefend,
		FlagType::TerritoryControl,
		FlagType::Invade,
		FlagType::ResourceControl,
		FlagType::RobotDestruction,
		FlagType::PlayerDestruction,
	] {
		assert_eq!(FlagType::from_raw(flag_type.to_raw()), Some(flag_type));
	}

	assert_eq!(FlagStatus::from_raw(3), None);
	assert_eq!(FlagType::from_raw(7), None);
}

#[test]
fn flags_are_controlled_through_their_inputs() {
	let fake = flag();
	let scope = ();
	let flag = CaptureFlag::new(mock_server(&scope), fake.entity()).unwrap();

	received();
	flag.enable().unwrap();
	flag.disable().unwrap();
	flag.force_drop().unwrap();
	flag.reset().unwrap();
	flag.reset_silently().unwrap();
	flag.set_return_delay(45).unwrap();
	flag.show_timer(10).unwrap();
	flag.set_glow_enabled(false).unwrap();
	flag.set_glow_enabled(true).unwrap();

	assert_eq!(
		received(),
		expected(&[
			(c"Enable", 0),
			(c"Disable", 0),
			(c"ForceDrop", 0),
			(c"ForceReset", 0),
			(c"ForceResetSilent", 0),
			(c"SetReturnTime", 45),
			(c"ShowTimer", 10),
			(c"ForceGlowDisabled", 1),
			(c"ForceGlowDisabled", 0),
		])
	);
}

#[test]
fn flags_are_read_from_their_variables() {
	let mut fake = flag();
	let scope = ();
	let flag = CaptureFlag::new(mock_server(&scope), fake.entity()).unwrap();

	assert_eq!(flag.status().unwrap(), FlagStatus::Home);
	assert_eq!(flag.flag_type().unwrap(), FlagType::CaptureTheFlag);
	assert_eq!(flag.team().unwrap(), None);
	assert_eq!(flag.return_time().unwrap(), None);
	assert_eq!(flag.neutral_time().unwrap(), None);
	assert!(flag.last_carrier().unwrap().is_none());

	fake.set_bool(DISABLED, true);
	fake.set_bool(VISIBLE_WHEN_DISABLED, true);
	fake.set_bool(GLOW, true);
	fake.set_int(FLAG_TYPE, raw::TF_FLAGTYPE_INVADE);
	fake.set_int(STATUS, raw::TF_FLAGINFO_DROPPED);
	fake.set_float(RESET_TIME, 75.0);
	fake.set_float(NEUTRAL_TIME, 50.0);
	fake.set_int(POINT_VALUE, 12);
	fake.set_int(RETURN_TIME, 60);
	fake.set_int(TEAM_NUMBER, 3);

	assert!(flag.is_disabled().unwrap());
	assert!(flag.is_visible_when_disabled().unwrap());
	assert!(flag.is_glow_enabled().unwrap());
	assert_eq!(flag.flag_type().unwrap(), FlagType::Invade);
	assert_eq!(flag.status().unwrap(), FlagStatus::Dropped);
	assert_eq!(flag.return_time().unwrap(), Some(75.0));
	assert_eq!(flag.neutral_time().unwrap(), Some(50.0));
	assert_eq!(flag.point_value().unwrap(), 12);
	assert_eq!(flag.return_delay().unwrap(), 60);
	assert_eq!(flag.team().unwrap(), Some(ScoringTeam::Blue));

	fake.set_int(STATUS, 4);
	fake.set_int(FLAG_TYPE, 9);

	assert!(matches!(
		flag.status(),
		Err(ObjectiveError::UnknownValue { value: 4, .. })
	));
	assert!(matches!(
		flag.flag_type(),
		Err(ObjectiveError::UnknownValue { value: 9, .. })
	));
}

#[test]
fn zones_are_read_and_controlled() {
	use sys::{_fieldtypes_FIELD_INTEGER as INTEGER, _fieldtypes_FIELD_VOID as VOID};

	let mut fake = FakeObjective::new(FakeClass {
		maps: vec![
			(
				c"CCaptureZone",
				vec![input(c"Enable", VOID), input(c"Disable", VOID)],
			),
			(c"CBaseTrigger", vec![]),
		],
		base_fields: vec![key_field(
			c"m_iTeamNum",
			c"TeamNum",
			INTEGER,
			TEAM_NUMBER,
			4,
		)],
		table: Some((c"DT_CaptureZone", vec![bool_prop(c"m_bDisabled", DISABLED)])),
	});
	let scope = ();
	let server = mock_server(&scope);
	let zone = CaptureZone::new(server, fake.entity()).unwrap();

	assert!(!zone.is_disabled().unwrap());
	assert_eq!(zone.team().unwrap(), None);

	fake.set_bool(DISABLED, true);
	fake.set_int(TEAM_NUMBER, 2);

	assert!(zone.is_disabled().unwrap());
	assert_eq!(zone.team().unwrap(), Some(ScoringTeam::Red));

	received();
	zone.enable().unwrap();
	zone.disable().unwrap();
	assert_eq!(received(), expected(&[(c"Enable", 0), (c"Disable", 0)]));

	// A zone is no flag.
	assert!(matches!(
		CaptureFlag::new(server, fake.entity()),
		Err(ObjectiveError::WrongClass {
			expected: "item_teamflag"
		})
	));
}
