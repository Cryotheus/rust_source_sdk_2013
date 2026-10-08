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
#[doc(alias("CAmbientGeneric"))]
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
	#[doc(alias("m_fActive"))]
	pub fn active(self, entity: *mut sys::CBaseEntity) -> *mut u8 {
		entity.wrapping_byte_add(self.active).cast()
	}

	/// The looping flag, `m_fLooping`, of `entity`, right after the playing
	/// flag. As for [`Self::active`].
	#[doc(alias("m_fLooping"))]
	pub fn looping(self, entity: *mut sys::CBaseEntity) -> *mut u8 {
		entity.wrapping_byte_add(self.active + 1).cast()
	}

	/// The sound's name, `m_iszSound`, of `entity`. As for [`Self::active`].
	#[doc(alias("m_iszSound"))]
	pub fn sound(self, entity: *mut sys::CBaseEntity) -> *mut sys::string_t {
		entity.wrapping_byte_add(self.sound).cast()
	}

	/// The handle of the entity it plays its sound from, `m_hSoundSource`, of
	/// `entity`, right after the source's name. As for [`Self::active`].
	#[doc(alias("m_hSoundSource"))]
	pub fn sound_source(self, entity: *mut sys::CBaseEntity) -> *mut sys::CBaseHandle {
		entity
			.wrapping_byte_add(self.source_name + size_of::<sys::string_t>())
			.cast()
	}
}
