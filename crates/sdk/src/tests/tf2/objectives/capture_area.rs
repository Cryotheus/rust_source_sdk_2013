//! Tests of capture areas, through a fake entity of their class.

use super::*;
use crate::test_support::entities::take_inputs;
use crate::test_support::server::mock_server;

use crate::test_support::tf2::objectives::{
	FIELDS_OFFSET, FakeClass, FakeObjective, expected, input, key_field, received, register_name,
	take_key_values,
};

const CAPTURE_TIME: usize = FIELDS_OFFSET + 8;
const DISABLED: usize = FIELDS_OFFSET + 12;
const POINT_NAME: usize = FIELDS_OFFSET;

/// A capture area, whose class chain includes a control point's, so that it
/// can find itself as its point.
fn area() -> FakeObjective {
	use sys::{
		_fieldtypes_FIELD_BOOLEAN as BOOLEAN, _fieldtypes_FIELD_FLOAT as FLOAT,
		_fieldtypes_FIELD_STRING as STRING, _fieldtypes_FIELD_VOID as VOID,
	};

	FakeObjective::new(FakeClass {
		maps: vec![
			(
				c"CTriggerAreaCapture",
				vec![
					key_field(
						c"m_iszCapPointName",
						c"area_cap_point",
						STRING,
						POINT_NAME,
						8,
					),
					key_field(c"m_flCapTime", c"area_time_to_cap", FLOAT, CAPTURE_TIME, 4),
					input(c"SetTeamCanCap", STRING),
					input(c"SetControlPoint", STRING),
					input(c"CaptureCurrentCP", VOID),
				],
			),
			(
				c"CBaseTrigger",
				vec![
					key_field(c"m_bDisabled", c"StartDisabled", BOOLEAN, DISABLED, 1),
					input(c"Enable", VOID),
					input(c"Disable", VOID),
					input(c"Toggle", VOID),
				],
			),
			(c"CTeamControlPoint", vec![]),
		],
		base_fields: vec![],
		table: None,
	})
}

#[test]
fn areas_are_controlled_through_their_inputs() {
	let fake = area();
	let scope = ();
	let area = CaptureArea::new(mock_server(&scope), fake.entity()).unwrap();

	take_inputs();
	area.enable().unwrap();
	area.disable().unwrap();
	area.toggle().unwrap();
	area.capture_current_point().unwrap();

	assert_eq!(
		received(),
		expected(&[
			(c"Enable", 0),
			(c"Disable", 0),
			(c"Toggle", 0),
			(c"CaptureCurrentCP", 0),
		])
	);

	area.set_team_can_capture(ScoringTeam::Red, false).unwrap();
	area.set_team_can_capture(ScoringTeam::Blue, true).unwrap();
	area.set_control_point(c"cp_last").unwrap();

	let inputs = take_inputs();
	let strings: Vec<(&CStr, &CStr)> = inputs
		.iter()
		.map(|input| {
			// SAFETY: The pool keeps its strings for the rest of the test.
			let string = unsafe { CStr::from_ptr(input.string) };

			(input.name.as_c_str(), string)
		})
		.collect();

	assert_eq!(
		strings,
		[
			(c"SetTeamCanCap", c"2 0"),
			(c"SetTeamCanCap", c"3 1"),
			(c"SetControlPoint", c"cp_last"),
		]
	);

	// The point's name is set again as a key value, which the game copies.
	assert_eq!(
		take_key_values(),
		[(
			fake.as_ptr(),
			c"area_cap_point".to_owned(),
			c"cp_last".to_owned()
		)]
	);
}

#[test]
fn areas_find_their_point() {
	let mut fake = area();
	let scope = ();
	let area = CaptureArea::new(mock_server(&scope), fake.entity()).unwrap();

	fake.set_string(POINT_NAME, c"cp_middle");
	fake.set_float(CAPTURE_TIME, 4.5);
	fake.set_bool(DISABLED, true);

	assert_eq!(area.control_point_name().unwrap().as_c_str(), c"cp_middle");
	assert_eq!(area.capture_time().unwrap(), 4.5);
	assert!(area.is_disabled().unwrap());

	assert!(area.control_point().unwrap().is_none());
	register_name(fake.as_ptr(), c"CP_Middle");

	let point = area.control_point().unwrap().unwrap();

	assert_eq!(point.entity().as_ptr(), fake.as_ptr());
}
