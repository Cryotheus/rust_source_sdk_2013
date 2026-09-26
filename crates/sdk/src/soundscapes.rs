//! Soundscapes: the ambient sound sets that `env_soundscape` entities choose
//! for each player.
//!
//! The server and each client load the same list of soundscapes: the files
//! `scripts/soundscapes_manifest.txt` lists, in order, then
//! `scripts/soundscapes_<map>.txt` unless the manifest lists it. A
//! [`SoundscapeIndex`] is a position in that list.
//!
//! Every frame, the game gives some players the nearest enabled soundscape
//! entity they can see, if it differs from the one they have, by writing its
//! index and [`SoundscapeId`] into the player's audio parameters. A client
//! starts a soundscape when those change to a valid index. Nothing stops one:
//! disabling a soundscape entity only keeps the game from choosing it, and a
//! client keeps playing its soundscape until it is given another.

use crate::datatables::NetPropError;
use crate::entities::{Entity, data_fields, data_map_class};
use crate::ffi::{borrow_cstr, copy_cstr};
use crate::interfaces::ServerTools;
use crate::server::{InterfaceError, Server};
use std::ffi::{CStr, CString, c_int};
use std::num::NonZero;

/// A position in the game's list of soundscapes.
///
/// Values come only from the game, so every one is an index the game could
/// have assigned itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoundscapeIndex(c_int);

impl SoundscapeIndex {
	/// No soundscape. A client given it keeps playing the one it has.
	pub const NONE: Self = Self(-1);

	pub const fn get(self) -> c_int {
		self.0
	}

	/// Whether this names no soundscape, as an unknown name resolves.
	pub const fn is_none(self) -> bool {
		self.0 < 0
	}
}

/// A soundscape entity's ID in the game's soundscape system, which players'
/// audio parameters refer to it by (`m_soundscapeEntityId`).
///
/// IDs are positions in the system's list of soundscape entities, from 1.
/// They stay the same for the level, since soundscape entities are never
/// removed before it ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoundscapeId(NonZero<c_int>);

impl SoundscapeId {
	pub const fn get(self) -> c_int {
		self.0.get()
	}

	const fn new(id: c_int) -> Option<Self> {
		match NonZero::new(id) {
			Some(id) if id.get() > 0 => Some(Self(id)),
			_ => None,
		}
	}
}

/// Where `CEnvSoundscape` keeps the fields its datamap leaves out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
	name: usize,
	disabled: usize,
}

impl Layout {
	/// `game/server/soundscape.h` declares the name, the index, the ID, eight
	/// position names, the proxy's handle, then the disabled flag. The datamap
	/// has all but the index and ID, whose place the others pin down.
	fn of(entity: Entity<'_>) -> Option<Self> {
		let map = entity
			.data_maps()
			.find(|&map| data_map_class(map) == Some(c"CEnvSoundscape"))?;

		let offset = |name: &CStr, field_type: sys::fieldtype_t| {
			data_fields(map)
				.iter()
				.find(|field| {
					field.fieldType == field_type
						// SAFETY: Field names are string literals of the game DLL.
						&& unsafe { borrow_cstr(field.fieldName) } == Some(name)
				})
				.and_then(|field| usize::try_from(field.fieldOffset[0]).ok())
		};

		let string = size_of::<sys::string_t>();
		let name = offset(c"m_soundscapeName", sys::_fieldtypes_FIELD_STRING)?;
		let positions = offset(c"m_positionNames[0]", sys::_fieldtypes_FIELD_STRING)?;
		let proxy = offset(c"m_hProxySoundscape", sys::_fieldtypes_FIELD_EHANDLE)?;
		let disabled = offset(c"m_bDisabled", sys::_fieldtypes_FIELD_BOOLEAN)?;

		let fits = name.is_multiple_of(align_of::<sys::string_t>())
			&& positions == name + string + 2 * size_of::<c_int>()
			&& proxy == positions + 8 * string
			&& disabled == proxy + size_of::<sys::CBaseHandle>()
			// Well within any entity, as the datamap's other offsets are.
			&& disabled < 1 << 16;

		fits.then_some(Self { name, disabled })
	}

