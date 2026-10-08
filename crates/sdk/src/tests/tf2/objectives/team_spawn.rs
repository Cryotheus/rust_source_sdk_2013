//! Tests of team spawn points, through a fake entity of their class.

use super::*;
use crate::test_support::server::mock_server;

use crate::test_support::tf2::objectives::{
	FIELDS_OFFSET, FakeClass, FakeObjective, expected, input, key_field, received,
};

const DISABLED: usize = FIELDS_OFFSET;
const MODE: usize = FIELDS_OFFSET + 4;
const TEAM_NUMBER: usize = FIELDS_OFFSET + 8;

#[test]
fn modes_convert() {
	for mode in [TeamSpawnMode::Normal, TeamSpawnMode::Triggered] {
		assert_eq!(TeamSpawnMode::from_raw(mode.to_raw()), Some(mode));
	}

	assert_eq!(TeamSpawnMode::from_raw(2), None);
}

#[test]
fn spawns_are_read_and_controlled() {
	use sys::{
		_fieldtypes_FIELD_BOOLEAN as BOOLEAN, _fieldtypes_FIELD_INTEGER as INTEGER,
		_fieldtypes_FIELD_VOID as VOID,
	};

	let mut fake = FakeObjective::new(FakeClass {
		maps: vec![(
			c"CTFTeamSpawn",
			vec![
				key_field(c"m_bDisabled", c"StartDisabled", BOOLEAN, DISABLED, 1),
				key_field(c"m_nSpawnMode", c"SpawnMode", INTEGER, MODE, 4),
				input(c"Enable", VOID),
				input(c"Disable", VOID),
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
	let spawn = TeamSpawn::new(mock_server(&scope), fake.entity()).unwrap();

	assert!(!spawn.is_disabled().unwrap());
	assert_eq!(spawn.mode().unwrap(), TeamSpawnMode::Normal);
	assert_eq!(spawn.team().unwrap(), None);

	fake.set_bool(DISABLED, true);
	fake.set_int(MODE, 1);
	fake.set_int(TEAM_NUMBER, 2);

	assert!(spawn.is_disabled().unwrap());
	assert_eq!(spawn.mode().unwrap(), TeamSpawnMode::Triggered);
	assert_eq!(spawn.team().unwrap(), Some(ScoringTeam::Red));

	fake.set_int(MODE, 5);
	assert!(matches!(
		spawn.mode(),
		Err(ObjectiveError::UnknownValue { value: 5, .. })
	));

	received();
	spawn.enable().unwrap();
	spawn.disable().unwrap();
	assert_eq!(received(), expected(&[(c"Enable", 0), (c"Disable", 0)]));
}
