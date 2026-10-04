//! Entities in the server's entity list.
//!
//! [`ServerTools`](crate::interfaces::ServerTools) finds entities, including
//! server-only ones. Properties use the native entity interfaces instead of
//! key values, and networked variables are reached through
//! [`NetProp`](crate::datatables::NetProp). Health, life state and damage
//! modes are in [`health`].

pub mod health;

#[cfg(test)]
#[path = "tests/entities.rs"]
mod tests;

use crate::NotThreadSafe;
use crate::datatables::ServerClass;
use crate::edicts::Edict;
use crate::math::{QAngle, Vector};
use sdk_raw::entities::datamap::DataMaps;

use sdk_raw::entities::{
	EFL_KILLME, ENT_ENTRY_MASK, INVALID_EHANDLE_INDEX, NUM_ENT_ENTRIES, NUM_SERIAL_NUM_SHIFT_BITS,
	TeleportSlot,
};

use sdk_raw::util::cstr::borrow_cstr;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};
use std::fmt::{self, Display, Formatter};
use std::marker::PhantomData;
use std::num::NonZero;
use std::ptr::{self, NonNull};
use std::sync::OnceLock;

/// An entity in the server's entity list, as referred to by a `CBaseEntity *`.
///
/// Removing an entity frees it at the end of the frame, so a handle is bound
/// to the scope that produced it. To refer to an entity across callbacks, keep
/// its [`EntityHandle`] and look it up again.
#[doc(alias("CBaseEntity"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity<'s> {
	raw: NonNull<sys::CBaseEntity>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> Entity<'s> {
	/// Wraps a pointer to an entity.
	///
	/// # Safety
	///
	/// `raw` must point to an entity in the entity list that stays allocated
	/// for `'s`.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::CBaseEntity>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Calls Source's `AcceptInput`, which runs the input's handler before
	/// returning, and returns whether it found the input and converted the
	/// value to the input's type.
	///
	/// # Safety
	///
	/// The input, and everything it runs, must free entities only through
	/// deferred deletion. Its handler must accept the activator and caller.
	/// A string value must be pooled, since handlers may keep it.
	pub(crate) unsafe fn accept_input(
		self,
		input: &CStr,
		mut value: sys::variant_t,
		activator: Option<Entity<'_>>,
		caller: Option<Entity<'_>>,
	) -> bool {
		// SAFETY: The entities are live during `'s`, on the main thread, and the
		// value is a local the call may convert in place. The output ID is 0,
		// as for the game's own calls and VScript's `AcceptInput`. The caller
		// upholds the rest.
		unsafe {
			sdk_raw::entities::accept_input(
				self.as_ptr(),
				input,
				activator.map_or(ptr::null_mut(), Entity::as_ptr),
				caller.map_or(ptr::null_mut(), Entity::as_ptr),
				&raw mut value,
				0,
			)
		}
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::CBaseEntity {
		self.raw.as_ptr()
	}

	/// The entity's class name, such as `tf_player`, or an empty string if
	/// Source reports none.
	#[doc(alias("GetClassname"))]
	pub fn class_name(self) -> &'s CStr {
		let Some(networkable) = self.networkable() else {
			return c"";
		};

		// SAFETY: The networkable belongs to the live entity. Class names are
		// pooled strings, which live until the level ends.
		unsafe { borrow_cstr(vcall!(networkable.as_ptr() => IServerNetworkable_GetClassName())) }
			.unwrap_or_default()
	}

	/// The entity's data description maps, from its own class to its bases.
	pub(crate) fn data_maps(self) -> DataMaps<'s> {
		// SAFETY: The entity is live. Its maps are statics of the game DLL,
		// complete since the first entity of its class exists, and the DLL stays
		// loaded for `'s` (`Server::new`).
		unsafe { DataMaps::new(sdk_raw::entities::data_desc_map(self.as_ptr())) }
	}

	/// The entity's edict, or `None` for a server-only entity.
	#[doc(alias("GetEdict"))]
	pub fn edict(self) -> Option<Edict<'s>> {
		let networkable = self.networkable()?;

		// SAFETY: The networkable belongs to the live entity.
		let edict = unsafe { vcall!(networkable.as_ptr() => IServerNetworkable_GetEdict()) };

		// SAFETY: The engine's edict table outlives `'s`.
		NonNull::new(edict).map(|edict| unsafe { Edict::from_raw(edict) })
	}

	/// Finds the offset of a field `CBaseEntity`'s own map declares.
	pub(crate) fn find_base_entity_field(
		self,
		name: &CStr,
		field_type: sys::fieldtype_t,
		size: usize,
	) -> Option<usize> {
		sdk_raw::entities::find_base_entity_field(self.data_maps(), name, field_type, size)
	}

	/// The ID Hammer gave the entity in the map's source file
	/// (`m_iHammerID`), or `None` if it has none. [`HammerId`] describes which
	/// entities have one: those a plugin or script creates usually do not,
	/// but the copies a `point_template` spawns share their template's.
	///
	/// The member is read directly, at the offset the `CBaseEntity` datamap
	/// gives for it. `None` also means that datamap lacks it.
	#[doc(alias("m_iHammerID"))]
	pub fn hammer_id(self) -> Option<HammerId> {
		static HAMMER_ID_OFFSET: OnceLock<Option<usize>> = OnceLock::new();

		let offset = (*HAMMER_ID_OFFSET.get_or_init(|| {
			self.find_base_entity_field(
				c"m_iHammerID",
				sys::_fieldtypes_FIELD_INTEGER,
				size_of::<c_int>(),
			)
		}))?;

		// SAFETY: The offset was validated against the entity datamap, which
		// every entity shares through its `CBaseEntity` base. The member is read
		// without forming a reference, as the game writes it too.
		HammerId::new(unsafe { self.as_ptr().byte_add(offset).cast::<c_int>().read() })
	}

	/// The handle that identifies this entity across frames.
	#[doc(alias("GetRefEHandle"))]
	pub fn handle(self) -> EntityHandle {
		// SAFETY: The entity is live.
		let handle = unsafe { vcall!(self.server_entity() => IServerEntity_GetRefEHandle()) };

		// SAFETY: The handle is a member of the live entity.
		NonNull::new(handle.cast_mut()).map_or(EntityHandle::INVALID, |handle| {
			EntityHandle(unsafe { (&raw const (*handle.as_ptr()).m_Index).read() })
		})
	}

	/// Whether a data description map of the entity's class or one of its
	/// bases is named `class`, such as `CTFPlayer`.
	pub(crate) fn has_data_map_class(self, class: &CStr) -> bool {
		self.data_maps().any(|map| map.class_name() == Some(class))
	}

	/// The entity's slot in the entity list, which is its edict index if it is
	/// networked, or `None` if its handle is invalid.
	#[doc(alias("entindex"))]
	pub fn index(self) -> Option<usize> {
		self.handle().index()
	}

	/// Whether Source has marked this entity for deferred deletion.
	///
	/// # Panics
	///
	/// If the entity's datamaps do not include `CBaseEntity`'s, or it does not
	/// declare `m_iEFlags` as an `int` at an aligned, plausible offset.
	#[doc(alias("EFL_KILLME", "IsMarkedForDeletion"))]
	pub fn is_marked_for_deletion(self) -> bool {
		static EFLAGS_OFFSET: OnceLock<usize> = OnceLock::new();

		let offset = *EFLAGS_OFFSET.get_or_init(|| {
			self.find_base_entity_field(
				c"m_iEFlags",
				sys::_fieldtypes_FIELD_INTEGER,
				size_of::<c_int>(),
			)
			.expect("Source's CBaseEntity datamap does not contain m_iEFlags")
		});

		// SAFETY: The offset was validated against the entity datamap, which
		// every entity shares through its `CBaseEntity` base.
		let flags = unsafe { self.as_ptr().byte_add(offset).cast::<c_int>().read() };

		flags & EFL_KILLME != 0
	}

	/// Whether the game keeps using this entity after it is freed, so that
	/// removing it before the level ends crashes the server.
	///
	/// That is the world, players, and soundscapes: removing a soundscape
	/// shifts the soundscape system's entity list, which the lists it built
	/// for each area of the map then index past.
	pub(crate) fn is_protected(self) -> bool {
		self.index() == Some(0)
			|| self.data_maps().any(|map| {
				matches!(
					map.class_name().map(CStr::to_bytes),
					Some(b"CBasePlayer" | b"CEnvSoundscape")
				)
			})
	}

	/// The entity's name field, `m_iName`, which the `targetname` key sets.
	///
	/// Returns `None` if the `CBaseEntity` datamap has no such field.
	pub(crate) fn name_field(self) -> Option<*mut sys::string_t> {
		static NAME_OFFSET: OnceLock<Option<usize>> = OnceLock::new();

		let offset = (*NAME_OFFSET.get_or_init(|| {
			self.find_base_entity_field(
				c"m_iName",
				sys::_fieldtypes_FIELD_STRING,
				size_of::<sys::string_t>(),
			)
		}))?;

		// SAFETY: The offset was validated against the entity datamap, which
		// every entity shares through its `CBaseEntity` base.
		Some(unsafe { self.as_ptr().byte_add(offset).cast() })
	}

	/// The entity's `IServerNetworkable`, or `None` if Source reports none.
	fn networkable(self) -> Option<NonNull<sys::IServerNetworkable>> {
		// SAFETY: As for `handle`.
		NonNull::new(unsafe { vcall!(self.server_entity() => IServerEntity_GetNetworkable()) })
	}

	/// Reads the absolute origin without key-value conversion.
	///
	/// Source's collision property returns its owner's `GetAbsOrigin()` here.
	/// Returns `None` if the entity has no collideable or it reports no origin.
	#[doc(alias("GetAbsOrigin", "GetCollisionOrigin"))]
	pub fn position(self) -> Option<Vector> {
		// SAFETY: As for `handle`.
		let collideable = NonNull::new(unsafe {
			vcall!(self.server_entity() => IServerEntity_GetCollideable())
		})?;

		// SAFETY: The collideable belongs to the live entity.
		let origin = NonNull::new(
			unsafe { vcall!(collideable.as_ptr() => ICollideable_GetCollisionOrigin()) }.cast_mut(),
		)?;

		// SAFETY: The origin is a member of the live entity, copied immediately.
		Some(unsafe { origin.as_ptr().read() }.into())
	}

	/// The class describing how the entity is networked, or `None` if Source
	/// reports none.
	#[doc(alias("GetServerClass"))]
	pub fn server_class(self) -> Option<ServerClass<'s>> {
		let networkable = self.networkable()?;

		// SAFETY: The networkable belongs to the live entity.
		let class = unsafe { vcall!(networkable.as_ptr() => IServerNetworkable_GetServerClass()) };

		// SAFETY: Server classes are statics of the game DLL.
		NonNull::new(class).map(|class| unsafe { ServerClass::from_raw(class) })
	}

	/// The entity as its primary base, `IServerEntity`, whose generated vtable
	/// starts with the methods of its own bases, `IHandleEntity` and
	/// `IServerUnknown`.
	fn server_entity(self) -> *mut sys::IServerEntity {
		// SAFETY: The entity is live, and its base is only projected to.
		unsafe { &raw mut (*self.as_ptr())._base }
	}

	/// The `string_t` field `GetKeyValue` reads for a key, or `None` if the key
	/// finds no field, or one of another type.
	///
	/// Fields are searched as `ExtractKeyvalue` does, as
	/// [`DataMaps::find_key_field`] describes.
	pub(crate) fn string_key_field(self, key: &CStr) -> Option<*const sys::string_t> {
		let (field, offset) = self.data_maps().find_key_field(key.to_bytes())?;

		let is_string = matches!(
			field.fieldType,
			sys::_fieldtypes_FIELD_STRING
				| sys::_fieldtypes_FIELD_MODELNAME
				| sys::_fieldtypes_FIELD_SOUNDNAME
		);

		// SAFETY: The offset was found in the entity's own datamap chain.
		(is_string && offset.is_multiple_of(align_of::<sys::string_t>()))
			.then(|| unsafe { self.as_ptr().byte_add(offset).cast_const().cast() })
	}

	/// Moves the entity through Source's `Teleport` method, found at `slot` of
	/// its vtable. Each argument left as `None` is unchanged.
	///
	/// # Safety
	///
	/// `slot` must be where the loaded game DLL's primary `CBaseEntity` vtable
	/// has `Teleport`, such as `Game::teleport_vtable_slot` of the game the
	/// server's game DLL was built for.
	pub(crate) unsafe fn teleport(
		self,
		slot: TeleportSlot,
		origin: Option<Vector>,
		angles: Option<QAngle>,
		velocity: Option<Vector>,
	) -> Result<(), TeleportError> {
		let finite_angles = |angles: QAngle| {
			angles.pitch.is_finite() && angles.yaw.is_finite() && angles.roll.is_finite()
		};

		if !(origin.is_none_or(|origin| origin.is_finite())
			&& angles.is_none_or(finite_angles)
			&& velocity.is_none_or(|velocity| velocity.is_finite()))
		{
			return Err(TeleportError::NonFinite);
		}

		if self.is_marked_for_deletion() {
			return Err(TeleportError::MarkedForDeletion);
		}

		let origin = origin.map(sys::Vector::from);
		let angles = angles.map(sys::QAngle::from);
		let velocity = velocity.map(sys::Vector::from);

		// SAFETY: The entity is live during `'s`, on the main thread, and the
		// caller guarantees `Teleport` is at `slot` of the game DLL's vtable,
		// which the entity's class keeps. Every pointer is null or a local, and
		// what the move runs frees entities only through deferred deletion
		// (`Server::new` condition 4).
		unsafe {
			sdk_raw::entities::teleport(
				self.as_ptr(),
				slot,
				origin.as_ref().map_or(ptr::null(), ptr::from_ref),
				angles.as_ref().map_or(ptr::null(), ptr::from_ref),
				velocity.as_ref().map_or(ptr::null(), ptr::from_ref),
			)
		};

		Ok(())
	}
}

