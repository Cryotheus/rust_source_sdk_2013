//! Tests of entities' solid flags, found through the collision property
//! `CBaseEntity`'s datamap embeds, and of the change tracking their setter
//! reaches.

use super::*;
use crate::interfaces::ValveEngine;
use crate::server::Module;
use crate::test_support::edicts::{set_change_accessor, set_shared_change_info};
use crate::test_support::entities::{MockEntity, base_entity_fields, set_datamap, set_networking};
use crate::test_support::sdk_core::change_tracking_engine;
use crate::test_support::server::{export, mock_server};
use sdk_raw::edicts::FL_EDICT_CHANGED;
use sdk_raw::test_support::edicts::mock_edict;
use sdk_raw::test_support::entities::{data_map, field};
use std::ffi::c_int;
use std::mem::zeroed;
use std::ptr::null_mut;

/// Where mock entities embed their collision property.
const MOCK_COLLISION_OFFSET: usize = 96;

/// Where the mock collision property stores `m_usSolidFlags`.
const MOCK_SOLID_FLAGS_FIELD: usize = 6;

/// Where mock entities store `m_usSolidFlags`.
const MOCK_SOLID_FLAGS_OFFSET: usize = MOCK_COLLISION_OFFSET + MOCK_SOLID_FLAGS_FIELD;

#[test]
fn solid_flags_need_the_embedded_collision_property() {
	let mut mock = MockEntity::new(5);

	// The flags alone, without the collision property embedding them.
	let mut solid_flags = field(
		c"m_usSolidFlags",
		sys::_fieldtypes_FIELD_SHORT,
		MOCK_SOLID_FLAGS_OFFSET,
	);

	solid_flags.fieldSizeInBytes = size_of::<u16>() as c_int;

	let mut fields = Vec::from(base_entity_fields());

	fields.push(solid_flags);

	let maps = data_map(c"CBaseEntity", fields, null_mut());

	assert_eq!(
		find_solid_flags_field(
			// SAFETY: The leaked maps are never modified.
			unsafe { sdk_raw::entities::datamap::DataMaps::new(maps) }
		),
		None
	);

	set_datamap(solid_maps());
	assert_eq!(
		// SAFETY: As above.
		find_solid_flags_field(unsafe { sdk_raw::entities::datamap::DataMaps::new(solid_maps()) }),
		Some(MOCK_SOLID_FLAGS_OFFSET)
	);
	assert_eq!(mock.entity().solid_flags(), Ok(SolidFlags::empty()));
}

/// A `CTFPlayer` map deriving from a `CBaseEntity` map, which embeds a
/// collision property, `m_Collision`, whose map declares `m_usSolidFlags` at
/// [`MOCK_SOLID_FLAGS_OFFSET`].
fn solid_maps() -> *mut sys::datamap_t {
	let mut solid_flags = field(
		c"m_usSolidFlags",
		sys::_fieldtypes_FIELD_SHORT,
		MOCK_SOLID_FLAGS_FIELD,
	);

	solid_flags.fieldSizeInBytes = size_of::<u16>() as c_int;

	let mut collision = field(
		c"m_Collision",
		sys::_fieldtypes_FIELD_EMBEDDED,
		MOCK_COLLISION_OFFSET,
	);

	collision.fieldSize = 1;
	collision.td = data_map(c"CCollisionProperty", vec![solid_flags], null_mut());

	let mut fields = Vec::from(base_entity_fields());

	fields.push(collision);
	data_map(
		c"CTFPlayer",
		vec![],
		data_map(c"CBaseEntity", fields, null_mut()),
	)
}

/// Writes the mock entity's solid flags to its storage.
fn store_flags(mock: &mut MockEntity, flags: u16) {
	// SAFETY: As for `stored_flags`.
	unsafe {
		mock.as_ptr()
			.byte_add(MOCK_SOLID_FLAGS_OFFSET)
			.cast::<u16>()
			.write(flags)
	};
}

/// Reads the mock entity's solid flags from its storage.
fn stored_flags(mock: &mut MockEntity) -> u16 {
	// SAFETY: Mock entities hold 64 words, and the offset is aligned.
	unsafe {
		mock.as_ptr()
			.byte_add(MOCK_SOLID_FLAGS_OFFSET)
			.cast::<u16>()
			.read()
	}
}

#[test]
fn the_custom_ray_test_is_set_alone_and_networked() {
	let mut mock = MockEntity::new(5);

	set_datamap(solid_maps());

	let trigger = FSOLID_NOT_SOLID | FSOLID_TRIGGER | 0x8000;

	store_flags(&mut mock, trigger);

	// Every bit is read, unnamed ones included.
	assert_eq!(
		mock.entity().solid_flags(),
		Ok(SolidFlags::NOT_SOLID | SolidFlags::TRIGGER | SolidFlags::from_bits_retain(0x8000))
	);

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
	export(
		Module::Engine,
		ValveEngine::VERSION,
		change_tracking_engine().as_ptr(),
	);

	let scope = ();
	let server = mock_server(&scope);

	// Setting the flag keeps the others, and reports it was clear.
	assert_eq!(mock.entity().set_custom_ray_test(server, true), Ok(false));
	assert_eq!(stored_flags(&mut mock), trigger | FSOLID_CUSTOMRAYTEST);
	assert_ne!(edict._base.m_fStateFlags & FL_EDICT_CHANGED, 0);
	assert_eq!(shared.m_ChangeInfos[0].m_nChangeOffsets, 1);
	assert_eq!(
		shared.m_ChangeInfos[0].m_ChangeOffsets[0],
		u16::try_from(MOCK_SOLID_FLAGS_OFFSET).unwrap()
	);

	// Setting it again changes nothing, and records no change.
	assert_eq!(mock.entity().set_custom_ray_test(server, true), Ok(true));
	assert_eq!(shared.m_ChangeInfos[0].m_nChangeOffsets, 1);

	assert_eq!(mock.entity().set_custom_ray_test(server, false), Ok(true));
	assert_eq!(stored_flags(&mut mock), trigger);

	set_change_accessor(null_mut());
	set_shared_change_info(null_mut());
	set_networking(null_mut(), null_mut());
}
