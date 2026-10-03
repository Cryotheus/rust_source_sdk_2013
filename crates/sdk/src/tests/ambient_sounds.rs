//! Tests of ambient sounds' fields, reached in entity memory at offsets
//! found through the game's data description map.

use super::*;
use crate::test_support::entities::{MockEntity, base_entity_fields, set_datamap};
use sdk_raw::test_support::entities::{data_map, field};
use std::ffi::c_int;

const PLAYING: usize = 200;
const SOUND: usize = SOURCE_NAME + 16;

/// After the two flags at `PLAYING`, the `MAX_PATH` characters of the file
/// name, then padding to the next string.
const SOURCE_NAME: usize = 464;

/// The layout's refusals are tested in `sdk_raw::ambient_sounds`; this
/// checks that the fields are reached through it.
#[test]
fn fields_outside_the_datamap_are_read_and_written_in_place() {
	let mut mock = MockEntity::new(1);

	// Other classes are refused.
	assert!(AmbientSound::new(mock.entity()).is_none());

	serve_ambient_map();

	let base = mock.as_ptr();

	// SAFETY: The fields lie within the mock's storage, aligned for what is
	// written there.
	unsafe {
		base.byte_add(PLAYING).cast::<u8>().write(1);
		base.byte_add(PLAYING + 1).cast::<u8>().write(0);
		base.byte_add(SOURCE_NAME + 8)
			.cast::<u32>()
			.write(0x0003_0011);
		base.byte_add(SOUND)
			.cast::<sys::string_t>()
			.write(sys::string_t {
				pszValue: c"Ambient.MachineHum".as_ptr(),
			});
	}

	let sound = AmbientSound::new(mock.entity()).unwrap();

	assert!(sound.is_playing());
	assert!(!sound.is_looping());
	assert_eq!(sound.sound().as_deref(), Some(c"Ambient.MachineHum"));
	assert_eq!(sound.source(), Some(EntityHandle::from_raw(0x0003_0011)));

	sound.set_source(None);

	assert_eq!(
		// SAFETY: As for the writes above.
		unsafe { base.byte_add(SOURCE_NAME + 8).cast::<u32>().read() },
		u32::MAX
	);
	assert_eq!(sound.source(), None);
	// The index of the source found at activation is left alone.
	assert_eq!(
		// SAFETY: As for the writes above.
		unsafe { base.byte_add(SOURCE_NAME + 12).cast::<c_int>().read() },
		0
	);

	let mut other = MockEntity::new(0x0004_0022);

	serve_ambient_map();
	sound.set_source(Some(other.entity()));

	assert_eq!(sound.source(), Some(EntityHandle::from_raw(0x0004_0022)));
}

/// Serves `CAmbientGeneric`'s datamap, with its fields where
/// `game/server/sound.cpp` declares them, from `PLAYING`.
fn serve_ambient_map() {
	let base = data_map(
		c"CBaseEntity",
		base_entity_fields().to_vec(),
		std::ptr::null_mut(),
	);
	let ambient = data_map(
		c"CAmbientGeneric",
		vec![
			field(c"m_iszSound", sys::_fieldtypes_FIELD_SOUNDNAME, SOUND),
			field(c"m_radius", sys::_fieldtypes_FIELD_FLOAT, PLAYING - 120),
			field(
				c"m_sSourceEntName",
				sys::_fieldtypes_FIELD_STRING,
				SOURCE_NAME,
			),
			field(c"m_fActive", sys::_fieldtypes_FIELD_BOOLEAN, PLAYING),
			field(c"m_fLooping", sys::_fieldtypes_FIELD_BOOLEAN, PLAYING + 1),
		],
		base,
	);

	set_datamap(ambient);
}
