//! Hand-written layout of `ambient_generic` entities (`CAmbientGeneric`), which
//! the generated bindings do not describe.

use crate::entities::datamap::DataMaps;
use crate::tier0::MAX_PATH;
use std::ffi::c_int;

/// Where a `CAmbientGeneric` keeps its fields, including those its datamap
/// leaves out: the sound file's name, the source's handle, and the index of
/// the source found at activation.
///
/// `game/server/sound.cpp` declares the playing and looping flags, the sound
/// file's name in [`MAX_PATH`] characters, the source's name, the source's
/// handle, the index of the source found at activation, then the sound's
/// name. The datamap has all but the file name, the handle, and the index,
/// whose places the others pin down.
#[doc(alias = "CAmbientGeneric")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AmbientGenericLayout {
	/// The offset of the playing flag, `m_fActive`.
	active: usize,

	/// The offset of the source's name, `m_sSourceEntName`.
	source_name: usize,

	/// The offset of the sound's name, `m_iszSound`.
	sound: usize,
}

impl AmbientGenericLayout {
	/// Finds the layout in `CAmbientGeneric`'s map among `maps`, an entity's.
	///
	/// Returns `None` unless the map is there and declares its fields in the
	/// order `game/server/sound.cpp` does, well within any entity.
	pub fn find(mut maps: DataMaps<'_>) -> Option<Self> {
		let map = maps.find(|map| map.class_name() == Some(c"CAmbientGeneric"))?;

		let offset = |name, field_type| map.field_offset(name, field_type);
		let active = offset(c"m_fActive", sys::_fieldtypes_FIELD_BOOLEAN)?;
		let looping = offset(c"m_fLooping", sys::_fieldtypes_FIELD_BOOLEAN)?;
		let source_name = offset(c"m_sSourceEntName", sys::_fieldtypes_FIELD_STRING)?;
		let sound = offset(c"m_iszSound", sys::_fieldtypes_FIELD_SOUNDNAME)?;

		let fits = looping == active + 1
			&& source_name == (looping + 1 + MAX_PATH).next_multiple_of(align_of::<sys::string_t>())
			&& sound
				== source_name
					+ size_of::<sys::string_t>()
					+ size_of::<sys::CBaseHandle>()
					+ size_of::<c_int>()
			// Well within any entity, as the datamap's other offsets are.
			&& sound < 1 << 16;

		fits.then_some(Self {
			active,
			source_name,
			sound,
		})
	}

	/// The playing flag, `m_fActive`, of `entity`, a byte the game stores 0
	/// or 1 in.
	///
	/// The pointer is computed without being dereferenced. It points to the
	/// field if `entity` points to a live `CAmbientGeneric` whose maps this
	/// layout was found in.
	#[doc(alias = "m_fActive")]
	pub fn active(self, entity: *mut sys::CBaseEntity) -> *mut u8 {
		entity.wrapping_byte_add(self.active).cast()
	}

	/// The looping flag, `m_fLooping`, of `entity`, right after the playing
	/// flag. As for [`Self::active`].
	#[doc(alias = "m_fLooping")]
	pub fn looping(self, entity: *mut sys::CBaseEntity) -> *mut u8 {
		entity.wrapping_byte_add(self.active + 1).cast()
	}

	/// The sound's name, `m_iszSound`, of `entity`. As for [`Self::active`].
	#[doc(alias = "m_iszSound")]
	pub fn sound(self, entity: *mut sys::CBaseEntity) -> *mut sys::string_t {
		entity.wrapping_byte_add(self.sound).cast()
	}

	/// The handle of the entity it plays its sound from, `m_hSoundSource`, of
	/// `entity`, right after the source's name. As for [`Self::active`].
	#[doc(alias = "m_hSoundSource")]
	pub fn sound_source(self, entity: *mut sys::CBaseEntity) -> *mut sys::CBaseHandle {
		entity
			.wrapping_byte_add(self.source_name + size_of::<sys::string_t>())
			.cast()
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::entities::datamap::test_support::{data_map, field};
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
}