/// Identifies an entity across frames without keeping it alive, like the
/// game's `CBaseHandle` and `EHANDLE`.
///
/// A handle combines the entity's slot in the entity list with the serial
/// number the slot had when the entity was created. Looking a handle up with
/// [`ServerTools::entity_by_handle`](crate::interfaces::ServerTools::entity_by_handle)
/// after its entity was removed finds nothing, rather than whatever entity
/// reused the slot.
#[doc(alias("CBaseHandle", "EHANDLE"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityHandle(u32);

impl EntityHandle {
	/// `INVALID_EHANDLE_INDEX`, which refers to no entity.
	#[doc(alias("INVALID_EHANDLE_INDEX"))]
	pub const INVALID: Self = Self(INVALID_EHANDLE_INDEX);

	/// `NUM_ENT_ENTRIES`, the number of slots in the entity list.
	#[doc(alias("NUM_ENT_ENTRIES"))]
	pub const SLOTS: usize = NUM_ENT_ENTRIES;

	/// Wraps a handle's raw value, the `m_Index` a `CBaseHandle` stores.
	pub const fn from_raw(raw: u32) -> Self {
		Self(raw)
	}

	/// The entity's slot in the entity list, or `None` for an invalid handle.
	#[doc(alias("GetEntryIndex"))]
	pub const fn index(self) -> Option<usize> {
		if self.is_valid() {
			Some((self.0 & ENT_ENTRY_MASK) as usize)
		} else {
			None
		}
	}

