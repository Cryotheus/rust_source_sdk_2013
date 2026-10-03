//! Entities in the server's entity list.
//!
//! [`ServerTools`](crate::interfaces::ServerTools) finds entities, including
//! server-only ones. Properties use the native entity interfaces instead of
//! key values, and networked variables are reached through
//! [`NetProp`](crate::datatables::NetProp).

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
#[doc(alias = "CBaseEntity")]
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
	#[doc(alias = "GetClassname")]
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
	#[doc(alias = "GetEdict")]
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
	#[doc(alias = "m_iHammerID")]
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
	#[doc(alias = "GetRefEHandle")]
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
	#[cfg_attr(
		not(feature = "tf2"),
		expect(dead_code, reason = "only the tf2 module checks classes so far")
	)]
	pub(crate) fn has_data_map_class(self, class: &CStr) -> bool {
		self.data_maps().any(|map| map.class_name() == Some(class))
	}

	/// The entity's slot in the entity list, which is its edict index if it is
	/// networked, or `None` if its handle is invalid.
	#[doc(alias = "entindex")]
	pub fn index(self) -> Option<usize> {
		self.handle().index()
	}

	/// Whether Source has marked this entity for deferred deletion.
	///
	/// # Panics
	///
	/// If the entity's datamaps do not include `CBaseEntity`'s, or it does not
	/// declare `m_iEFlags` as an `int` at an aligned, plausible offset.
	#[doc(alias = "EFL_KILLME")]
	#[doc(alias = "IsMarkedForDeletion")]
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
	#[doc(alias = "GetAbsOrigin")]
	#[doc(alias = "GetCollisionOrigin")]
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
	#[doc(alias = "GetServerClass")]
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
#[doc(alias = "CBaseHandle")]
#[doc(alias = "EHANDLE")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityHandle(u32);

impl EntityHandle {
	/// `INVALID_EHANDLE_INDEX`, which refers to no entity.
	#[doc(alias = "INVALID_EHANDLE_INDEX")]
	pub const INVALID: Self = Self(INVALID_EHANDLE_INDEX);

	/// `NUM_ENT_ENTRIES`, the number of slots in the entity list.
	#[doc(alias = "NUM_ENT_ENTRIES")]
	pub const SLOTS: usize = NUM_ENT_ENTRIES;

	/// Wraps a handle's raw value, the `m_Index` a `CBaseHandle` stores.
	pub const fn from_raw(raw: u32) -> Self {
		Self(raw)
	}

	/// The entity's slot in the entity list, or `None` for an invalid handle.
	#[doc(alias = "GetEntryIndex")]
	pub const fn index(self) -> Option<usize> {
		if self.is_valid() {
			Some((self.0 & ENT_ENTRY_MASK) as usize)
		} else {
			None
		}
	}

	/// Whether the handle is not [`INVALID`](Self::INVALID). A valid handle
	/// can still refer to an entity that has since been removed.
	#[doc(alias = "IsValid")]
	pub const fn is_valid(self) -> bool {
		self.0 != Self::INVALID.0
	}

	/// The serial number the entity's slot had when the entity was created,
	/// which tells it apart from later entities in the same slot.
	#[doc(alias = "GetSerialNumber")]
	pub const fn serial_number(self) -> u32 {
		self.0 >> NUM_SERIAL_NUM_SHIFT_BITS
	}

	/// The handle's raw value, the `m_Index` a `CBaseHandle` stores.
	#[doc(alias = "ToInt")]
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

#[cfg(test)]
pub(crate) mod test_support {
	use super::*;
	use sdk_raw::util::mock::unexpected_call;
	use std::cell::{Cell, RefCell};
	use std::ffi::CString;
	use std::mem::offset_of;
	use std::ptr::null_mut;

	/// Where mock entities store `m_iEFlags`, as their datamap declares.
	pub(crate) const MOCK_EFLAGS_OFFSET: usize = 32;

	/// Where mock entities store `m_iHammerID`, as their datamap declares.
	const MOCK_HAMMER_ID_OFFSET: usize = 56;

	/// Where mock entities store the handle `GetRefEHandle` points to.
	const MOCK_HANDLE_OFFSET: usize = 40;

