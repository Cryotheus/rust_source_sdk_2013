//! Tests of reading the actions of an entity's outputs, found through its data
//! description maps.

use super::*;
use crate::test_support::entities::{MockEntity, base_entity_fields, set_datamap};
use sdk_raw::entities::datamap::FTYPEDESC_KEY;
use sdk_raw::test_support::entities::{data_map, field};
use std::ffi::{c_int, c_short};
use std::ptr::{null, null_mut};

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