	/// Whether the handle is not [`INVALID`](Self::INVALID). A valid handle
	/// can still refer to an entity that has since been removed.
	#[doc(alias("IsValid"))]
	pub const fn is_valid(self) -> bool {
		self.0 != Self::INVALID.0
	}

	/// The serial number the entity's slot had when the entity was created,
	/// which tells it apart from later entities in the same slot.
	#[doc(alias("GetSerialNumber"))]
	pub const fn serial_number(self) -> u32 {
		self.0 >> NUM_SERIAL_NUM_SHIFT_BITS
	}

	/// The handle's raw value, the `m_Index` a `CBaseHandle` stores.
	#[doc(alias("ToInt"))]
	pub const fn to_raw(self) -> u32 {
		self.0
	}
}

impl Display for EntityHandle {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self.index() {
			Some(index) => write!(f, "{index}:{}", self.serial_number()),
			None => f.write_str("invalid"),
		}
	}
}

/// The ID Hammer gave an entity in the map's source file, which the compiled
/// map keeps as the entity's `hammerid` key, and the game as `m_iHammerID`.
///
/// The map compiler gives one to every entity it writes, the world included,
/// and the game creates them again from the map with the same IDs when a
/// round restarts. Entities added to a compiled map by other tools, such as
/// Stripper:Source, have none. Neither do entities created while the game
/// runs, unless given one through the `hammerid` key, as the copies a
/// `point_template` spawns are: they share their template entity's ID.
#[doc(alias("hammerid", "m_iHammerID"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HammerId(NonZero<c_int>);

impl HammerId {
	/// Returns `None` for 0, which the game stores for entities without one.
	pub const fn new(id: c_int) -> Option<Self> {
		match NonZero::new(id) {
			Some(id) => Some(Self(id)),
			None => None,
		}
	}

	/// The ID as the game stores it, which is never 0.
	pub const fn get(self) -> c_int {
		self.0.get()
	}
}

impl Display for HammerId {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		Display::fmt(&self.0, f)
	}
}

/// Why [`ServerTools::remove`] refused an entity: the game keeps using it
/// after it is freed.
///
/// [`ServerTools::remove`]: crate::interfaces::ServerTools::remove
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the game keeps using this entity after it is freed")]
pub struct ProtectedEntity;

/// An entity cannot be teleported as requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TeleportError {
	/// A component of the origin, angles, or velocity is infinite or NaN.
	#[error("the destination contains a non-finite component")]
	NonFinite,

	/// The entity is [marked for deletion](Entity::is_marked_for_deletion).
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,
}