	/// Where mock entities store `m_iName`, as their datamap declares.
	pub(crate) const MOCK_NAME_OFFSET: usize = 48;

	/// An input a mock entity's `AcceptInput` received.
	#[derive(Debug, Clone, PartialEq)]
	pub(crate) struct ReceivedInput {
		pub(crate) target: *mut sys::CBaseEntity,
		pub(crate) name: CString,
		pub(crate) activator: *mut sys::CBaseEntity,
		pub(crate) caller: *mut sys::CBaseEntity,
		pub(crate) field_type: sys::fieldtype_t,
		/// The bytes every union member covers.
		pub(crate) payload: [u8; 12],
		/// The string pointer, read as a pointer, for string values.
		pub(crate) string: *const std::ffi::c_char,
		pub(crate) handle: u32,
		pub(crate) output_id: c_int,
	}

	thread_local! {
		static COLLIDEABLE: Cell<*mut sys::ICollideable> = const { Cell::new(null_mut()) };
		static NETWORKABLE: Cell<*mut sys::IServerNetworkable> = const { Cell::new(null_mut()) };
		static ORIGIN: Cell<sys::Vector> = const { Cell::new(sys::Vector { x: 1.0, y: 2.0, z: 3.0 }) };
		static TELEPORTS: Cell<usize> = const { Cell::new(0) };
		static DATA_MAP: Cell<*mut sys::datamap_t> = const { Cell::new(null_mut()) };
		static SERVER_CLASS: Cell<*mut sys::ServerClass> = const { Cell::new(null_mut()) };
		static EDICT: Cell<*mut sys::edict_t> = const { Cell::new(null_mut()) };
		static INPUTS: RefCell<Vec<ReceivedInput>> = const { RefCell::new(Vec::new()) };
		static ACCEPTS: Cell<bool> = const { Cell::new(true) };
	}

	/// An entity with a class name, origin, datamap, handle, and TF2 `Teleport`,
	/// whose teleports are counted by [`teleports`].
	///
	/// Its allocations are leaked, and only reached through raw pointers, like
	/// the engine's objects.
	pub(crate) struct MockEntity {
		storage: *mut [usize; 64],
	}

