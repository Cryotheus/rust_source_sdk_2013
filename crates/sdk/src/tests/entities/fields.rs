//! Tests of datamap members read and written by name, as the types their maps
//! declare, and of the changes their writes record for networking.

use super::*;
use crate::test_support::edicts::{set_change_accessor, set_shared_change_info};
use crate::test_support::entities::{MockEntity, set_datamap, set_networking};
use crate::test_support::sdk_core::change_tracking_engine;
use sdk_raw::edicts::FL_EDICT_CHANGED;
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::{data_map, field};
use std::mem::zeroed;
use std::ptr::null_mut;

/// Where the test's maps declare `m_iAmmo`, an array of four `int`s.
const AMMO_OFFSET: usize = 256;

/// Where the test's maps declare `m_vecGoal`, 4 bytes past a multiple of 8.
const GOAL_OFFSET: usize = 276;

/// Where the test's maps declare `m_Inner`, whose map declares `m_nInner` 6
/// bytes into it.
const INNER_OFFSET: usize = 320;

/// Where the test's maps declare `m_flSpeed`.
const SPEED_OFFSET: usize = 272;

/// Where the test's maps declare `m_hTarget`.
const TARGET_OFFSET: usize = 292;

/// A member of `count` elements of `size` bytes.
fn member(
	name: &'static CStr,
	field_type: sys::fieldtype_t,
	offset: usize,
	count: u16,
	size: usize,
) -> sys::typedescription_t {
	let mut member = field(name, field_type, offset);

	member.fieldSize = count;
	member.fieldSizeInBytes = c_int::try_from(size * usize::from(count)).unwrap();
	member
}

#[test]
fn members_are_found_by_name_from_the_class_to_its_bases() {
	let mut mock = MockEntity::new(5);

	set_datamap(test_maps());

	// SAFETY: The storage holds at least 64 words, past every member here.
	unsafe {
		let storage = mock.as_ptr();

		storage
			.byte_add(AMMO_OFFSET)
			.cast::<[c_int; 4]>()
			.write([10, 20, 30, 40]);

		storage.byte_add(SPEED_OFFSET).cast::<f32>().write(1.5);

		storage
			.byte_add(GOAL_OFFSET)
			.cast::<sys::Vector>()
			.write(sys::Vector {
				x: 1.0,
				y: 2.0,
				z: 3.0,
			});

		storage
			.byte_add(TARGET_OFFSET)
			.cast::<u32>()
			.write(9 | 3 << 16);
		storage.byte_add(INNER_OFFSET + 6).cast::<i16>().write(-7);
	}

	let entity = mock.entity();

	assert_eq!(entity.data_field::<c_int>(c"m_iAmmo"), Ok(10));
	assert_eq!(entity.data_element::<c_int>(c"m_iAmmo", 3), Ok(40));
	assert_eq!(entity.data_field::<f32>(c"m_flSpeed"), Ok(1.5));
	assert_eq!(
		entity.data_field::<Vector>(c"m_vecGoal"),
		Ok(Vector::new(1.0, 2.0, 3.0))
	);
	assert_eq!(
		entity.data_field::<QAngle>(c"m_vecGoal"),
		Ok(QAngle {
			pitch: 1.0,
			yaw: 2.0,
			roll: 3.0
		})
	);
	assert_eq!(
		entity.data_field::<EntityHandle>(c"m_hTarget"),
		Ok(EntityHandle::from_raw(9 | 3 << 16))
	);

	// Embedded objects are searched, at their offset.
	assert_eq!(entity.data_field::<i16>(c"m_nInner"), Ok(-7));

	// The class's own map shadows its bases'.
	assert_eq!(entity.data_field::<c_int>(c"m_iShared"), Ok(10));
}

#[test]
fn members_are_only_read_as_their_declared_types() {
	let mut mock = MockEntity::new(5);

	set_datamap(test_maps());

	let entity = mock.entity();
	let name = |name: &CStr| name.to_owned();

	assert_eq!(
		entity.data_field::<c_int>(c"m_iMissing"),
		Err(FieldError::NotFound {
			name: name(c"m_iMissing")
		})
	);
	assert_eq!(
		entity.data_field::<f32>(c"m_iAmmo"),
		Err(FieldError::WrongType {
			name: name(c"m_iAmmo"),
			field_type: sys::_fieldtypes_FIELD_INTEGER,
		})
	);
	assert_eq!(
		entity.data_field::<Color32>(c"m_iAmmo"),
		Err(FieldError::WrongType {
			name: name(c"m_iAmmo"),
			field_type: sys::_fieldtypes_FIELD_INTEGER,
		})
	);
	assert_eq!(
		entity.data_element::<c_int>(c"m_iAmmo", 4),
		Err(FieldError::OutOfBounds {
			name: name(c"m_iAmmo"),
			count: 4,
			element: 4,
		})
	);

	// A member of no elements is not one to read.
	assert_eq!(
		entity.data_field::<c_int>(c"m_iEmpty"),
		Err(FieldError::WrongType {
			name: name(c"m_iEmpty"),
			field_type: sys::_fieldtypes_FIELD_INTEGER,
		})
	);

	// The embedded object itself is no value.
	assert!(matches!(
		entity.data_field::<c_int>(c"m_Inner"),
		Err(FieldError::WrongType { .. })
	));
}

