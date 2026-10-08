//! Tests of soundscape entities' fields, reached in entity memory at offsets
//! found through the game's data description map, and of their protection
//! from removal.

use super::*;
use crate::test_support::entities::{MockEntity, base_entity_fields, set_datamap};
use sdk_raw::test_support::entities::{data_map, field};

const NAME: usize = 200;

/// The layout's refusals are tested in `sdk_raw::soundscapes`; this checks
/// that the fields are reached through it.
#[test]
fn fields_outside_the_datamap_are_read_and_written_in_place() {
	let mut mock = MockEntity::new(1);

	// Other classes are refused.
	assert!(Soundscape::new(mock.entity()).is_none());

	serve_soundscape_map();

	let base = mock.as_ptr();

	// SAFETY: The fields lie within the mock's storage, aligned for what is
	// written there.
	unsafe {
		base.byte_add(NAME + 8).cast::<c_int>().write(55);
		base.byte_add(NAME + 12).cast::<c_int>().write(3);
		base.byte_add(NAME)
			.cast::<sys::string_t>()
			.write(sys::string_t {
				pszValue: c"Halloween.Outside".as_ptr(),
			});
		base.byte_add(NAME + 84).cast::<u8>().write(1);
	}

	let soundscape = Soundscape::new(mock.entity()).unwrap();

	assert_eq!(soundscape.index(), SoundscapeIndex(55));
	assert_eq!(soundscape.id().map(SoundscapeId::get), Some(3));
	assert_eq!(soundscape.name().as_deref(), Some(c"Halloween.Outside"));
	assert!(!soundscape.is_enabled());

	soundscape.set_index(SoundscapeIndex::NONE);

	assert_eq!(
		// SAFETY: As for the writes above.
		unsafe { base.byte_add(NAME + 8).cast::<c_int>().read() },
		-1
	);
	assert!(soundscape.index().is_none());
}

/// Serves `CEnvSoundscape`'s datamap, with its fields where
/// `game/server/soundscape.h` declares them, from `NAME`.
fn serve_soundscape_map() {
	let base = data_map(
		c"CBaseEntity",
		base_entity_fields().to_vec(),
		std::ptr::null_mut(),
	);
	let soundscape = data_map(
		c"CEnvSoundscape",
		vec![
			field(c"m_flRadius", sys::_fieldtypes_FIELD_FLOAT, NAME - 8),
			field(c"m_soundscapeName", sys::_fieldtypes_FIELD_STRING, NAME),
			field(
				c"m_hProxySoundscape",
				sys::_fieldtypes_FIELD_EHANDLE,
				NAME + 80,
			),
			field(
				c"m_positionNames[0]",
				sys::_fieldtypes_FIELD_STRING,
				NAME + 16,
			),
			field(c"m_bDisabled", sys::_fieldtypes_FIELD_BOOLEAN, NAME + 84),
		],
		base,
	);

	set_datamap(soundscape);
}

#[test]
fn soundscapes_cannot_be_removed() {
	let mut mock = MockEntity::new(1);
	serve_soundscape_map();

	assert!(mock.entity().is_protected());
}