	impl MockEntity {
		/// Builds an entity whose handle is `handle`, and resets the datamap,
		/// origin, teleport count, server class, and edict that mock entities on
		/// this thread report.
		pub(crate) fn new(handle: u32) -> Self {
			let slot_count = sdk_raw::entities::TF2_TELEPORT_SLOT
				.max(sdk_raw::entities::GET_DATA_DESC_MAP_SLOT)
				.max(sdk_raw::entities::ACCEPT_INPUT_SLOT)
				+ 1;
			let mut vtable = vec![unexpected_call as *const (); slot_count];
			let slot = |field: usize| field / size_of::<usize>();

			vtable[slot(offset_of!(
				sys::IServerEntity__bindgen_vtable,
				IServerEntity_GetCollideable
			))] = get_collideable as *const ();
			vtable[slot(offset_of!(
				sys::IServerEntity__bindgen_vtable,
				IServerEntity_GetNetworkable
			))] = get_networkable as *const ();
			vtable[slot(offset_of!(
				sys::IServerEntity__bindgen_vtable,
				IServerEntity_GetRefEHandle
			))] = get_handle as *const ();
			vtable[sdk_raw::entities::TF2_TELEPORT_SLOT] = teleport_entity as *const ();
			vtable[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = get_datamap as *const ();
			vtable[sdk_raw::entities::ACCEPT_INPUT_SLOT] = accept_input as *const ();

			let storage = leak([0usize; 64]);
			let vtable = vtable.leak();

			// Written as a pointer, so the entity's vtable keeps its provenance.
			unsafe {
				storage.cast::<*const *const ()>().write(vtable.as_ptr());
				storage
					.cast::<usize>()
					.add(MOCK_HANDLE_OFFSET / size_of::<usize>())
					.write(handle as usize);
			}

			let fields = leak(base_entity_fields());
			let map = leak(unsafe { std::mem::zeroed::<sys::datamap_t>() });

			unsafe {
				(*map).dataDesc = fields.cast();
				(*map).dataNumFields = (*fields).len() as c_int;
				(*map).dataClassName = c"CBaseEntity".as_ptr();
			}

			let mut collideable_vtable = vtable_slots::<sys::ICollideable__bindgen_vtable>();
			collideable_vtable[slot(offset_of!(
				sys::ICollideable__bindgen_vtable,
				ICollideable_GetCollisionOrigin
			))] = get_origin as *const ();
			let collideable = leak(sys::ICollideable {
				vtable_: collideable_vtable.leak().as_ptr().cast(),
			});

			let mut networkable_vtable = vtable_slots::<sys::IServerNetworkable__bindgen_vtable>();
			networkable_vtable[slot(offset_of!(
				sys::IServerNetworkable__bindgen_vtable,
				IServerNetworkable_GetClassName
			))] = get_class_name as *const ();
			networkable_vtable[slot(offset_of!(
				sys::IServerNetworkable__bindgen_vtable,
				IServerNetworkable_GetServerClass
			))] = get_server_class as *const ();
			networkable_vtable[slot(offset_of!(
				sys::IServerNetworkable__bindgen_vtable,
				IServerNetworkable_GetEdict
			))] = get_edict as *const ();
			let networkable = leak(sys::IServerNetworkable {
				vtable_: networkable_vtable.leak().as_ptr().cast(),
			});

			DATA_MAP.set(map);
			COLLIDEABLE.set(collideable);
			NETWORKABLE.set(networkable);
			ORIGIN.set(sys::Vector {
				x: 1.0,
				y: 2.0,
				z: 3.0,
			});
			TELEPORTS.set(0);
			set_networking(null_mut(), null_mut());

			Self { storage }
		}

		/// The entity's address, as the game would pass it.
		pub(crate) fn as_ptr(&mut self) -> *mut sys::CBaseEntity {
			self.storage.cast()
		}

		/// A handle to the entity, bound to this borrow of the mock.
		pub(crate) fn entity(&mut self) -> Entity<'_> {
			unsafe { Entity::from_raw(NonNull::new(self.as_ptr()).unwrap()) }
		}

		/// Reads the entity's `m_iName`.
		pub(crate) fn name(&mut self) -> sys::string_t {
			unsafe {
				self.as_ptr()
					.byte_add(MOCK_NAME_OFFSET)
					.cast::<sys::string_t>()
					.read()
			}
		}

		/// Writes the entity's `m_iEFlags`.
		pub(crate) fn set_eflags(&mut self, flags: c_int) {
			unsafe {
				self.as_ptr()
					.byte_add(MOCK_EFLAGS_OFFSET)
					.cast::<c_int>()
					.write(flags)
			};
		}

		/// Writes the entity's `m_iHammerID`.
		pub(crate) fn set_hammer_id(&mut self, id: c_int) {
			unsafe {
				self.as_ptr()
					.byte_add(MOCK_HAMMER_ID_OFFSET)
					.cast::<c_int>()
					.write(id)
			};
		}

		/// Writes the entity's `m_iName`.
		pub(crate) fn set_name(&mut self, name: sys::string_t) {
			unsafe {
				self.as_ptr()
					.byte_add(MOCK_NAME_OFFSET)
					.cast::<sys::string_t>()
					.write(name)
			};
		}
	}

	unsafe extern "C" fn accept_input(
		this: *mut sys::CBaseEntity,
		name: *const std::ffi::c_char,
		activator: *mut sys::CBaseEntity,
		caller: *mut sys::CBaseEntity,
		value: *mut sys::variant_t,
		output_id: c_int,
	) -> bool {
		let received = unsafe {
			ReceivedInput {
				target: this,
				name: CStr::from_ptr(name).to_owned(),
				activator,
				caller,
				field_type: (&raw const (*value).fieldType).read(),
				payload: value.cast::<[u8; 12]>().read(),
				string: if (&raw const (*value).fieldType).read() == sys::_fieldtypes_FIELD_STRING {
					(&raw const (*value).__bindgen_anon_1.iszVal.pszValue).read()
				} else {
					ptr::null()
				},
				handle: (&raw const (*value).eVal._base.m_Index).read(),
				output_id,
			}
		};

		// `Convert` changes the caller's copy in place.
		unsafe { (&raw mut (*value).fieldType).write(sys::_fieldtypes_FIELD_VOID) };

		INPUTS.with_borrow_mut(|inputs| inputs.push(received));
		ACCEPTS.get()
	}

