//! Tests of respawn rooms, their visualizers, resupply zones and zones
//! without buildings, through fake entities of their classes.

use super::*;
use crate::test_support::entities::take_inputs;
use crate::test_support::server::mock_server;

use crate::test_support::tf2::objectives::{
	FIELDS_OFFSET, FakeClass, FakeObjective, expected, input, key_field, received, register_name,
	take_key_values,
};

const ALLOW_DISPENSER: usize = FIELDS_OFFSET + 2;
const ALLOW_SENTRY: usize = FIELDS_OFFSET + 1;
const ALLOW_TELEPORTERS: usize = FIELDS_OFFSET + 3;
const DESTROY_BUILDINGS: usize = FIELDS_OFFSET + 4;
const DISABLED: usize = FIELDS_OFFSET;
const ROOM_NAME: usize = FIELDS_OFFSET + 16;
const SOLID: usize = FIELDS_OFFSET + 5;
const TEAM_NUMBER: usize = FIELDS_OFFSET + 8;

#[test]
fn no_build_zones_are_read_and_controlled() {
	use sys::{_fieldtypes_FIELD_BOOLEAN as BOOLEAN, _fieldtypes_FIELD_VOID as VOID};

	let mut fake = FakeObjective::new(FakeClass {
		maps: vec![
			(
				c"CFuncNoBuild",
				vec![
					input(c"SetActive", VOID),
					input(c"SetInactive", VOID),
					input(c"ToggleActive", VOID),
					key_field(c"m_bAllowSentry", c"AllowSentry", BOOLEAN, ALLOW_SENTRY, 1),
					key_field(
						c"m_bAllowDispenser",
						c"AllowDispenser",
						BOOLEAN,
						ALLOW_DISPENSER,
						1,
					),
					key_field(
						c"m_bAllowTeleporters",
						c"AllowTeleporters",
						BOOLEAN,
						ALLOW_TELEPORTERS,
						1,
					),
					key_field(
						c"m_bDestroyBuildingsOnActive",
						c"DestroyBuildings",
						BOOLEAN,
						DESTROY_BUILDINGS,
						1,
					),
				],
			),
			(
				c"CBaseTrigger",
				vec![key_field(
					c"m_bDisabled",
					c"StartDisabled",
					BOOLEAN,
					DISABLED,
					1,
				)],
			),
		],
		base_fields: vec![team_field()],
		table: None,
	});
	let scope = ();
	let zone = NoBuildZone::new(mock_server(&scope), fake.entity()).unwrap();

	assert!(zone.is_active().unwrap());
	assert!(!zone.destroys_buildings().unwrap());
	assert_eq!(zone.team().unwrap(), None);

	for kind in ObjectKind::ALL {
		assert!(!zone.allows(kind).unwrap());
	}

	fake.set_bool(DISABLED, true);
	fake.set_bool(ALLOW_DISPENSER, true);
	fake.set_bool(DESTROY_BUILDINGS, true);
	fake.set_int(TEAM_NUMBER, 3);

	assert!(!zone.is_active().unwrap());
	assert!(zone.allows(ObjectKind::Dispenser).unwrap());
	assert!(!zone.allows(ObjectKind::Sentry).unwrap());
	assert!(!zone.allows(ObjectKind::Teleporter).unwrap());
	assert!(zone.destroys_buildings().unwrap());
	assert_eq!(zone.team().unwrap(), Some(ScoringTeam::Blue));

	fake.set_bool(ALLOW_SENTRY, true);
	fake.set_bool(ALLOW_TELEPORTERS, true);
	assert!(zone.allows(ObjectKind::Sentry).unwrap());
	assert!(zone.allows(ObjectKind::Teleporter).unwrap());

	received();
	zone.set_active(true).unwrap();
	zone.set_active(false).unwrap();
	zone.toggle_active().unwrap();

	assert_eq!(
		received(),
		expected(&[(c"SetActive", 0), (c"SetInactive", 0), (c"ToggleActive", 0)])
	);

	zone.set_allows(ObjectKind::Sentry, true).unwrap();
	zone.set_allows(ObjectKind::Teleporter, false).unwrap();

	let keys: Vec<_> = take_key_values()
		.into_iter()
		.map(|(_, key, value)| (key, value))
		.collect();

	assert_eq!(
		keys,
		[
			(c"AllowSentry".to_owned(), c"1".to_owned()),
			(c"AllowTeleporters".to_owned(), c"0".to_owned()),
		]
	);
}

