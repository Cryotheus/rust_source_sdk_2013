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
use crate::entities::{Entity, data_field_offset, data_map_class};
use crate::interfaces::ServerTools;
use crate::server::{InterfaceError, Server};
use sdk_raw::util::cstr::copy_cstr;
use std::ffi::{CStr, CString, c_int};
use std::num::NonZero;

/// The number of position names `CEnvSoundscape` keeps, as many as there are
/// local sounds (`localSound`) in a player's audio parameters.
const POSITION_NAMES: usize = 8;

/// Where `CEnvSoundscape` keeps the fields its datamap leaves out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Layout {
	/// The offset of the soundscape's name, `m_soundscapeName`.
	name: usize,

	/// The offset of the disabled flag, `m_bDisabled`.
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

		let offset = |name, field_type| data_field_offset(map, name, field_type);
		let string = size_of::<sys::string_t>();
		let name = offset(c"m_soundscapeName", sys::_fieldtypes_FIELD_STRING)?;
		let positions = offset(c"m_positionNames[0]", sys::_fieldtypes_FIELD_STRING)?;
		let proxy = offset(c"m_hProxySoundscape", sys::_fieldtypes_FIELD_EHANDLE)?;
		let disabled = offset(c"m_bDisabled", sys::_fieldtypes_FIELD_BOOLEAN)?;

		let fits = name.is_multiple_of(align_of::<sys::string_t>())
			&& positions == name + string + 2 * size_of::<c_int>()
			&& proxy == positions + POSITION_NAMES * string
			&& disabled == proxy + size_of::<sys::CBaseHandle>()
			// Well within any entity, as the datamap's other offsets are.
			&& disabled < 1 << 16;

		fits.then_some(Self { name, disabled })
	}

	/// The offset of the ID, `m_soundscapeEntityId`, right after the index.
	const fn id(self) -> usize {
		self.index() + size_of::<c_int>()
	}

	/// The offset of the index, `m_soundscapeIndex`, right after the name.
	const fn index(self) -> usize {
		self.name + size_of::<sys::string_t>()
	}
}

/// The soundscape a player's client was last told of, in the player's audio
/// parameters (`m_Local.m_audio`), which only that client receives.
#[doc(alias = "audioparams_t")]
#[doc(alias = "m_audio")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerSoundscape {
	/// The soundscape entity that gave it, which the game keeps as the
	/// player's while it stays enabled and in sight.
	#[doc(alias = "entIndex")]
	pub source: Option<SoundscapeId>,

	/// The soundscape's position in the game's list, or none
	/// ([`SoundscapeIndex::is_none`]).
	#[doc(alias = "soundscapeIndex")]
	pub index: SoundscapeIndex,
}

/// An `env_soundscape`, `env_soundscape_proxy`, or
/// `env_soundscape_triggerable` entity.
#[doc(alias = "CEnvSoundscape")]
#[doc(alias = "env_soundscape")]
#[doc(alias = "env_soundscape_proxy")]
#[doc(alias = "env_soundscape_triggerable")]
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

	/// The soundscape entity itself.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Points to the field `offset` bytes into the entity.
	fn field<T>(self, offset: usize) -> *mut T {
		// SAFETY: `Layout::of` found the offset in the entity's own datamap
		// chain, so it lies within the live entity.
		unsafe { self.entity.as_ptr().byte_add(offset).cast() }
	}

	/// Its ID in the soundscape system, or `None` if it has none.
	#[doc(alias = "m_soundscapeEntityId")]
	pub fn id(self) -> Option<SoundscapeId> {
		// SAFETY: As for `index`.
		SoundscapeId::new(unsafe { self.field::<c_int>(self.layout.id()).read() })
	}

	/// The soundscape it gives players (`m_soundscapeIndex`).
	///
	/// The game resolves its name when it spawns, and a proxy copies its main
	/// soundscape's when the level starts.
	#[doc(alias = "m_soundscapeIndex")]
	pub fn index(self) -> SoundscapeIndex {
		// SAFETY: The field lies within the live entity, and is read without
		// forming a reference, as the game writes it too.
		SoundscapeIndex(unsafe { self.field::<c_int>(self.layout.index()).read() })
	}

	/// Whether the game may choose it for players (not `StartDisabled`).
	#[doc(alias = "m_bDisabled")]
	pub fn is_enabled(self) -> bool {
		// SAFETY: As for `index`. The game only stores 0 or 1.
		unsafe { self.field::<u8>(self.layout.disabled).read() == 0 }
	}

	/// The name of the soundscape it was given, such as `Halloween.Outside`.
	/// Returns `None` if the name is null.
	#[doc(alias = "m_soundscapeName")]
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

	/// Changes the soundscape it gives players from now on. Players it already
	/// gave one are not told.
	#[doc(alias = "m_soundscapeIndex")]
	pub fn set_index(self, index: SoundscapeIndex) {
		// SAFETY: As for `index`. The game only copies the index into players'
		// audio parameters, and every value it can hold is one the game could
		// assign itself.
		unsafe { self.field::<c_int>(self.layout.index()).write(index.0) };
	}
}