	/// The fields `CBaseEntity`'s map declares that mock entities store.
	pub(crate) fn base_entity_fields() -> [sys::typedescription_t; 3] {
		let mut flags = field();

		flags.fieldType = sys::_fieldtypes_FIELD_INTEGER;
		flags.fieldName = c"m_iEFlags".as_ptr();
		flags.fieldOffset[0] = MOCK_EFLAGS_OFFSET as c_int;
		flags.fieldSizeInBytes = size_of::<c_int>() as c_int;

		let mut name = field();

		name.fieldType = sys::_fieldtypes_FIELD_STRING;
		name.fieldName = c"m_iName".as_ptr();
		name.fieldOffset[0] = MOCK_NAME_OFFSET as c_int;
		name.fieldSizeInBytes = size_of::<sys::string_t>() as c_int;

		let mut hammer_id = field();

		hammer_id.fieldType = sys::_fieldtypes_FIELD_INTEGER;
		hammer_id.fieldName = c"m_iHammerID".as_ptr();
		hammer_id.fieldOffset[0] = MOCK_HAMMER_ID_OFFSET as c_int;
		hammer_id.fieldSizeInBytes = size_of::<c_int>() as c_int;

		[flags, name, hammer_id]
	}

	/// Builds a leaked map for `class`, deriving from `base`.
	pub(crate) fn data_map(
		class: &'static CStr,
		declared: Vec<sys::typedescription_t>,
		base: *mut sys::datamap_t,
	) -> *mut sys::datamap_t {
		let count = declared.len() as c_int;
		let declared = declared.leak();
		let map = leak(unsafe { std::mem::zeroed::<sys::datamap_t>() });

		unsafe {
			(*map).dataDesc = declared.as_mut_ptr();
			(*map).dataNumFields = count;
			(*map).dataClassName = class.as_ptr();
			(*map).baseMap = base;
		}

		map
	}

	/// A zeroed field description, to be filled in.
	pub(crate) fn field() -> sys::typedescription_t {
		// SAFETY: Zero is valid for every field of `typedescription_t`.
		unsafe { std::mem::zeroed() }
	}

	unsafe extern "C" fn get_class_name(
		_: *const sys::IServerNetworkable,
	) -> *const std::ffi::c_char {
		c"tf_player".as_ptr()
	}

	unsafe extern "C" fn get_collideable(_: *mut sys::IServerEntity) -> *mut sys::ICollideable {
		COLLIDEABLE.get()
	}

	unsafe extern "C" fn get_datamap(_: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
		DATA_MAP.get()
	}

	unsafe extern "C" fn get_edict(_: *const sys::IServerNetworkable) -> *mut sys::edict_t {
		EDICT.get()
	}

	unsafe extern "C" fn get_handle(this: *const sys::IServerEntity) -> *const sys::CBaseHandle {
		unsafe { this.byte_add(MOCK_HANDLE_OFFSET).cast() }
	}

	unsafe extern "C" fn get_networkable(
		_: *mut sys::IServerEntity,
	) -> *mut sys::IServerNetworkable {
		NETWORKABLE.get()
	}

	unsafe extern "C" fn get_origin(_: *const sys::ICollideable) -> *const sys::Vector {
		ORIGIN.with(Cell::as_ptr).cast_const()
	}

	unsafe extern "C" fn get_server_class(
		_: *mut sys::IServerNetworkable,
	) -> *mut sys::ServerClass {
		SERVER_CLASS.get()
	}

	/// Leaks a value, so pointers into it stay valid however the test moves
	/// what it keeps.
	pub(crate) fn leak<T>(value: T) -> *mut T {
		Box::into_raw(Box::new(value))
	}

	/// Sets what mock entities' `AcceptInput` returns on this thread.
	pub(crate) fn set_accepts(accepts: bool) {
		ACCEPTS.set(accepts);
	}

