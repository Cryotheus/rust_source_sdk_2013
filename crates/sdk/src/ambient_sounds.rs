//! Ambient sounds: the sounds `ambient_generic` entities play, such as the
//! hum of machinery, apart from soundscapes.
//!
//! An ambient sound plays from a source: itself, or the entity its
//! `SourceEntityName` names, which the game finds when it activates the
//! ambient sound, as it does for the map's when the level starts. It sends
//! every sound through its source, except stops, which go to the entity it
//! found then once it has no source. So an ambient sound without a source
//! sends nothing but stops, whatever the map tells it, until it is given its
//! source back.
//!
//! Clients that connect during the level also start the looping sounds that
//! were playing when it loaded, which the game writes into the level's
//! connection (signon) data then. Nothing an ambient sound does afterwards
//! changes that data; stopping the sound again once such a client is in the
//! game does.

use crate::entities::{Entity, EntityHandle, data_field_offset, data_map_class};
use crate::ffi::copy_cstr;
use std::ffi::{CString, c_int};

/// `MAX_PATH`, the length of the sound file name `CAmbientGeneric` keeps.
const MAX_PATH: usize = 260;

/// An `ambient_generic` entity.
#[doc(alias = "CAmbientGeneric")]
#[doc(alias = "ambient_generic")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmbientSound<'s> {
	entity: Entity<'s>,
	layout: Layout,
}

impl<'s> AmbientSound<'s> {
	/// Returns `None` unless the entity is an ambient sound whose fields are
	/// where the SDK declares them.
	pub fn new(entity: Entity<'s>) -> Option<Self> {
		Some(Self {
			entity,
			layout: Layout::of(entity)?,
		})
	}

	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	fn field<T>(self, offset: usize) -> *mut T {
		// SAFETY: `Layout::of` found the offset in the entity's own datamap
		// chain, or next to fields it found there, so it lies within the live
		// entity.
		unsafe { self.entity.as_ptr().byte_add(offset).cast() }
	}

	/// Whether its sound loops, unless the map set "Is NOT Looped".
	pub fn is_looping(self) -> bool {
		// SAFETY: As for `is_playing`.
		unsafe { self.field::<u8>(self.layout.looping()).read() != 0 }
	}

	/// Whether the game plays its looping sound (`m_fActive`), which it does
	/// from the level's start unless the map starts it silent, and while the
	/// map has it play. The game never marks a sound that does not loop as
	/// playing. A muted ambient sound can be playing without being heard.
	pub fn is_playing(self) -> bool {
		// SAFETY: The field lies within the live entity, and is read without
		// forming a reference, as the game writes it too. The game only stores
		// 0 or 1.
		unsafe { self.field::<u8>(self.layout.playing).read() != 0 }
	}

	/// Changes the entity it plays its sound from. `None` mutes it: it sends
	/// nothing but stops until it is given a source again.
	///
	/// Clients match a stop to the entity a sound was played from. A muted
	/// ambient sound sends stops to the source found at activation, and one
	/// with a source through that source, so a sound played from any other
	/// source keeps playing until the level ends unless it is stopped while
	/// that source is set.
	pub fn set_source(self, source: Option<Entity<'_>>) {
		let handle = source.map_or(EntityHandle::INVALID, Entity::handle);

		// SAFETY: As for `is_playing`. The handle is invalid, which the game
		// checks for, or a live entity's, whose slot is within the entity
		// list the game looks it up in. The game checks the slot's serial
		// number each time it uses the handle, so it refers to no entity once
		// that one is removed.
		unsafe {
			self.field::<u32>(self.layout.source())
				.write(handle.to_raw())
		};
	}

	/// The sound it plays, as the map named it: a sound file, or an entry of
	/// the game's sound scripts, such as `Ambient.MachineHum`.
	pub fn sound(self) -> Option<CString> {
		// SAFETY: As for `is_playing`. The name is a pooled string or null, and
		// is copied at once.
		unsafe {
			copy_cstr(
				self.field::<sys::string_t>(self.layout.sound)
					.read()
					.pszValue,
			)
		}
	}

	/// The entity it plays its sound from (`m_hSoundSource`), or `None` if it
	/// is muted, or was never activated: the game activates the map's ambient
	/// sounds, including those it spawns again or from templates, and VScript's
	/// `SpawnEntityFromTable` does too, but
	/// [`ServerTools::dispatch_spawn`](crate::interfaces::ServerTools::dispatch_spawn)
	/// does not.
	///
	/// The handle may refer to an entity removed since. Whether an ambient
	/// sound came from the map is told by [`Entity::hammer_id`] instead.
	pub fn source(self) -> Option<EntityHandle> {
		// SAFETY: As for `is_playing`.
		let handle =
			EntityHandle::from_raw(unsafe { self.field::<u32>(self.layout.source()).read() });

		handle.is_valid().then_some(handle)
	}
}

