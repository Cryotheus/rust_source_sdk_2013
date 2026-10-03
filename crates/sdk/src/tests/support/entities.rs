//! A mock entity, as the game's `CBaseEntity` answers the engine wrappers, and
//! the fields its data description map declares.

use super::leak;
use sdk_raw::test_support::entities::field;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int};
use std::mem::offset_of;
use std::ptr::{self, null_mut};

/// Where mock entities store `m_iEFlags`, as their datamap declares.
pub const MOCK_EFLAGS_OFFSET: usize = 32;

/// Where mock entities store `m_iHammerID`, as their datamap declares.
const MOCK_HAMMER_ID_OFFSET: usize = 56;

/// Where mock entities store the handle `GetRefEHandle` points to.
const MOCK_HANDLE_OFFSET: usize = 40;

/// Where mock entities store `m_iName`, as their datamap declares.
pub const MOCK_NAME_OFFSET: usize = 48;

/// An input a mock entity's `AcceptInput` received.
#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedInput {
	/// The entity the input was sent to.
	pub target: *mut sys::CBaseEntity,
	/// The input's name.
	pub name: CString,
	/// The entity that caused the input.
	pub activator: *mut sys::CBaseEntity,
	/// The entity that sent the input.
	pub caller: *mut sys::CBaseEntity,
	/// The value's type.
	pub field_type: sys::fieldtype_t,
	/// The bytes every union member covers.
	pub payload: [u8; 12],
	/// The string pointer, read as a pointer, for string values.
	pub string: *const c_char,
	/// The handle member's index, read whatever the type.
	pub handle: u32,
	/// The output the input came from.
	pub output_id: c_int,
}

thread_local! {
	static ACCEPTS: Cell<bool> = const { Cell::new(true) };
	static COLLIDEABLE: Cell<*mut sys::ICollideable> = const { Cell::new(null_mut()) };
	static DATA_MAP: Cell<*mut sys::datamap_t> = const { Cell::new(null_mut()) };
	static EDICT: Cell<*mut sys::edict_t> = const { Cell::new(null_mut()) };
	static INPUTS: RefCell<Vec<ReceivedInput>> = const { RefCell::new(Vec::new()) };
	static NETWORKABLE: Cell<*mut sys::IServerNetworkable> = const { Cell::new(null_mut()) };
	static ORIGIN: Cell<sys::Vector> = const { Cell::new(sys::Vector { x: 1.0, y: 2.0, z: 3.0 }) };
	static SERVER_CLASS: Cell<*mut sys::ServerClass> = const { Cell::new(null_mut()) };
	static TELEPORTS: Cell<usize> = const { Cell::new(0) };
}

/// An entity with a class name, origin, datamap, handle, and TF2 `Teleport`,
/// whose teleports are counted by [`teleports`].
///
/// For tests only. Its allocations are leaked, and only reached through raw
/// pointers, like the engine's objects.
pub struct MockEntity {
	storage: *mut [usize; 64],
}