	/// Replaces the datamap chain mock entities on this thread report.
	pub(crate) fn set_datamap(map: *mut sys::datamap_t) {
		DATA_MAP.set(map);
	}

	/// Sets the server class and edict that mock entities on this thread report.
	pub(crate) fn set_networking(class: *mut sys::ServerClass, edict: *mut sys::edict_t) {
		SERVER_CLASS.set(class);
		EDICT.set(edict);
	}

	/// Takes the inputs mock entities received on this thread.
	pub(crate) fn take_inputs() -> Vec<ReceivedInput> {
		INPUTS.take()
	}

	unsafe extern "C" fn teleport_entity(
		_: *mut sys::CBaseEntity,
		position: *const sys::Vector,
		angles: *const sys::QAngle,
		velocity: *const sys::Vector,
	) {
		assert!(angles.is_null() && velocity.is_null());
		ORIGIN.set(unsafe { *position });
		TELEPORTS.set(TELEPORTS.get() + 1);
	}

	/// How many times mock entities on this thread were teleported since the
	/// last [`MockEntity::new`].
	pub(crate) fn teleports() -> usize {
		TELEPORTS.get()
	}

	fn vtable_slots<T>() -> Vec<*const ()> {
		vec![unexpected_call as *const (); size_of::<T>() / size_of::<*const ()>()]
	}
}

#[cfg(test)]
mod tests {
	use super::test_support::{MockEntity, teleports};
	use super::*;
	use glam::Vec3;

	#[test]
	fn entities_read_native_properties_and_teleport() {
		let mut mock = MockEntity::new(5 | 7 << 16);
		let entity = mock.entity();

		assert_eq!(entity.class_name(), c"tf_player");
		assert_eq!(entity.position(), Some(Vector::new(1.0, 2.0, 3.0)));
		assert_eq!(entity.handle(), EntityHandle::from_raw(5 | 7 << 16));
		assert_eq!(entity.index(), Some(5));
		assert!(!entity.is_marked_for_deletion());

		let slot = TeleportSlot::TeamFortress2;

		// SAFETY: The mock's vtable has `teleport_entity` at TF2's `Teleport`
		// slot, here and below.
		assert_eq!(
			unsafe {
				entity.teleport(
					slot,
					Some(Vector(Vec3::new(f32::NAN, 0.0, 0.0))),
					None,
					None,
				)
			},
			Err(TeleportError::NonFinite)
		);

		unsafe { entity.teleport(slot, Some(Vector::new(4.0, 5.0, 6.0)), None, None) }.unwrap();
		assert_eq!(entity.position(), Some(Vector::new(4.0, 5.0, 6.0)));
		assert_eq!(teleports(), 1);

		mock.set_eflags(EFL_KILLME);
		let entity = mock.entity();

		assert!(entity.is_marked_for_deletion());
		assert_eq!(
			unsafe { entity.teleport(slot, Some(Vector::new(7.0, 8.0, 9.0)), None, None) },
			Err(TeleportError::MarkedForDeletion)
		);
		assert_eq!(teleports(), 1);
	}

	#[test]
	fn hammer_ids_are_read_from_the_member() {
		let mut mock = MockEntity::new(5);

		assert_eq!(mock.entity().hammer_id(), None);

		mock.set_hammer_id(1234);

		assert_eq!(mock.entity().hammer_id(), HammerId::new(1234));
		assert_eq!(mock.entity().hammer_id().map(HammerId::get), Some(1234));
		assert_eq!(HammerId::new(1234).unwrap().to_string(), "1234");
		assert_eq!(HammerId::new(0), None);
	}

	#[test]
	fn handles_split_into_slot_and_serial_number() {
		let handle = EntityHandle::from_raw(42 | 3 << 16);

		assert_eq!(handle.index(), Some(42));
		assert_eq!(handle.serial_number(), 3);
		assert_eq!(handle.to_string(), "42:3");
		assert_eq!(EntityHandle::INVALID.index(), None);
		assert_eq!(EntityHandle::INVALID.to_string(), "invalid");
	}
}
