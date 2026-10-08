//! Tests of finding `CAmbientGeneric`'s layout in the game's datamaps.

use source_sdk_2013_raw::ambient_sounds::AmbientGenericLayout;
use source_sdk_2013_raw::entities::datamap::DataMaps;
use source_sdk_2013_raw::test_support::entities::{data_map, field};
use std::ptr::null_mut;

const ACTIVE: usize = 200;
const SOUND: usize = SOURCE_NAME + 16;

/// After the two flags at `ACTIVE`, the `MAX_PATH` characters of the file
/// name, then padding to the next string.
const SOURCE_NAME: usize = 464;

/// Where a fixture's datamap puts `CAmbientGeneric`'s fields.
#[derive(Debug, Clone, Copy)]
struct Offsets {
	active: usize,
	looping: usize,
	source_name: usize,
	sound: usize,
}

impl Offsets {
	/// The layout `game/server/sound.cpp` declares, from `ACTIVE`.
	const EXPECTED: Self = Self {
		active: ACTIVE,
		looping: ACTIVE + 1,
		source_name: SOURCE_NAME,
		sound: SOUND,
	};

	/// Finds the layout in `CAmbientGeneric`'s map, with its fields at
	/// these offsets.
	fn find(self) -> Option<AmbientGenericLayout> {
		let base = data_map(c"CBaseEntity", vec![], null_mut());
		let ambient = data_map(
			c"CAmbientGeneric",
			vec![
				field(c"m_iszSound", sys::_fieldtypes_FIELD_SOUNDNAME, self.sound),
				field(c"m_radius", sys::_fieldtypes_FIELD_FLOAT, ACTIVE - 120),
				field(
					c"m_sSourceEntName",
					sys::_fieldtypes_FIELD_STRING,
					self.source_name,
				),
				field(c"m_fActive", sys::_fieldtypes_FIELD_BOOLEAN, self.active),
				field(c"m_fLooping", sys::_fieldtypes_FIELD_BOOLEAN, self.looping),
			],
			base,
		);

		// SAFETY: The tests' maps are leaked and never changed.
		AmbientGenericLayout::find(unsafe { DataMaps::new(ambient) })
	}

	/// Moves every field `bytes` further into the entity.
	const fn shifted(self, bytes: usize) -> Self {
		Self {
			active: self.active + bytes,
			looping: self.looping + bytes,
			source_name: self.source_name + bytes,
			sound: self.sound + bytes,
		}
	}
}

#[test]
fn fields_are_placed_by_the_declared_order() {
	let layout = Offsets::EXPECTED.find().unwrap();
	let entity = null_mut::<sys::CBaseEntity>();
	let at = |field: *mut u8| field.addr();

	assert_eq!(at(layout.active(entity)), ACTIVE);
	assert_eq!(at(layout.looping(entity)), ACTIVE + 1);
	assert_eq!(at(layout.sound_source(entity).cast()), SOURCE_NAME + 8);
	assert_eq!(at(layout.sound(entity).cast()), SOUND);
}

#[test]
fn unexpected_layouts_and_other_classes_are_refused() {
	let other = data_map(c"CBaseEntity", vec![], null_mut());

	// SAFETY: The tests' maps are leaked and never changed.
	assert!(AmbientGenericLayout::find(unsafe { DataMaps::new(other) }).is_none());

	// Moved as a whole, the layout still fits, so each layout below is
	// refused for its one difference.
	assert!(Offsets::EXPECTED.shifted(8).find().is_some());

	let expected = Offsets::EXPECTED;
	let layouts = [
		// Something between the playing and looping flags.
		Offsets {
			looping: ACTIVE + 2,
			..expected
		},
		// Other than `MAX_PATH` characters before the source's name.
		Offsets {
			source_name: SOURCE_NAME + 8,
			sound: SOUND + 8,
			..expected
		},
		// Something between the source's index and the sound's name.
		Offsets {
			sound: SOUND + 8,
			..expected
		},
		// Further into the entity than any of its fields could be.
		expected.shifted(1 << 16),
	];

	for offsets in layouts {
		assert!(offsets.find().is_none(), "{offsets:?}");
	}
}
