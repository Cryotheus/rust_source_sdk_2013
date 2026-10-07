//! Tests of pickups, through a fake entity of their class.

use super::*;
use crate::test_support::server::mock_server;

use crate::test_support::tf2::objectives::{
	FIELDS_OFFSET, FakeClass, FakeObjective, expected, input, key_field, received, take_key_values,
};

const AUTO_MATERIALIZE: usize = FIELDS_OFFSET + 1;
const DISABLED: usize = FIELDS_OFFSET;
const TEAM_NUMBER: usize = FIELDS_OFFSET + 4;

/// A health kit, whose class derives from `CTFPowerup`.
fn health_kit() -> FakeObjective {
	use sys::{
		_fieldtypes_FIELD_BOOLEAN as BOOLEAN, _fieldtypes_FIELD_INTEGER as INTEGER,
		_fieldtypes_FIELD_VOID as VOID,
	};

	FakeObjective::new(FakeClass {
		maps: vec![
			(c"CHealthKit", vec![]),
			(
				c"CTFPowerup",
				vec![
					key_field(c"m_bDisabled", c"StartDisabled", BOOLEAN, DISABLED, 1),
					key_field(
						c"m_bAutoMaterialize",
						c"AutoMaterialize",
						BOOLEAN,
						AUTO_MATERIALIZE,
						1,
					),
					input(c"Enable", VOID),
					input(c"Disable", VOID),
					input(c"Toggle", VOID),
				],
			),
		],
		base_fields: vec![key_field(
			c"m_iTeamNum",
			c"TeamNum",
			INTEGER,
			TEAM_NUMBER,
			4,
		)],
		table: None,
	})
}

#[test]
fn pickups_are_read_and_controlled() {
	let mut fake = health_kit();
	let scope = ();
	let pickup = Pickup::new(mock_server(&scope), fake.entity()).unwrap();

	assert!(!pickup.is_disabled().unwrap());
	assert!(!pickup.auto_materializes().unwrap());
	assert_eq!(pickup.team().unwrap(), None);

	fake.set_bool(DISABLED, true);
	fake.set_bool(AUTO_MATERIALIZE, true);
	fake.set_int(TEAM_NUMBER, 3);

	assert!(pickup.is_disabled().unwrap());
	assert!(pickup.auto_materializes().unwrap());
	assert_eq!(pickup.team().unwrap(), Some(ScoringTeam::Blue));

	received();
	pickup.enable().unwrap();
	pickup.disable().unwrap();
	pickup.toggle().unwrap();

	assert_eq!(
		received(),
		expected(&[(c"Enable", 0), (c"Disable", 0), (c"Toggle", 0)])
	);

	pickup.set_auto_materialize(false).unwrap();

	assert_eq!(
		take_key_values(),
		[(
			fake.as_ptr(),
			c"AutoMaterialize".to_owned(),
			c"0".to_owned()
		)]
	);
}