/// Where `CAmbientGeneric` keeps the fields its datamap leaves out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
	playing: usize,
	source_name: usize,
	sound: usize,
}

impl Layout {
	/// `game/server/sound.cpp` declares the playing and looping flags, the
	/// sound file's name in `MAX_PATH` characters, the source's name, the
	/// source's handle, the index of the source found at activation, then the
	/// sound's name. The datamap has all but the file name, the handle, and
	/// the index, whose places the others pin down.
	fn of(entity: Entity<'_>) -> Option<Self> {
		let map = entity
			.data_maps()
			.find(|&map| data_map_class(map) == Some(c"CAmbientGeneric"))?;

		let offset = |name, field_type| data_field_offset(map, name, field_type);
		let playing = offset(c"m_fActive", sys::_fieldtypes_FIELD_BOOLEAN)?;
		let looping = offset(c"m_fLooping", sys::_fieldtypes_FIELD_BOOLEAN)?;
		let source_name = offset(c"m_sSourceEntName", sys::_fieldtypes_FIELD_STRING)?;
		let sound = offset(c"m_iszSound", sys::_fieldtypes_FIELD_SOUNDNAME)?;

		let fits = looping == playing + 1
			&& source_name == (looping + 1 + MAX_PATH).next_multiple_of(align_of::<sys::string_t>())
			&& sound
			== source_name
			+ size_of::<sys::string_t>()
			+ size_of::<sys::CBaseHandle>()
			+ size_of::<c_int>()
			// Well within any entity, as the datamap's other offsets are.
			&& sound < 1 << 16;

		fits.then_some(Self {
			playing,
			source_name,
			sound,
		})
	}

	const fn looping(self) -> usize {
		self.playing + 1
	}

	const fn source(self) -> usize {
		self.source_name + size_of::<sys::string_t>()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	use crate::entities::test_support::{
		MockEntity, base_entity_fields, data_map, field, set_datamap,
	};

	use std::ffi::CStr;

	const PLAYING: usize = 200;
	const SOUND: usize = SOURCE_NAME + 16;
	const SOURCE_NAME: usize = 464;

	fn ambient_field(
		name: &'static CStr,
		field_type: sys::fieldtype_t,
		offset: usize,
	) -> sys::typedescription_t {
		let mut field = field();

		field.fieldType = field_type;
		field.fieldName = name.as_ptr();
		field.fieldOffset[0] = offset as c_int;
		field
	}

	#[test]
	fn fields_outside_the_datamap_are_read_and_written_in_place() {
		let mut mock = MockEntity::new(1);
		serve_ambient_map(SOUND);

		let base = mock.as_ptr();

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
			unsafe { base.byte_add(SOURCE_NAME + 8).cast::<u32>().read() },
			u32::MAX
		);
		assert_eq!(sound.source(), None);
		// The index of the source found at activation is left alone.
		assert_eq!(
			unsafe { base.byte_add(SOURCE_NAME + 12).cast::<c_int>().read() },
			0
		);

		let mut other = MockEntity::new(0x0004_0022);

		serve_ambient_map(SOUND);
		sound.set_source(Some(other.entity()));

		assert_eq!(sound.source(), Some(EntityHandle::from_raw(0x0004_0022)));
	}

	/// Serves `CAmbientGeneric`'s datamap, with the sound's name at `sound`.
	fn serve_ambient_map(sound: usize) {
		let base = data_map(
			c"CBaseEntity",
			base_entity_fields().to_vec(),
			std::ptr::null_mut(),
		);
		let ambient = data_map(
			c"CAmbientGeneric",
			vec![
				ambient_field(c"m_iszSound", sys::_fieldtypes_FIELD_SOUNDNAME, sound),
				ambient_field(c"m_radius", sys::_fieldtypes_FIELD_FLOAT, PLAYING - 120),
				ambient_field(
					c"m_sSourceEntName",
					sys::_fieldtypes_FIELD_STRING,
					SOURCE_NAME,
				),
				ambient_field(c"m_fActive", sys::_fieldtypes_FIELD_BOOLEAN, PLAYING),
				ambient_field(c"m_fLooping", sys::_fieldtypes_FIELD_BOOLEAN, PLAYING + 1),
			],
			base,
		);

		set_datamap(ambient);
	}

	#[test]
	fn the_file_name_pins_the_source_name() {
		// The flags, then 260 characters, then padding to the next string.
		assert_eq!(
			(PLAYING + 2 + MAX_PATH).next_multiple_of(align_of::<sys::string_t>()),
			SOURCE_NAME
		);
	}

	#[test]
	fn unexpected_layouts_and_other_classes_are_refused() {
		let mut mock = MockEntity::new(1);

		assert!(AmbientSound::new(mock.entity()).is_none());

		serve_ambient_map(SOUND + 8);

		assert!(AmbientSound::new(mock.entity()).is_none());
	}
}
