//! Tests of reading the actions of an entity's outputs, found through its data
//! description maps.

use super::*;
use crate::test_support::entities::{MockEntity, base_entity_fields, set_datamap};
use sdk_raw::entities::datamap::FTYPEDESC_KEY;
use sdk_raw::test_support::entities::{data_map, field};
use std::ffi::{c_int, c_short};
use std::ptr::{null, null_mut};

/// Where the mock entity's base class declares its outputs.
const BASE_OUTPUTS_OFFSET: usize = 320;

/// Where the mock entity embeds an object, whose output lies 8 bytes in.
const EMBEDDED_OFFSET: usize = 256;

/// Where the mock entity's first output lies, with a second one after it.
const OUTPUTS_OFFSET: usize = 128;

/// Where the mock entity's string key that is no output lies.
const STRING_OFFSET: usize = 192;

/// An action the game allocated, sending `input` to `target` with
/// `parameter`, before `next`.
fn action(
	target: &'static CStr,
	input: &'static CStr,
	parameter: Option<&'static CStr>,
	(delay, times_to_fire): (f32, c_int),
	next: *mut sys::CEventAction,
) -> *mut sys::CEventAction {
	let string = |string: Option<&'static CStr>| sys::string_t {
		pszValue: string.map_or(null(), CStr::as_ptr),
	};

	Box::into_raw(Box::new(sys::CEventAction {
		m_iTarget: string(Some(target)),
		m_iTargetInput: string(Some(input)),
		m_iParameter: string(parameter),
		m_flDelay: delay,
		m_nTimesToFire: times_to_fire,
		m_iIDStamp: 0,
		m_pNext: next,
	}))
}

#[test]
fn every_output_is_listed_once_by_name() {
	use sys::{_fieldtypes_FIELD_CUSTOM as CUSTOM, _fieldtypes_FIELD_STRING as STRING};

	let mut mock = MockEntity::new(3);
	let output_size = size_of::<sys::CBaseEntityOutput>();

	let mut embedded = field(
		c"m_Touching",
		sys::_fieldtypes_FIELD_EMBEDDED,
		EMBEDDED_OFFSET,
	);

	embedded.fieldSize = 1;
	embedded.td = data_map(
		c"CTouching",
		vec![key(c"OnStartTouch", CUSTOM, 8, FTYPEDESC_OUTPUT)],
		null_mut(),
	);

	let mut base_fields = base_entity_fields().to_vec();

	base_fields.extend([
		// Hidden by the derived class's output of the name.
		key(c"ONCAPTEAM1", CUSTOM, BASE_OUTPUTS_OFFSET, FTYPEDESC_OUTPUT),
		key(
			c"OnUser1",
			CUSTOM,
			BASE_OUTPUTS_OFFSET + output_size,
			FTYPEDESC_OUTPUT,
		),
		// Misaligned, so no output.
		key(
			c"OnUser2",
			CUSTOM,
			BASE_OUTPUTS_OFFSET + 2 * output_size + 4,
			FTYPEDESC_OUTPUT,
		),
	]);

	set_datamap(data_map(
		c"CTriggerMultiple",
		vec![
			embedded,
			key(c"OnCapTeam1", CUSTOM, OUTPUTS_OFFSET, FTYPEDESC_OUTPUT),
			key(c"point_printname", STRING, STRING_OFFSET, 0),
			// Hidden by the embedded object's output of the name.
			key(
				c"onstarttouch",
				CUSTOM,
				OUTPUTS_OFFSET + output_size,
				FTYPEDESC_OUTPUT,
			),
		],
		data_map(c"CBaseEntity", base_fields, null_mut()),
	));

	let kill = action(c"!activator", c"Kill", None, (0.0, 1), null_mut());

	// SAFETY: The mock's zeroed storage holds the embedded object's output at
	// the offset, which takes the action.
	unsafe {
		let output = mock
			.as_ptr()
			.byte_add(EMBEDDED_OFFSET + 8)
			.cast::<sys::CBaseEntityOutput>();

		(&raw mut (*output).m_ActionList).write(kill);
	}

	let entity = mock.entity();
	let outputs = entity.outputs();

	// The embedded object's output comes before the fields after it, and the
	// derived class's before its base's.
	assert_eq!(
		outputs,
		[
			Output {
				name: c"OnStartTouch",
				actions: vec![OutputAction {
					target: c"!activator".into(),
					input: c"Kill".into(),
					parameter: CString::default(),
					delay: 0.0,
					times_to_fire: Some(1),
				}],
			},
			Output {
				name: c"OnCapTeam1",
				actions: Vec::new(),
			},
			Output {
				name: c"OnUser1",
				actions: Vec::new(),
			},
		]
	);

	// Each output is the one its name finds.
	for output in &outputs {
		assert_eq!(
			entity.output_actions(output.name).as_ref(),
			Some(&output.actions)
		);
	}

	assert_eq!(entity.output_actions(c"OnUser2"), None);
}

