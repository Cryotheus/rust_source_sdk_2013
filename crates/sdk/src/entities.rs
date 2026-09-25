//! Entities in the server's entity list.
//!
//! [`ServerTools`](crate::interfaces::ServerTools) finds entities, including
//! server-only ones. Properties use the native entity interfaces instead of
//! key values, and networked variables are reached through
//! [`NetProp`](crate::datatables::NetProp).

use crate::datatables::ServerClass;
use crate::edicts::Edict;
use crate::ffi::{NotThreadSafe, borrow_cstr, vcall};
use crate::math::{QAngle, Vector};
use std::ffi::{CStr, c_int};
use std::fmt::{self, Display, Formatter};
use std::marker::PhantomData;
use std::mem::{align_of, size_of, transmute};
use std::ptr::{self, NonNull};
use std::sync::OnceLock;

/// `EFL_KILLME` from `game/shared/shareddefs.h`.
const EFL_KILLME: c_int = 1 << 0;

/// An entity cannot be teleported as requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TeleportError {
	#[error("the destination contains a non-finite component")]
	NonFinite,

	#[error("the entity is marked for deletion")]
	MarkedForDeletion,
}

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

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::CBaseEntity {
		self.raw.as_ptr()
	}

	/// The handle that identifies this entity across frames.
	#[doc(alias = "GetRefEHandle")]
	pub fn handle(self) -> EntityHandle {
		// SAFETY: `CBaseEntity`'s primary base is `IServerEntity`, which derives
		// from `IServerUnknown`, so the pointers coincide.
		let handle = unsafe { vcall!(self.unknown() => IServerUnknown_GetRefEHandle()) };

		// SAFETY: The handle is a member of the live entity.
		NonNull::new(handle.cast_mut()).map_or(EntityHandle::INVALID, |handle| {
			EntityHandle(unsafe { (&raw const (*handle.as_ptr()).m_Index).read() })
		})
	}

	/// The entity's slot in the entity list, which is its edict index if it is
	/// networked.
	#[doc(alias = "entindex")]
	pub fn index(self) -> Option<usize> {
		self.handle().index()
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

	/// The entity's class name, such as `tf_player`.
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

	/// The class describing how the entity is networked.
	#[doc(alias = "GetServerClass")]
	pub fn server_class(self) -> Option<ServerClass<'s>> {
		let networkable = self.networkable()?;

		// SAFETY: The networkable belongs to the live entity.
		let class = unsafe { vcall!(networkable.as_ptr() => IServerNetworkable_GetServerClass()) };

		// SAFETY: Server classes are statics of the game DLL.
		NonNull::new(class).map(|class| unsafe { ServerClass::from_raw(class) })
	}

	/// Reads the absolute origin without key-value conversion.
	///
	/// Source's collision property returns its owner's `GetAbsOrigin()` here.
	#[doc(alias = "GetAbsOrigin")]
	pub fn position(self) -> Option<Vector> {
		// SAFETY: As for `handle`.
		let collideable =
			NonNull::new(unsafe { vcall!(self.unknown() => IServerUnknown_GetCollideable()) })?;

		// SAFETY: The collideable belongs to the live entity.
		let origin = NonNull::new(
			unsafe { vcall!(collideable.as_ptr() => ICollideable_GetCollisionOrigin()) }.cast_mut(),
		)?;

		// SAFETY: The origin is a member of the live entity, copied immediately.
		Some(unsafe { origin.as_ptr().read() }.into())
	}

	/// Whether Source has marked this entity for deferred deletion.
	#[doc(alias = "IsMarkedForDeletion")]
	pub fn is_marked_for_deletion(self) -> bool {
		static EFLAGS_OFFSET: OnceLock<usize> = OnceLock::new();

		let offset = *EFLAGS_OFFSET.get_or_init(|| {
			find_eflags_offset(self)
				.expect("Source's CBaseEntity datamap does not contain m_iEFlags")
		});

		// SAFETY: The offset was validated against the entity datamap, which
		// every entity shares through its `CBaseEntity` base.
		let flags = unsafe { self.as_ptr().byte_add(offset).cast::<c_int>().read() };

		flags & EFL_KILLME != 0
	}

	/// Moves the entity through Source's `Teleport` method, found at `slot` of
	/// its vtable. Each argument left as `None` is unchanged.
	pub(crate) fn teleport(
		self,
		slot: usize,
		origin: Option<Vector>,
		angles: Option<QAngle>,
		velocity: Option<Vector>,
	) -> Result<(), TeleportError> {
		type Teleport = unsafe extern "C" fn(
			*mut sys::CBaseEntity,
			*const sys::Vector,
			*const sys::QAngle,
			*const sys::Vector,
		);

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

		// SAFETY: `Game` selected the slot of `Teleport` in the game's vtable.
		let teleport: Teleport = unsafe { transmute(self.vtable_slot(slot)) };

		// SAFETY: The entity is live and every pointer is null or a local.
		unsafe {
			teleport(
				self.as_ptr(),
				origin.as_ref().map_or(ptr::null(), ptr::from_ref),
				angles.as_ref().map_or(ptr::null(), ptr::from_ref),
				velocity.as_ref().map_or(ptr::null(), ptr::from_ref),
			)
		};

		Ok(())
	}

	fn unknown(self) -> *mut sys::IServerUnknown {
		self.raw.as_ptr().cast()
	}

	fn networkable(self) -> Option<NonNull<sys::IServerNetworkable>> {
		// SAFETY: As for `handle`.
		NonNull::new(unsafe { vcall!(self.unknown() => IServerUnknown_GetNetworkable()) })
	}

	fn vtable_slot(self, slot: usize) -> *const () {
		// SAFETY: `CBaseEntity` has `IServerEntity` as its primary, zero-offset
		// base, so the entity starts with its primary vtable pointer.
		let vtable = unsafe { self.as_ptr().cast::<*const *const ()>().read() };

		// SAFETY: Callers only pass slots within `CBaseEntity`'s vtable.
		unsafe { vtable.add(slot).read() }
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
#[repr(transparent)]
pub struct EntityHandle(u32);

impl EntityHandle {
	/// `INVALID_EHANDLE_INDEX`, which refers to no entity.
	pub const INVALID: Self = Self(u32::MAX);

	/// `NUM_ENT_ENTRIES`, the number of slots in the entity list.
	pub const SLOTS: usize = 1 << 13;

	/// `NUM_SERIAL_NUM_SHIFT_BITS`.
	const SERIAL_NUMBER_SHIFT: u32 = 16;

	pub const fn from_raw(raw: u32) -> Self {
		Self(raw)
	}

	pub const fn to_raw(self) -> u32 {
		self.0
	}

	pub const fn is_valid(self) -> bool {
		self.0 != Self::INVALID.0
	}

	/// The entity's slot in the entity list, or `None` for an invalid handle.
	#[doc(alias = "GetEntryIndex")]
	pub const fn index(self) -> Option<usize> {
		if self.is_valid() {
			Some((self.0 & ((1 << Self::SERIAL_NUMBER_SHIFT) - 1)) as usize)
		} else {
			None
		}
	}

	#[doc(alias = "GetSerialNumber")]
	pub const fn serial_number(self) -> u32 {
		self.0 >> Self::SERIAL_NUMBER_SHIFT
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

fn find_eflags_offset(entity: Entity<'_>) -> Option<usize> {
	type GetDataDescMap = unsafe extern "C" fn(*mut sys::CBaseEntity) -> *mut sys::datamap_t;

	// SAFETY: `GetDataDescMap` occupies this slot under both ABIs.
	let get_map: GetDataDescMap =
		unsafe { transmute(entity.vtable_slot(sys::CBASEENTITY_DATAMAP_VTABLE_SLOT)) };

	// SAFETY: The entity is live.
	let mut map = unsafe { get_map(entity.as_ptr()) };

	for _ in 0..64 {
		let map_ptr = NonNull::new(map)?;

		// SAFETY: Datamaps are immutable statics of the game DLL.
		let map_ref = unsafe { map_ptr.as_ref() };
		let is_base_entity = !map_ref.dataClassName.is_null()
			&& unsafe { CStr::from_ptr(map_ref.dataClassName) } == c"CBaseEntity";

		if is_base_entity
			&& (0..=4096).contains(&map_ref.dataNumFields)
			&& !map_ref.dataDesc.is_null()
		{
			// SAFETY: As for `map_ref`.
			let fields = unsafe {
				std::slice::from_raw_parts(map_ref.dataDesc, map_ref.dataNumFields as usize)
			};

			for field in fields {
				if field.fieldType == sys::_fieldtypes_FIELD_INTEGER
					&& field.fieldSizeInBytes == size_of::<c_int>() as c_int
					&& !field.fieldName.is_null()
				{
					// SAFETY: As for `map_ref`.
					let name = unsafe { CStr::from_ptr(field.fieldName) };

					if name == c"m_iEFlags" {
						let offset = usize::try_from(field.fieldOffset[0]).ok()?;

						return (offset < 8192 && offset.is_multiple_of(align_of::<c_int>()))
							.then_some(offset);
					}
				}
			}
		}

		map = map_ref.baseMap;
	}

	None
}

#[cfg(test)]
pub(crate) mod test_support {
	use super::*;
	use crate::ffi::test_support::unexpected_call;
	use std::cell::Cell;
	use std::mem::offset_of;
	use std::ptr::null_mut;

	pub(crate) const MOCK_EFLAGS_OFFSET: usize = 32;
	const MOCK_HANDLE_OFFSET: usize = 40;

	thread_local! {
		static COLLIDEABLE: Cell<*mut sys::ICollideable> = const { Cell::new(null_mut()) };
		static NETWORKABLE: Cell<*mut sys::IServerNetworkable> = const { Cell::new(null_mut()) };
		static ORIGIN: Cell<sys::Vector> = const { Cell::new(sys::Vector { x: 1.0, y: 2.0, z: 3.0 }) };
		static TELEPORTS: Cell<usize> = const { Cell::new(0) };
		static DATA_MAP: Cell<*mut sys::datamap_t> = const { Cell::new(null_mut()) };
		static SERVER_CLASS: Cell<*mut sys::ServerClass> = const { Cell::new(null_mut()) };
		static EDICT: Cell<*mut sys::edict_t> = const { Cell::new(null_mut()) };
	}

	/// Sets the server class and edict that mock entities on this thread report.
	pub(crate) fn set_networking(class: *mut sys::ServerClass, edict: *mut sys::edict_t) {
		SERVER_CLASS.set(class);
		EDICT.set(edict);
	}

	unsafe extern "C" fn get_server_class(
		_: *mut sys::IServerNetworkable,
	) -> *mut sys::ServerClass {
		SERVER_CLASS.get()
	}

	unsafe extern "C" fn get_edict(_: *const sys::IServerNetworkable) -> *mut sys::edict_t {
		EDICT.get()
	}

	unsafe extern "C" fn get_datamap(_: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
		DATA_MAP.get()
	}

	unsafe extern "C" fn get_collideable(_: *mut sys::IServerUnknown) -> *mut sys::ICollideable {
		COLLIDEABLE.get()
	}

	unsafe extern "C" fn get_networkable(
		_: *mut sys::IServerUnknown,
	) -> *mut sys::IServerNetworkable {
		NETWORKABLE.get()
	}

	unsafe extern "C" fn get_handle(this: *const sys::IServerUnknown) -> *const sys::CBaseHandle {
		unsafe { this.byte_add(MOCK_HANDLE_OFFSET).cast() }
	}

	unsafe extern "C" fn get_origin(_: *const sys::ICollideable) -> *const sys::Vector {
		ORIGIN.with(Cell::as_ptr).cast_const()
	}

	unsafe extern "C" fn get_class_name(
		_: *const sys::IServerNetworkable,
	) -> *const std::ffi::c_char {
		c"tf_player".as_ptr()
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

	fn vtable_slots<T>() -> Box<[usize]> {
		vec![unexpected_call as *const () as usize; size_of::<T>() / size_of::<usize>()]
			.into_boxed_slice()
	}

	/// An entity with a class name, origin, datamap, handle, and TF2 `Teleport`,
	/// whose teleports are counted by [`teleports`].
	pub(crate) struct MockEntity {
		storage: Box<[usize]>,
		_vtable: Box<[usize]>,
		_collideable_vtable: Box<[usize]>,
		_collideable: Box<sys::ICollideable>,
		_networkable_vtable: Box<[usize]>,
		_networkable: Box<sys::IServerNetworkable>,
		_field: Box<sys::typedescription_t>,
		_map: Box<sys::datamap_t>,
	}

	impl MockEntity {
		pub(crate) fn new(handle: u32) -> Self {
			let slot_count = sys::CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT
				.max(sys::CBASEENTITY_DATAMAP_VTABLE_SLOT)
				+ 1;
			let mut vtable =
				vec![unexpected_call as *const () as usize; slot_count].into_boxed_slice();
			let slot = |field: usize| field / size_of::<usize>();

			vtable[slot(offset_of!(
				sys::IServerUnknown__bindgen_vtable,
				IServerUnknown_GetCollideable
			))] = get_collideable as *const () as usize;
			vtable[slot(offset_of!(
				sys::IServerUnknown__bindgen_vtable,
				IServerUnknown_GetNetworkable
			))] = get_networkable as *const () as usize;
			vtable[slot(offset_of!(
				sys::IServerUnknown__bindgen_vtable,
				IServerUnknown_GetRefEHandle
			))] = get_handle as *const () as usize;
			vtable[sys::CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT] =
				teleport_entity as *const () as usize;
			vtable[sys::CBASEENTITY_DATAMAP_VTABLE_SLOT] = get_datamap as *const () as usize;

			let mut storage = vec![0usize; 64].into_boxed_slice();
			storage[0] = vtable.as_ptr() as usize;
			storage[MOCK_HANDLE_OFFSET / size_of::<usize>()] = handle as usize;

			let mut field = Box::new(unsafe { std::mem::zeroed::<sys::typedescription_t>() });
			field.fieldType = sys::_fieldtypes_FIELD_INTEGER;
			field.fieldName = c"m_iEFlags".as_ptr();
			field.fieldOffset[0] = MOCK_EFLAGS_OFFSET as c_int;
			field.fieldSizeInBytes = size_of::<c_int>() as c_int;

			let mut map = Box::new(unsafe { std::mem::zeroed::<sys::datamap_t>() });
			map.dataDesc = &raw mut *field;
			map.dataNumFields = 1;
			map.dataClassName = c"CBaseEntity".as_ptr();

			let mut collideable_vtable = vtable_slots::<sys::ICollideable__bindgen_vtable>();
			collideable_vtable[slot(offset_of!(
				sys::ICollideable__bindgen_vtable,
				ICollideable_GetCollisionOrigin
			))] = get_origin as *const () as usize;
			let mut collideable = Box::new(sys::ICollideable {
				vtable_: collideable_vtable.as_ptr().cast(),
			});

			let mut networkable_vtable = vtable_slots::<sys::IServerNetworkable__bindgen_vtable>();
			networkable_vtable[slot(offset_of!(
				sys::IServerNetworkable__bindgen_vtable,
				IServerNetworkable_GetClassName
			))] = get_class_name as *const () as usize;
			networkable_vtable[slot(offset_of!(
				sys::IServerNetworkable__bindgen_vtable,
				IServerNetworkable_GetServerClass
			))] = get_server_class as *const () as usize;
			networkable_vtable[slot(offset_of!(
				sys::IServerNetworkable__bindgen_vtable,
				IServerNetworkable_GetEdict
			))] = get_edict as *const () as usize;
			let mut networkable = Box::new(sys::IServerNetworkable {
				vtable_: networkable_vtable.as_ptr().cast(),
			});

			DATA_MAP.set(&raw mut *map);
			COLLIDEABLE.set(&raw mut *collideable);
			NETWORKABLE.set(&raw mut *networkable);
			ORIGIN.set(sys::Vector {
				x: 1.0,
				y: 2.0,
				z: 3.0,
			});
			TELEPORTS.set(0);
			set_networking(null_mut(), null_mut());

			Self {
				storage,
				_vtable: vtable,
				_collideable_vtable: collideable_vtable,
				_collideable: collideable,
				_networkable_vtable: networkable_vtable,
				_networkable: networkable,
				_field: field,
				_map: map,
			}
		}

		pub(crate) fn as_ptr(&mut self) -> *mut sys::CBaseEntity {
			self.storage.as_mut_ptr().cast()
		}

		pub(crate) fn entity(&mut self) -> Entity<'_> {
			unsafe { Entity::from_raw(NonNull::new(self.as_ptr()).unwrap()) }
		}

		pub(crate) fn set_eflags(&mut self, flags: c_int) {
			unsafe {
				self.as_ptr()
					.byte_add(MOCK_EFLAGS_OFFSET)
					.cast::<c_int>()
					.write(flags)
			};
		}
	}

	pub(crate) fn teleports() -> usize {
		TELEPORTS.get()
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

		let slot = sys::CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT;

		assert_eq!(
			entity.teleport(
				slot,
				Some(Vector(Vec3::new(f32::NAN, 0.0, 0.0))),
				None,
				None
			),
			Err(TeleportError::NonFinite)
		);

		entity
			.teleport(slot, Some(Vector::new(4.0, 5.0, 6.0)), None, None)
			.unwrap();
		assert_eq!(entity.position(), Some(Vector::new(4.0, 5.0, 6.0)));
		assert_eq!(teleports(), 1);

		mock.set_eflags(EFL_KILLME);
		let entity = mock.entity();

		assert!(entity.is_marked_for_deletion());
		assert_eq!(
			entity.teleport(slot, Some(Vector::new(7.0, 8.0, 9.0)), None, None),
			Err(TeleportError::MarkedForDeletion)
		);
		assert_eq!(teleports(), 1);
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