#[test]
fn members_with_implausible_declarations_are_not_read() {
	let mut mock = MockEntity::new(5);
	let int = size_of::<c_int>();

	let misaligned = member(
		c"m_iMisaligned",
		sys::_fieldtypes_FIELD_INTEGER,
		258,
		1,
		int,
	);
	let mut oversized = member(c"m_flOversized", sys::_fieldtypes_FIELD_FLOAT, 260, 1, int);

	oversized.fieldSizeInBytes = 8;
	set_datamap(data_map(
		c"CBaseEntity",
		vec![misaligned, oversized],
		null_mut(),
	));

	let entity = mock.entity();

	assert!(matches!(
		entity.data_field::<c_int>(c"m_iMisaligned"),
		Err(FieldError::WrongType { .. })
	));
	assert!(matches!(
		entity.data_field::<f32>(c"m_flOversized"),
		Err(FieldError::WrongType { .. })
	));
}

/// A map chain from a `CTFThing` map declaring the test's members to a
/// `CBaseEntity` map declaring `m_iShared`, which the derived map shadows.
fn test_maps() -> *mut sys::datamap_t {
	let inner = data_map(
		c"CInner",
		vec![member(
			c"m_nInner",
			sys::_fieldtypes_FIELD_SHORT,
			6,
			1,
			size_of::<i16>(),
		)],
		null_mut(),
	);

	let mut embedded = field(c"m_Inner", sys::_fieldtypes_FIELD_EMBEDDED, INNER_OFFSET);
	embedded.fieldSize = 1;
	embedded.td = inner;

	let int = size_of::<c_int>();
	let base = data_map(
		c"CBaseEntity",
		vec![member(
			c"m_iShared",
			sys::_fieldtypes_FIELD_INTEGER,
			AMMO_OFFSET + 8,
			1,
			int,
		)],
		null_mut(),
	);

	data_map(
		c"CTFThing",
		vec![
			member(
				c"m_iAmmo",
				sys::_fieldtypes_FIELD_INTEGER,
				AMMO_OFFSET,
				4,
				int,
			),
			member(
				c"m_iShared",
				sys::_fieldtypes_FIELD_INTEGER,
				AMMO_OFFSET,
				1,
				int,
			),
			member(
				c"m_flSpeed",
				sys::_fieldtypes_FIELD_FLOAT,
				SPEED_OFFSET,
				1,
				int,
			),
			member(
				c"m_vecGoal",
				sys::_fieldtypes_FIELD_VECTOR,
				GOAL_OFFSET,
				1,
				size_of::<sys::Vector>(),
			),
			member(
				c"m_hTarget",
				sys::_fieldtypes_FIELD_EHANDLE,
				TARGET_OFFSET,
				1,
				int,
			),
			member(
				c"m_iEmpty",
				sys::_fieldtypes_FIELD_INTEGER,
				TARGET_OFFSET + 4,
				0,
				int,
			),
			embedded,
		],
		base,
	)
}

#[test]
fn writes_land_at_the_member_and_record_the_change() {
	let mut mock = MockEntity::new(5);
	let engine = change_tracking_engine();
	// SAFETY: Zero is valid for every field of `CSharedEdictChangeInfo`.
	let mut shared = Box::new(unsafe { zeroed::<sys::CSharedEdictChangeInfo>() });
	let mut accessor = sys::IChangeInfoAccessor {
		m_iChangeInfo: 0,
		m_iChangeInfoSerialNumber: 0,
	};
	let mut edict = mock_edict(5, false);

	shared.m_iSerialNumber = 7;
	set_change_accessor(&raw mut accessor);
	set_shared_change_info(&raw mut *shared);
	set_networking(null_mut(), &raw mut edict);
	set_datamap(test_maps());

	let entity = mock.entity();

	entity.set_data_element(engine, c"m_iAmmo", 2, 33).unwrap();
	entity
		.set_data_field(engine, c"m_flSpeed", 4.25_f32)
		.unwrap();
	entity
		.set_data_field(engine, c"m_vecGoal", Vector::new(4.0, 5.0, 6.0))
		.unwrap();
	entity.set_data_field(engine, c"m_nInner", 12_i16).unwrap();

	assert_eq!(
		entity.set_data_element(engine, c"m_iAmmo", 9, 1),
		Err(FieldError::OutOfBounds {
			name: c"m_iAmmo".to_owned(),
			count: 4,
			element: 9,
		})
	);
	assert!(matches!(
		entity.set_data_field(engine, c"m_flSpeed", 1),
		Err(FieldError::WrongType { .. })
	));

	assert_eq!(entity.data_element::<c_int>(c"m_iAmmo", 2), Ok(33));
	assert_eq!(entity.data_field::<f32>(c"m_flSpeed"), Ok(4.25));
	assert_eq!(
		entity.data_field::<Vector>(c"m_vecGoal"),
		Ok(Vector::new(4.0, 5.0, 6.0))
	);
	assert_eq!(entity.data_field::<i16>(c"m_nInner"), Ok(12));
	assert_ne!(edict._base.m_fStateFlags & FL_EDICT_CHANGED, 0);

	let changes = &shared.m_ChangeInfos[0];
	let offsets = [
		AMMO_OFFSET + 2 * size_of::<c_int>(),
		SPEED_OFFSET,
		GOAL_OFFSET,
		INNER_OFFSET + 6,
	]
	.map(|offset| u16::try_from(offset).unwrap());

	assert_eq!(changes.m_nChangeOffsets, 4);
	assert_eq!(changes.m_ChangeOffsets[..4], offsets);

	set_change_accessor(null_mut());
	set_shared_change_info(null_mut());
	set_networking(null_mut(), null_mut());
}