/// A key of `field_type` named `name`, at `offset` in its object, with
/// `flags` besides [`FTYPEDESC_KEY`].
fn key(
	name: &'static CStr,
	field_type: sys::fieldtype_t,
	offset: usize,
	flags: c_short,
) -> sys::typedescription_t {
	let mut key = field(name, field_type, offset);

	key.fieldSize = 1;
	key.flags = FTYPEDESC_KEY | flags;
	key.externalName = name.as_ptr();
	key
}

#[test]
fn outputs_are_listed_up_to_the_most() {
	let mut mock = MockEntity::new(3);

	let fields = (0..=MAX_OUTPUTS)
		.map(|index| {
			let name = CString::new(format!("OnCase{index:04}")).unwrap();

			key(
				Box::leak(name.into_boxed_c_str()),
				sys::_fieldtypes_FIELD_CUSTOM,
				OUTPUTS_OFFSET,
				FTYPEDESC_OUTPUT,
			)
		})
		.collect();

	set_datamap(data_map(c"CLogicCase", fields, null_mut()));

	let outputs = mock.entity().outputs();

	assert_eq!(outputs.len(), MAX_OUTPUTS);
	assert_eq!(
		outputs.last().map(|output| output.name),
		Some(c"OnCase1023")
	);
}

#[test]
fn outputs_list_their_actions_in_order() {
	let mut mock = MockEntity::new(3);
	let mut fields = base_entity_fields().to_vec();
	let output_size = size_of::<sys::CBaseEntityOutput>();

	fields.extend([
		key(
			c"OnCapTeam1",
			sys::_fieldtypes_FIELD_CUSTOM,
			OUTPUTS_OFFSET,
			FTYPEDESC_OUTPUT,
		),
		key(
			c"OnCapReset",
			sys::_fieldtypes_FIELD_CUSTOM,
			OUTPUTS_OFFSET + output_size,
			FTYPEDESC_OUTPUT,
		),
		key(
			c"point_printname",
			sys::_fieldtypes_FIELD_STRING,
			STRING_OFFSET,
			0,
		),
	]);

	set_datamap(data_map(c"CTeamControlPoint", fields, null_mut()));

	let last = action(c"cap_lights*", c"Skin", None, (0.0, -1), null_mut());
	let first = action(c"cap_base", c"Skin", Some(c"1"), (0.5, 2), last);

	// SAFETY: The mock's zeroed storage holds an output at the offset, which
	// takes the actions.
	unsafe {
		let output = mock
			.as_ptr()
			.byte_add(OUTPUTS_OFFSET)
			.cast::<sys::CBaseEntityOutput>();

		(&raw mut (*output).m_ActionList).write(first);
	}

	let entity = mock.entity();

	assert_eq!(
		entity.output_actions(c"oncapteam1"),
		Some(vec![
			OutputAction {
				target: c"cap_base".into(),
				input: c"Skin".into(),
				parameter: c"1".into(),
				delay: 0.5,
				times_to_fire: Some(2),
			},
			OutputAction {
				target: c"cap_lights*".into(),
				input: c"Skin".into(),
				parameter: CString::default(),
				delay: 0.0,
				times_to_fire: None,
			},
		])
	);

	// An output without actions, a key that is no output, and no key.
	assert_eq!(entity.output_actions(c"OnCapReset"), Some(Vec::new()));
	assert_eq!(entity.output_actions(c"point_printname"), None);
	assert_eq!(entity.output_actions(c"OnCapTeam2"), None);
}