/// Why a player's soundscape could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum SoundscapeError {
	/// The game DLL or engine does not export an interface it needs.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The player's audio parameters could not be found or accessed, as when
	/// the entity is not networked or not a player.
	#[error(transparent)]
	NetProp(#[from] NetPropError),
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
	/// Returns `None` unless `id` is positive, since IDs start from 1.
	const fn new(id: c_int) -> Option<Self> {
		match NonZero::new(id) {
			Some(id) if id.get() > 0 => Some(Self(id)),
			_ => None,
		}
	}

	/// The ID as the game stores it, which is always positive.
	pub const fn get(self) -> c_int {
		self.0.get()
	}
}

/// A position in the game's list of soundscapes.
///
/// Values come only from the game, so every one is an index the game could
/// have assigned itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoundscapeIndex(c_int);

impl SoundscapeIndex {
	/// No soundscape. A client given it keeps playing the one it has.
	pub const NONE: Self = Self(-1);

	/// The index as the game stores it, which is negative for no soundscape.
	pub const fn get(self) -> c_int {
		self.0
	}

	/// Whether this names no soundscape, as an unknown name resolves.
	pub const fn is_none(self) -> bool {
		self.0 < 0
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
	///
	/// Returns `None` if the game could not create the entity, or if, once it
	/// is spawned, its fields are not where [`Soundscape::new`] expects them.
	/// In that case the entity stays spawned until the level ends, and its
	/// handle is not returned.
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

/// Reads the soundscape a player's client was last told of.
///
/// Fails if the game DLL's interface is missing, or the player's audio
/// parameters cannot be found or read.
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
///
/// Fails if the game DLL's or engine's interface is missing, or the player's
/// audio parameters cannot be found or written. The index is written before
/// the source, so a failure to write the source leaves the index changed.
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
	fn fields_outside_the_datamap_are_read_and_written_in_place() {
		let mut mock = MockEntity::new(1);
		serve_soundscape_map(Offsets::EXPECTED);

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

	/// Serves `CEnvSoundscape`'s datamap, with its fields at `offsets`.
	fn serve_soundscape_map(offsets: Offsets) {
		let base = data_map(
			c"CBaseEntity",
			base_entity_fields().to_vec(),
			std::ptr::null_mut(),
		);
		let soundscape = data_map(
			c"CEnvSoundscape",
			vec![
				soundscape_field(c"m_flRadius", sys::_fieldtypes_FIELD_FLOAT, NAME - 8),
				soundscape_field(
					c"m_soundscapeName",
					sys::_fieldtypes_FIELD_STRING,
					offsets.name,
				),
				soundscape_field(
					c"m_hProxySoundscape",
					sys::_fieldtypes_FIELD_EHANDLE,
					offsets.proxy,
				),
				soundscape_field(
					c"m_positionNames[0]",
					sys::_fieldtypes_FIELD_STRING,
					offsets.positions,
				),
				soundscape_field(
					c"m_bDisabled",
					sys::_fieldtypes_FIELD_BOOLEAN,
					offsets.disabled,
				),
			],
			base,
		);

		set_datamap(soundscape);
	}

	fn soundscape_field(
		name: &'static CStr,
		field_type: sys::fieldtype_t,
		offset: usize,
	) -> sys::typedescription_t {
		let mut field = field();

		field.fieldType = field_type;
		field.fieldName = name.as_ptr();
		field.fieldOffset[0] = c_int::try_from(offset).unwrap();
		field
	}

	#[test]
	fn soundscapes_cannot_be_removed() {
		let mut mock = MockEntity::new(1);
		serve_soundscape_map(Offsets::EXPECTED);

		assert!(mock.entity().is_protected());
	}

	#[test]
	fn unassigned_ids_are_none() {
		let mut mock = MockEntity::new(1);
		serve_soundscape_map(Offsets::EXPECTED);

		unsafe { mock.as_ptr().byte_add(NAME + 12).cast::<c_int>().write(-1) };

		assert_eq!(Soundscape::new(mock.entity()).unwrap().id(), None);
	}

	#[test]
	fn unexpected_layouts_and_other_classes_are_refused() {
		let mut mock = MockEntity::new(1);

		assert!(Soundscape::new(mock.entity()).is_none());

		// Moved as a whole, the layout still fits, so each layout below is
		// refused for its one difference.
		serve_soundscape_map(Offsets::EXPECTED.shifted(8));

		assert!(Soundscape::new(mock.entity()).is_some());

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
			serve_soundscape_map(offsets);

			assert!(Soundscape::new(mock.entity()).is_none(), "{offsets:?}");
		}
	}
}