impl MockEntity {
	/// Builds an entity whose handle is `handle`, and resets the datamap,
	/// origin, teleport count, server class, and edict that mock entities on
	/// this thread report.
	pub fn new(handle: u32) -> Self {
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

		// SAFETY: The storage is a leaked allocation of 64 words, which holds
		// the vtable pointer at its start and the handle at its offset. The
		// vtable is written as a pointer, so it keeps its provenance.
		unsafe {
			storage.cast::<*const *const ()>().write(vtable.as_ptr());
			storage
				.cast::<usize>()
				.add(MOCK_HANDLE_OFFSET / size_of::<usize>())
				.write(handle as usize);
		}

		let fields = Vec::from(base_entity_fields());
		let map = sdk_raw::test_support::entities::data_map(c"CBaseEntity", fields, null_mut());

		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes a slot
		// of the vtable being built.
		let collideable_vtable = unsafe {
			mock_vtable::<sys::ICollideable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).ICollideable_GetCollisionOrigin).write(get_origin);
				},
			)
		};
		let collideable = leak(sys::ICollideable {
			vtable_: Box::leak(collideable_vtable),
		});

		// SAFETY: As for the collideable's vtable.
		let networkable_vtable = unsafe {
			mock_vtable::<sys::IServerNetworkable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerNetworkable_GetClassName).write(get_class_name);
					(&raw mut (*vtable).IServerNetworkable_GetServerClass).write(get_server_class);
					(&raw mut (*vtable).IServerNetworkable_GetEdict).write(get_edict);
				},
			)
		};
		let networkable = leak(sys::IServerNetworkable {
			vtable_: Box::leak(networkable_vtable),
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
	pub fn as_ptr(&mut self) -> *mut sys::CBaseEntity {
		self.storage.cast()
	}

	/// A handle to the entity, bound to this borrow of the mock.
	#[cfg(test)]
	pub(crate) fn entity(&mut self) -> crate::entities::Entity<'_> {
		let entity = std::ptr::NonNull::new(self.as_ptr()).unwrap();

		// SAFETY: The mock is leaked, so it outlives the borrow, and its vtable
		// answers what the wrappers call of a `CBaseEntity`.
		unsafe { crate::entities::Entity::from_raw(entity) }
	}

	/// Reads the entity's `m_iName`.
	pub fn name(&mut self) -> sys::string_t {
		// SAFETY: The storage holds the name at its offset, within its 64
		// words.
		unsafe {
			self.as_ptr()
				.byte_add(MOCK_NAME_OFFSET)
				.cast::<sys::string_t>()
				.read()
		}
	}

	/// Writes the entity's `m_iEFlags`.
	pub fn set_eflags(&mut self, flags: c_int) {
		// SAFETY: The storage holds the flags at their offset, within its 64
		// words.
		unsafe {
			self.as_ptr()
				.byte_add(MOCK_EFLAGS_OFFSET)
				.cast::<c_int>()
				.write(flags)
		};
	}

	/// Writes the entity's `m_iHammerID`.
	pub fn set_hammer_id(&mut self, id: c_int) {
		// SAFETY: The storage holds the ID at its offset, within its 64 words.
		unsafe {
			self.as_ptr()
				.byte_add(MOCK_HAMMER_ID_OFFSET)
				.cast::<c_int>()
				.write(id)
		};
	}

	/// Writes the entity's `m_iName`.
	pub fn set_name(&mut self, name: sys::string_t) {
		// SAFETY: As for `name`.
		unsafe {
			self.as_ptr()
				.byte_add(MOCK_NAME_OFFSET)
				.cast::<sys::string_t>()
				.write(name)
		};
	}
}

/// `CBaseEntity::AcceptInput`, which records the input for [`take_inputs`]
/// and converts the caller's value to `FIELD_VOID`, as `variant_t::Convert`
/// does.
unsafe extern "C" fn accept_input(
	this: *mut sys::CBaseEntity,
	name: *const c_char,
	activator: *mut sys::CBaseEntity,
	caller: *mut sys::CBaseEntity,
	value: *mut sys::variant_t,
	output_id: c_int,
) -> bool {
	// SAFETY: The wrappers pass a NUL-terminated name and a live `variant_t`,
	// whose string member is read only for string values.
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

	// SAFETY: `Convert` changes the caller's copy in place, which is live.
	unsafe { (&raw mut (*value).fieldType).write(sys::_fieldtypes_FIELD_VOID) };

	INPUTS.with_borrow_mut(|inputs| inputs.push(received));
	ACCEPTS.get()
}