	const fn index(self) -> usize {
		self.name + size_of::<sys::string_t>()
	}

	const fn id(self) -> usize {
		self.index() + size_of::<c_int>()
	}
}

/// An `env_soundscape`, `env_soundscape_proxy`, or
/// `env_soundscape_triggerable` entity.
#[doc(alias = "CEnvSoundscape")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Soundscape<'s> {
	entity: Entity<'s>,
	layout: Layout,
}

impl<'s> Soundscape<'s> {
	/// Returns `None` unless the entity is a soundscape whose fields are where
	/// the SDK declares them.
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
		// chain, so it lies within the live entity.
		unsafe { self.entity.as_ptr().byte_add(offset).cast() }
	}

	/// The soundscape it gives players (`m_soundscapeIndex`).
	///
	/// The game resolves its name when it spawns, and a proxy copies its main
	/// soundscape's when the level starts.
	pub fn index(self) -> SoundscapeIndex {
		// SAFETY: The field lies within the live entity, and is read without
		// forming a reference, as the game writes it too.
		SoundscapeIndex(unsafe { self.field::<c_int>(self.layout.index()).read() })
	}

	/// Changes the soundscape it gives players from now on. Players it already
	/// gave one are not told.
	pub fn set_index(self, index: SoundscapeIndex) {
		// SAFETY: As for `index`. The game only copies the index into players'
		// audio parameters, and every value it can hold is one the game could
		// assign itself.
		unsafe { self.field::<c_int>(self.layout.index()).write(index.0) };
	}

	/// Its ID in the soundscape system, or `None` if it has none.
	pub fn id(self) -> Option<SoundscapeId> {
		// SAFETY: As for `index`.
		SoundscapeId::new(unsafe { self.field::<c_int>(self.layout.id()).read() })
	}

	/// The name of the soundscape it was given, such as `Halloween.Outside`.
	pub fn name(self) -> Option<CString> {
		// SAFETY: As for `index`. The name is a pooled string or null, and is
		// copied at once.
		unsafe {
			copy_cstr(
				self.field::<sys::string_t>(self.layout.name)
					.read()
					.pszValue,
			)
		}
	}

	/// Whether the game may choose it for players (not `StartDisabled`).
	pub fn is_enabled(self) -> bool {
		// SAFETY: As for `index`. The game only stores 0 or 1.
		unsafe { self.field::<u8>(self.layout.disabled).read() == 0 }
	}
}

impl<'s> ServerTools<'s> {
	/// Spawns a disabled `env_soundscape` naming a soundscape, which resolves
	/// the name as the game's own soundscapes do: [`Soundscape::index`] is the
	/// soundscape's index, or [`SoundscapeIndex::NONE`] if the game has none by
	/// that name.
	///
	/// The game chooses the entity for no player, since it builds its lists of
	/// candidates when the level starts, and never disabled ones. Like every
	/// soundscape, it cannot be removed before the level ends, so spawn one per
	/// name and keep its [`EntityHandle`](crate::entities::EntityHandle).
	pub fn spawn_soundscape(self, name: &CStr) -> Option<Soundscape<'s>> {
		// SAFETY: `CEnvSoundscape`'s constructor only registers the entity with
		// the soundscape system (`game/server/soundscape.cpp`), and creating an
		// entity adds it to the entity list.
		let entity = unsafe { self.create_entity_by_name(c"env_soundscape") }?;

		self.set_key_value(entity, c"soundscape", name);
		self.set_key_value(entity, c"StartDisabled", c"1");

		// SAFETY: Its `Spawn` only looks the name up, in `Precache`.
		unsafe { self.dispatch_spawn(entity) };

		Soundscape::new(entity)
	}
}

/// The soundscape a player's client was last told of, in the player's audio
/// parameters (`m_Local.m_audio`), which only that client receives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerSoundscape {
	/// The soundscape entity that gave it, which the game keeps as the
	/// player's while it stays enabled and in sight.
	pub source: Option<SoundscapeId>,

	pub index: SoundscapeIndex,
}

/// Why a player's soundscape could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum SoundscapeError {
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	#[error(transparent)]
	NetProp(#[from] NetPropError),
}

