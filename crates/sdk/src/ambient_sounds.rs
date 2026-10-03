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

#[cfg(test)]
#[path = "tests/ambient_sounds.rs"]
mod tests;

use crate::entities::{Entity, EntityHandle};
use sdk_raw::ambient_sounds::AmbientGenericLayout;
use sdk_raw::util::cstr::copy_cstr;
use std::ffi::CString;

/// An `ambient_generic` entity.
#[doc(alias("ambient_generic", "CAmbientGeneric"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmbientSound<'s> {
	entity: Entity<'s>,
	layout: AmbientGenericLayout,
}

impl<'s> AmbientSound<'s> {
	/// Returns `None` unless the entity is an ambient sound whose fields are
	/// where the SDK declares them.
	pub fn new(entity: Entity<'s>) -> Option<Self> {
		Some(Self {
			entity,
			layout: AmbientGenericLayout::find(entity.data_maps())?,
		})
	}

	/// The `ambient_generic` entity itself.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Whether its sound loops, unless the map set "Is NOT Looped".
	#[doc(alias("m_fLooping"))]
	pub fn is_looping(self) -> bool {
		// SAFETY: As for `is_playing`.
		unsafe { self.layout.looping(self.entity.as_ptr()).read() != 0 }
	}

	/// Whether the game plays its looping sound (`m_fActive`), which it does
	/// from the level's start unless the map starts it silent, and while the
	/// map has it play. The game never marks a sound that does not loop as
	/// playing. A muted ambient sound can be playing without being heard.
	#[doc(alias("m_fActive"))]
	pub fn is_playing(self) -> bool {
		// SAFETY: The layout was found in the live entity's own maps, so the
		// field lies within it, and is read without forming a reference, as
		// the game writes it too. The game only stores 0 or 1.
		unsafe { self.layout.active(self.entity.as_ptr()).read() != 0 }
	}

	/// Changes the entity it plays its sound from. `None` mutes it: it sends
	/// nothing but stops until it is given a source again.
	///
	/// Clients match a stop to the entity a sound was played from. A muted
	/// ambient sound sends stops to the source found at activation, and one
	/// with a source through that source, so a sound played from any other
	/// source keeps playing until the level ends unless it is stopped while
	/// that source is set.
	#[doc(alias("m_hSoundSource"))]
	pub fn set_source(self, source: Option<Entity<'_>>) {
		let handle = source.map_or(EntityHandle::INVALID, Entity::handle);

		// SAFETY: As for `is_playing`. The handle is invalid, which the game
		// checks for, or a live entity's, whose slot is within the entity
		// list the game looks it up in. The game checks the slot's serial
		// number each time it uses the handle, so it refers to no entity once
		// that one is removed.
		unsafe {
			(&raw mut (*self.layout.sound_source(self.entity.as_ptr())).m_Index)
				.write(handle.to_raw())
		};
	}

	/// The sound it plays, as the map named it: a sound file, or an entry of
	/// the game's sound scripts, such as `Ambient.MachineHum`. Returns `None`
	/// if the name is null.
	#[doc(alias("m_iszSound"))]
	pub fn sound(self) -> Option<CString> {
		// SAFETY: As for `is_playing`. The name is a pooled string or null, and
		// is copied at once.
		unsafe { copy_cstr(self.layout.sound(self.entity.as_ptr()).read().pszValue) }
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
	#[doc(alias("m_hSoundSource"))]
	pub fn source(self) -> Option<EntityHandle> {
		// SAFETY: As for `is_playing`.
		let handle = EntityHandle::from_raw(unsafe {
			(&raw const (*self.layout.sound_source(self.entity.as_ptr())).m_Index).read()
		});

		handle.is_valid().then_some(handle)
	}
}
