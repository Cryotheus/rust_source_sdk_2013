//! Hand-written layout of `env_soundscape` entities (`CEnvSoundscape`), which
//! the generated bindings do not describe.

use crate::entities::datamap::DataMaps;
use std::ffi::c_int;

/// The soundscape index that names none: what `CEnvSoundscape`'s constructor
/// sets `m_soundscapeIndex` to, in `game/server/soundscape.cpp`.
pub const NO_SOUNDSCAPE: c_int = -1;

/// The number of position names `CEnvSoundscape` keeps (`m_positionNames`),
/// as many as there are local sounds (`localSound`) in a player's audio
/// parameters.
pub const POSITION_NAMES: usize = 8;

/// Where a `CEnvSoundscape` keeps its fields, including those its datamap
/// leaves out: the soundscape's index and the entity's ID.
///
/// `game/server/soundscape.h` declares the name, the index, the ID, eight
/// position names, the proxy's handle, then the disabled flag. The datamap
/// has all but the index and ID, whose place the others pin down.
#[doc(alias("CEnvSoundscape"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnvSoundscapeLayout {
	/// The offset of the soundscape's name, `m_soundscapeName`.
	name: usize,

	/// The offset of the disabled flag, `m_bDisabled`.
	disabled: usize,
}

impl EnvSoundscapeLayout {
	/// Finds the layout in `CEnvSoundscape`'s map among `maps`, an entity's.
	///
	/// Returns `None` unless the map is there and declares its fields in the
	/// order `game/server/soundscape.h` does, with the name aligned, well
	/// within any entity.
	pub fn find(mut maps: DataMaps<'_>) -> Option<Self> {
		let map = maps.find(|map| map.class_name() == Some(c"CEnvSoundscape"))?;

		let offset = |name, field_type| map.field_offset(name, field_type);
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

	/// The disabled flag, `m_bDisabled`, of `entity`, a byte the game stores
	/// 0 or 1 in.
	///
	/// The pointer is computed without being dereferenced. It points to the
	/// field if `entity` points to a live `CEnvSoundscape` whose maps this
	/// layout was found in.
	#[doc(alias("m_bDisabled"))]
	pub fn disabled(self, entity: *mut sys::CBaseEntity) -> *mut u8 {
		entity.wrapping_byte_add(self.disabled).cast()
	}

	/// The entity's ID in the soundscape system, `m_soundscapeEntityId`, of
	/// `entity`, right after the index. As for [`Self::disabled`].
	#[doc(alias("m_soundscapeEntityId"))]
	pub fn entity_id(self, entity: *mut sys::CBaseEntity) -> *mut c_int {
		self.index(entity).wrapping_add(1)
	}

	/// The soundscape's index, `m_soundscapeIndex`, of `entity`, right after
	/// the name. As for [`Self::disabled`].
	#[doc(alias("m_soundscapeIndex"))]
	pub fn index(self, entity: *mut sys::CBaseEntity) -> *mut c_int {
		entity
			.wrapping_byte_add(self.name + size_of::<sys::string_t>())
			.cast()
	}

	/// The soundscape's name, `m_soundscapeName`, of `entity`. As for
	/// [`Self::disabled`].
	#[doc(alias("m_soundscapeName"))]
	pub fn name(self, entity: *mut sys::CBaseEntity) -> *mut sys::string_t {
		entity.wrapping_byte_add(self.name).cast()
	}
}
