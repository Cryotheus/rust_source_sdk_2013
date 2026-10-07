//! Tests of train watchers, through a fake entity of their class.

use super::*;
use crate::test_support::entities::take_inputs;
use crate::test_support::server::mock_server;

use crate::test_support::tf2::objectives::{
	FIELDS_OFFSET, FakeClass, FakeObjective, expected, float_prop, input, int_prop, key_field,
	received, register_name,
};

const CAN_RECEDE: usize = FIELDS_OFFSET + 1;
const CAPPERS: usize = FIELDS_OFFSET + 16;
const DISABLED: usize = FIELDS_OFFSET;
const HANDLES_MOVEMENT: usize = FIELDS_OFFSET + 2;
const PROGRESS: usize = FIELDS_OFFSET + 4;
const RECEDE_DELAY: usize = FIELDS_OFFSET + 20;
const RECEDE_TIME: usize = FIELDS_OFFSET + 12;
const SPEED_LEVEL: usize = FIELDS_OFFSET + 8;
const SPEED_MODIFIER: usize = FIELDS_OFFSET + 24;
const TRAIN: usize = FIELDS_OFFSET + 32;

/// A networked train watcher, whose class chain includes a capture area's, so
/// that mock entities can stand in for the area that counts its cappers.
fn watcher() -> FakeObjective {
	use sys::{
		_fieldtypes_FIELD_BOOLEAN as BOOLEAN, _fieldtypes_FIELD_FLOAT as FLOAT,
		_fieldtypes_FIELD_INTEGER as INTEGER, _fieldtypes_FIELD_STRING as STRING,
		_fieldtypes_FIELD_VOID as VOID,
	};

	FakeObjective::new(FakeClass {
		maps: vec![
			(
				c"CTeamTrainWatcher",
				vec![
					key_field(c"m_iszTrain", c"train", STRING, TRAIN, 8),
					key_field(
						c"m_bTrainCanRecede",
						c"train_can_recede",
						BOOLEAN,
						CAN_RECEDE,
						1,
					),
					key_field(
						c"m_bHandleTrainMovement",
						c"handle_train_movement",
						BOOLEAN,
						HANDLES_MOVEMENT,
						1,
					),
					key_field(c"m_bDisabled", c"StartDisabled", BOOLEAN, DISABLED, 1),
					key_field(
						c"m_flSpeedForwardModifier",
						c"speed_forward_modifier",
						FLOAT,
						SPEED_MODIFIER,
						4,
					),
					key_field(
						c"m_nTrainRecedeTime",
						c"train_recede_time",
						INTEGER,
						RECEDE_DELAY,
						4,
					),
					input(c"SetNumTrainCappers", INTEGER),
					input(c"Enable", VOID),
					input(c"Disable", VOID),
					input(c"SetSpeedForwardModifier", FLOAT),
					input(c"SetTrainRecedeTime", INTEGER),
					input(c"SetTrainCanRecede", BOOLEAN),
					input(c"SetTrainRecedeTimeAndUpdate", INTEGER),
				],
			),
			(c"CTriggerAreaCapture", vec![]),
		],
		base_fields: vec![],
		table: Some((
			c"DT_TeamTrainWatcher",
			vec![
				float_prop(c"m_flTotalProgress", PROGRESS),
				int_prop(c"m_iTrainSpeedLevel", SPEED_LEVEL),
				float_prop(c"m_flRecedeTime", RECEDE_TIME),
				int_prop(c"m_nNumCappers", CAPPERS),
			],
		)),
	})
}

