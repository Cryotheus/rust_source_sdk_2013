//! Tests of finding `CEnvSoundscape`'s layout in the game's datamaps.

use source_sdk_2013_raw::entities::datamap::DataMaps;
use source_sdk_2013_raw::soundscapes::EnvSoundscapeLayout;
use source_sdk_2013_raw::test_support::entities::{data_map, field};
use std::ptr::null_mut;

const NAME: usize = 200;

/// Where a fixture's datamap puts `CEnvSoundscape`'s fields.
#[derive(Debug, Clone, Copy)]
struct Offsets {
	name: usize,
	positions: usize,
	proxy: usize,
	disabled: usize,
}

impl Offsets {
	/// The layout `game/server/soundscape.h` declares, from `NAME`.
	const EXPECTED: Self = Self {
		name: NAME,
		positions: NAME + 16,
		proxy: NAME + 80,
		disabled: NAME + 84,
	};

	/// Finds the layout in `CEnvSoundscape`'s map, with its fields at
	/// these offsets.
	fn find(self) -> Option<EnvSoundscapeLayout> {
		let base = data_map(c"CBaseEntity", vec![], null_mut());
		let soundscape = data_map(
			c"CEnvSoundscape",
			vec![
				field(c"m_flRadius", sys::_fieldtypes_FIELD_FLOAT, NAME - 8),
				field(
					c"m_soundscapeName",
					sys::_fieldtypes_FIELD_STRING,
					self.name,
				),
				field(
					c"m_hProxySoundscape",
					sys::_fieldtypes_FIELD_EHANDLE,
					self.proxy,
				),
				field(
					c"m_positionNames[0]",
					sys::_fieldtypes_FIELD_STRING,
					self.positions,
				),
				field(
					c"m_bDisabled",
					sys::_fieldtypes_FIELD_BOOLEAN,
					self.disabled,
				),
			],
			base,
		);

		// SAFETY: The tests' maps are leaked and never changed.
		EnvSoundscapeLayout::find(unsafe { DataMaps::new(soundscape) })
	}

	/// Moves every field `bytes` further into the entity.
	const fn shifted(self, bytes: usize) -> Self {
		Self {
			name: self.name + bytes,
			positions: self.positions + bytes,
			proxy: self.proxy + bytes,
			disabled: self.disabled + bytes,
		}
	}
}

#[test]
fn fields_are_placed_by_the_declared_order() {
	let layout = Offsets::EXPECTED.find().unwrap();
	let entity = null_mut::<sys::CBaseEntity>();

	assert_eq!(layout.name(entity).addr(), NAME);
	assert_eq!(layout.index(entity).addr(), NAME + 8);
	assert_eq!(layout.entity_id(entity).addr(), NAME + 12);
	assert_eq!(layout.disabled(entity).addr(), NAME + 84);
}

#[test]
fn unexpected_layouts_and_other_classes_are_refused() {
	let other = data_map(c"CBaseEntity", vec![], null_mut());

	// SAFETY: The tests' maps are leaked and never changed.
	assert!(EnvSoundscapeLayout::find(unsafe { DataMaps::new(other) }).is_none());

	// Moved as a whole, the layout still fits, so each layout below is
	// refused for its one difference.
	assert!(Offsets::EXPECTED.shifted(8).find().is_some());

	let expected = Offsets::EXPECTED;
	let layouts = [
		// A misaligned name.
		expected.shifted(4),
		// Something other than the index and ID before the position names.
		Offsets {
			positions: NAME + 24,
			proxy: NAME + 88,
			disabled: NAME + 92,
			..expected
		},
		// Other than eight position names.
		Offsets {
			proxy: NAME + 88,
			disabled: NAME + 92,
			..expected
		},
		// Something between the proxy's handle and the disabled flag.
		Offsets {
			disabled: NAME + 88,
			..expected
		},
		// Further into the entity than any of its fields could be.
		expected.shifted(1 << 16),
	];

	for offsets in layouts {
		assert!(offsets.find().is_none(), "{offsets:?}");
	}
}