/// Reads the soundscape a player's client was last told of.
pub fn player_soundscape<'s>(
	server: Server<'s>,
	player: Entity<'s>,
) -> Result<PlayerSoundscape, SoundscapeError> {
	let game = server.server_game_dll()?;
	let index = game
		.entity_net_prop(player, c"m_audio.soundscapeIndex")?
		.get::<i32>(player)?;
	let source = game
		.entity_net_prop(player, c"m_audio.entIndex")?
		.get::<i32>(player)?;

	Ok(PlayerSoundscape {
		source: SoundscapeId::new(source),
		index: SoundscapeIndex(index),
	})
}

/// Tells a player's client of a soundscape, which the client starts if it
/// differs from the one it has and is valid.
///
/// The game replaces it when a different enabled soundscape entity comes into
/// the player's sight, so a soundscape meant to last needs every entity the
/// game could choose to give it too.
pub fn set_player_soundscape<'s>(
	server: Server<'s>,
	player: Entity<'s>,
	soundscape: PlayerSoundscape,
) -> Result<(), SoundscapeError> {
	let game = server.server_game_dll()?;
	let engine = server.valve_engine()?;
	let index = game.entity_net_prop(player, c"m_audio.soundscapeIndex")?;
	let source = game.entity_net_prop(player, c"m_audio.entIndex")?;
	let source_id = soundscape.source.map_or(0, SoundscapeId::get);

	// SAFETY: The values are ones the game assigns itself: an index read from a
	// soundscape entity or -1, and 0 or the ID of a soundscape entity, which
	// the game looks up in its list only after checking it is within the list.
	unsafe {
		index.set(engine, player, soundscape.index.get())?;
		source.set(engine, player, source_id)?;
	}

	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::entities::test_support::{
		MockEntity, base_entity_fields, data_map, field, set_datamap,
	};

	const NAME: usize = 200;

	fn soundscape_field(
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

	/// Serves `CEnvSoundscape`'s datamap, with the disabled flag at `disabled`.
	fn serve_soundscape_map(disabled: usize) {
		let base = data_map(
			c"CBaseEntity",
			base_entity_fields().to_vec(),
			std::ptr::null_mut(),
		);
		let soundscape = data_map(
			c"CEnvSoundscape",
			vec![
				soundscape_field(c"m_flRadius", sys::_fieldtypes_FIELD_FLOAT, NAME - 8),
				soundscape_field(c"m_soundscapeName", sys::_fieldtypes_FIELD_STRING, NAME),
				soundscape_field(
					c"m_hProxySoundscape",
					sys::_fieldtypes_FIELD_EHANDLE,
					NAME + 80,
				),
				soundscape_field(
					c"m_positionNames[0]",
					sys::_fieldtypes_FIELD_STRING,
					NAME + 16,
				),
				soundscape_field(c"m_bDisabled", sys::_fieldtypes_FIELD_BOOLEAN, disabled),
			],
			base,
		);

		set_datamap(soundscape);
	}

	#[test]
	fn fields_outside_the_datamap_are_read_and_written_in_place() {
		let mut mock = MockEntity::new(1);
		serve_soundscape_map(NAME + 84);

		let base = mock.as_ptr();

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
			unsafe { base.byte_add(NAME + 8).cast::<c_int>().read() },
			-1
		);
		assert!(soundscape.index().is_none());
	}

	#[test]
	fn unexpected_layouts_and_other_classes_are_refused() {
		let mut mock = MockEntity::new(1);

		assert!(Soundscape::new(mock.entity()).is_none());

		serve_soundscape_map(NAME + 88);

		assert!(Soundscape::new(mock.entity()).is_none());
	}

	#[test]
	fn unassigned_ids_are_none() {
		let mut mock = MockEntity::new(1);
		serve_soundscape_map(NAME + 84);

		unsafe { mock.as_ptr().byte_add(NAME + 12).cast::<c_int>().write(-1) };

		assert_eq!(Soundscape::new(mock.entity()).unwrap().id(), None);
	}

	#[test]
	fn soundscapes_cannot_be_removed() {
		let mut mock = MockEntity::new(1);
		serve_soundscape_map(NAME + 84);

		assert!(mock.entity().is_protected());
	}
}