#[test]
fn watchers_are_controlled_through_their_inputs() {
	let fake = watcher();
	let scope = ();
	let watcher = TrainWatcher::new(mock_server(&scope), fake.entity()).unwrap();

	received();
	watcher.enable().unwrap();
	watcher.disable().unwrap();
	watcher.set_recede_delay(20).unwrap();
	watcher.set_recede_delay_and_restart(10).unwrap();

	assert_eq!(
		received(),
		expected(&[
			(c"Enable", 0),
			(c"Disable", 0),
			(c"SetTrainRecedeTime", 20),
			(c"SetTrainRecedeTimeAndUpdate", 10),
		])
	);

	watcher.set_can_recede(true).unwrap();
	watcher.set_speed_forward_modifier(0.5).unwrap();

	let inputs = take_inputs();

	assert_eq!(inputs[0].field_type, sys::_fieldtypes_FIELD_BOOLEAN);
	assert_eq!(inputs[0].payload[0], 1);
	assert_eq!(inputs[1].field_type, sys::_fieldtypes_FIELD_FLOAT);
	assert_eq!(inputs[1].payload[..4], 0.5_f32.to_ne_bytes());

	// The count comes from the capture area, whose blocking the watcher reads,
	// or from the watcher itself. Mock entities share one class, so the world
	// stands in for the area.
	let area = CaptureArea::new(mock_server(&scope), fake.world_entity()).unwrap();

	watcher.set_cappers(2, Some(area)).unwrap();
	watcher.set_cappers(-1, None).unwrap();

	let inputs = take_inputs();
	let cappers: Vec<_> = inputs
		.iter()
		.map(|input| {
			let value = c_int::from_ne_bytes(input.payload[..4].try_into().unwrap());

			(input.name.as_c_str(), value, input.caller)
		})
		.collect();

	assert_eq!(
		cappers,
		[
			(c"SetNumTrainCappers", 2, fake.world_entity().as_ptr()),
			(c"SetNumTrainCappers", -1, fake.as_ptr()),
		]
	);
}

#[test]
fn watchers_are_read_from_their_variables() {
	let mut fake = watcher();
	let scope = ();
	let watcher = TrainWatcher::new(mock_server(&scope), fake.entity()).unwrap();

	assert_eq!(watcher.recede_time().unwrap(), None);
	assert_eq!(watcher.recede_delay().unwrap(), None);

	fake.set_bool(DISABLED, true);
	fake.set_bool(CAN_RECEDE, true);
	fake.set_bool(HANDLES_MOVEMENT, true);
	fake.set_float(PROGRESS, 0.4);
	fake.set_int(SPEED_LEVEL, -1);
	fake.set_float(RECEDE_TIME, 33.0);
	fake.set_int(CAPPERS, 3);
	fake.set_int(RECEDE_DELAY, 25);
	fake.set_float(SPEED_MODIFIER, 0.75);

	assert!(watcher.is_disabled().unwrap());
	assert!(watcher.can_recede().unwrap());
	assert!(watcher.handles_train_movement().unwrap());
	assert_eq!(watcher.progress().unwrap(), 0.4);
	assert_eq!(watcher.speed_level().unwrap(), -1);
	assert_eq!(watcher.recede_time().unwrap(), Some(33.0));
	assert_eq!(watcher.cappers().unwrap(), 3);
	assert_eq!(watcher.recede_delay().unwrap(), Some(25));
	assert_eq!(watcher.speed_forward_modifier().unwrap(), 0.75);

	// A delay of 0 or less is the cvar's.
	fake.set_int(RECEDE_DELAY, -5);
	assert_eq!(watcher.recede_delay().unwrap(), None);
}

#[test]
fn watchers_find_their_train() {
	let mut fake = watcher();
	let scope = ();
	let watcher = TrainWatcher::new(mock_server(&scope), fake.entity()).unwrap();

	fake.set_string(TRAIN, c"minecart");

	assert_eq!(watcher.train_name().unwrap().as_c_str(), c"minecart");
	assert!(watcher.train().unwrap().is_none());

	register_name(fake.world.as_ptr(), c"Minecart");

	assert_eq!(
		watcher.train().unwrap().unwrap().as_ptr(),
		fake.world.as_ptr()
	);
}