#[test]
fn regenerate_zones_are_controlled() {
	use sys::_fieldtypes_FIELD_VOID as VOID;

	let mut fake = FakeObjective::new(FakeClass {
		maps: vec![
			(c"CRegenerateZone", vec![]),
			(
				c"CBaseTrigger",
				vec![
					input(c"Enable", VOID),
					input(c"Disable", VOID),
					input(c"Toggle", VOID),
				],
			),
		],
		base_fields: vec![team_field()],
		table: None,
	});
	let scope = ();
	let zone = RegenerateZone::new(mock_server(&scope), fake.entity()).unwrap();

	fake.set_int(TEAM_NUMBER, 2);
	assert_eq!(zone.team().unwrap(), Some(ScoringTeam::Red));

	received();
	zone.enable().unwrap();
	zone.disable().unwrap();
	zone.toggle().unwrap();

	assert_eq!(
		received(),
		expected(&[(c"Enable", 0), (c"Disable", 0), (c"Toggle", 0)])
	);
}

/// A respawn room whose class chain includes a visualizer's, as mock
/// entities share one class, so that one entity stands in for both.
fn respawn_room() -> FakeObjective {
	use sys::{
		_fieldtypes_FIELD_BOOLEAN as BOOLEAN, _fieldtypes_FIELD_STRING as STRING,
		_fieldtypes_FIELD_VOID as VOID,
	};

	FakeObjective::new(FakeClass {
		maps: vec![
			(
				c"CFuncRespawnRoomVisualizer",
				vec![
					key_field(
						c"m_iszRespawnRoomName",
						c"respawnroomname",
						STRING,
						ROOM_NAME,
						8,
					),
					key_field(c"m_bSolid", c"solid_to_enemies", BOOLEAN, SOLID, 1),
					input(c"SetSolid", BOOLEAN),
				],
			),
			(
				c"CFuncRespawnRoom",
				vec![
					input(c"SetActive", VOID),
					input(c"SetInactive", VOID),
					input(c"ToggleActive", VOID),
				],
			),
		],
		base_fields: vec![team_field()],
		table: None,
	})
}

#[test]
fn respawn_rooms_are_controlled() {
	let mut fake = respawn_room();
	let scope = ();
	let room = RespawnRoom::new(mock_server(&scope), fake.entity()).unwrap();

	assert_eq!(room.team().unwrap(), None);
	fake.set_int(TEAM_NUMBER, 3);
	assert_eq!(room.team().unwrap(), Some(ScoringTeam::Blue));

	received();
	room.set_active(false).unwrap();
	room.set_active(true).unwrap();
	room.toggle_active().unwrap();

	assert_eq!(
		received(),
		expected(&[(c"SetInactive", 0), (c"SetActive", 0), (c"ToggleActive", 0)])
	);
}

/// The `m_iTeamNum` of `CBaseEntity`'s data description.
fn team_field() -> sys::typedescription_t {
	key_field(
		c"m_iTeamNum",
		c"TeamNum",
		sys::_fieldtypes_FIELD_INTEGER,
		TEAM_NUMBER,
		4,
	)
}

#[test]
fn visualizers_find_their_room() {
	let mut fake = respawn_room();
	let scope = ();
	let visualizer = RespawnRoomVisualizer::new(mock_server(&scope), fake.entity()).unwrap();

	fake.set_string(ROOM_NAME, c"spawn_red");
	fake.set_bool(SOLID, true);
	fake.set_int(TEAM_NUMBER, 2);

	assert_eq!(visualizer.room_name().unwrap().as_c_str(), c"spawn_red");
	assert!(visualizer.is_solid().unwrap());
	assert_eq!(visualizer.team().unwrap(), Some(ScoringTeam::Red));

	assert!(visualizer.room().unwrap().is_none());
	register_name(fake.as_ptr(), c"Spawn_Red");
	assert_eq!(
		visualizer.room().unwrap().unwrap().entity().as_ptr(),
		fake.as_ptr()
	);

	take_inputs();
	visualizer.set_solid(false).unwrap();

	let inputs = take_inputs();

	assert_eq!(inputs.len(), 1);
	assert_eq!(inputs[0].name.as_c_str(), c"SetSolid");
	assert_eq!(inputs[0].field_type, sys::_fieldtypes_FIELD_BOOLEAN);
	assert_eq!(inputs[0].payload[0], 0);
}