/// The fields `CBaseEntity`'s map declares that mock entities store:
/// `m_iEFlags`, `m_iName` and `m_iHammerID`.
///
/// For tests only.
pub fn base_entity_fields() -> [sys::typedescription_t; 3] {
	let mut flags = field(
		c"m_iEFlags",
		sys::_fieldtypes_FIELD_INTEGER,
		MOCK_EFLAGS_OFFSET,
	);

	flags.fieldSizeInBytes = size_of::<c_int>() as c_int;

	let mut name = field(c"m_iName", sys::_fieldtypes_FIELD_STRING, MOCK_NAME_OFFSET);

	name.fieldSizeInBytes = size_of::<sys::string_t>() as c_int;

	let mut hammer_id = field(
		c"m_iHammerID",
		sys::_fieldtypes_FIELD_INTEGER,
		MOCK_HAMMER_ID_OFFSET,
	);

	hammer_id.fieldSizeInBytes = size_of::<c_int>() as c_int;

	[flags, name, hammer_id]
}

/// `IServerNetworkable::GetClassName`, which names every mock entity a
/// player.
unsafe extern "C" fn get_class_name(_: *const sys::IServerNetworkable) -> *const c_char {
	c"tf_player".as_ptr()
}

unsafe extern "C" fn get_collideable(_: *mut sys::IServerEntity) -> *mut sys::ICollideable {
	COLLIDEABLE.get()
}

/// `CBaseEntity::GetDataDescMap`, which returns the map [`set_datamap`] or
/// the last [`MockEntity::new`] set on this thread.
///
/// # Safety
///
/// None: it reads no argument. It is `unsafe` to fit the vtable slot.
pub unsafe extern "C" fn get_datamap(_: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	DATA_MAP.get()
}

unsafe extern "C" fn get_edict(_: *const sys::IServerNetworkable) -> *mut sys::edict_t {
	EDICT.get()
}

unsafe extern "C" fn get_handle(this: *const sys::IServerEntity) -> *const sys::CBaseHandle {
	// SAFETY: Every mock entity stores its handle at its offset.
	unsafe { this.byte_add(MOCK_HANDLE_OFFSET).cast() }
}

unsafe extern "C" fn get_networkable(_: *mut sys::IServerEntity) -> *mut sys::IServerNetworkable {
	NETWORKABLE.get()
}

unsafe extern "C" fn get_origin(_: *const sys::ICollideable) -> *const sys::Vector {
	ORIGIN.with(Cell::as_ptr).cast_const()
}

unsafe extern "C" fn get_server_class(_: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	SERVER_CLASS.get()
}

/// Sets what mock entities' `AcceptInput` returns on this thread.
///
/// For tests only.
pub fn set_accepts(accepts: bool) {
	ACCEPTS.set(accepts);
}

/// Replaces the datamap chain mock entities, and [`get_datamap`], report on
/// this thread.
///
/// For tests only. The map must stay alive while it is reported.
pub fn set_datamap(map: *mut sys::datamap_t) {
	DATA_MAP.set(map);
}

/// Sets the server class and edict that mock entities on this thread report.
///
/// For tests only. Both must stay alive while they are reported.
pub fn set_networking(class: *mut sys::ServerClass, edict: *mut sys::edict_t) {
	SERVER_CLASS.set(class);
	EDICT.set(edict);
}

/// Takes the inputs mock entities received on this thread.
///
/// For tests only.
pub fn take_inputs() -> Vec<ReceivedInput> {
	INPUTS.take()
}

/// TF2's `CBaseEntity::Teleport`, which moves the mock entity's origin and
/// counts the teleport.
unsafe extern "C" fn teleport_entity(
	_: *mut sys::CBaseEntity,
	position: *const sys::Vector,
	angles: *const sys::QAngle,
	velocity: *const sys::Vector,
) {
	assert!(angles.is_null() && velocity.is_null());

	// SAFETY: The wrappers pass a live destination.
	ORIGIN.set(unsafe { *position });
	TELEPORTS.set(TELEPORTS.get() + 1);
}

/// How many times mock entities on this thread were teleported since the
/// last [`MockEntity::new`].
///
/// For tests only.
pub fn teleports() -> usize {
	TELEPORTS.get()
}
