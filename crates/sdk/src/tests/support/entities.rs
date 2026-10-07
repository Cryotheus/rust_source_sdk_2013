//! A mock entity, as the game's `CBaseEntity` answers the engine wrappers, and
//! the fields its data description map declares.

use super::leak;
use sdk_raw::players::LIFE_ALIVE;
use sdk_raw::test_support::entities::field;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::alloc::{Layout, alloc_zeroed, handle_alloc_error};
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

/// Where mock entities store `m_iHealth`, as [`health_fields`] declares.
pub const MOCK_HEALTH_OFFSET: usize = 64;

/// Where mock entities store `m_lifeState`, as [`health_fields`] declares.
pub const MOCK_LIFE_STATE_OFFSET: usize = 72;

/// What mock entities' `GetMaxHealth` adds to their `m_iMaxHealth`, as TF2's
/// players add their attributes' bonuses to their class's maximum.
pub const MOCK_MAX_HEALTH_BONUS: c_int = 25;

/// Where mock entities store `m_iMaxHealth`, as [`health_fields`] declares.
pub const MOCK_MAX_HEALTH_OFFSET: usize = 68;

/// Where mock entities store `m_iName`, as their datamap declares.
pub const MOCK_NAME_OFFSET: usize = 48;

/// Where mock entities store their [`MockState`], whose members
/// [`state_fields`] declares.
pub const MOCK_STATE_OFFSET: usize = 128;

/// Where mock entities store `m_takedamage`, as [`health_fields`] declares.
pub const MOCK_TAKE_DAMAGE_OFFSET: usize = 73;

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
	static COLLISION_GROUP: Cell<c_int> = const { Cell::new(sdk_raw::entities::COLLISION_GROUP_NONE) };
	static DATA_MAP: Cell<*mut sys::datamap_t> = const { Cell::new(null_mut()) };
	static EDICT: Cell<*mut sys::edict_t> = const { Cell::new(null_mut()) };
	static INPUTS: RefCell<Vec<ReceivedInput>> = const { RefCell::new(Vec::new()) };
	static NETWORKABLE: Cell<*mut sys::IServerNetworkable> = const { Cell::new(null_mut()) };
	static ORIGIN: Cell<sys::Vector> = const { Cell::new(sys::Vector { x: 1.0, y: 2.0, z: 3.0 }) };
	static SERVER_CLASS: Cell<*mut sys::ServerClass> = const { Cell::new(null_mut()) };
	static TAKE_HEALTH_CALLS: RefCell<Vec<(f32, c_int)>> = const { RefCell::new(Vec::new()) };
	static TELEPORTS: Cell<usize> = const { Cell::new(0) };
	static TRANSMIT_STATE_UPDATES: Cell<usize> = const { Cell::new(0) };
}

/// An entity with a class name, origin, datamap, handle, TF2 `Teleport`,
/// whose teleports are counted by [`teleports`], health methods, whose
/// `TakeHealth` calls [`take_health_calls`] records, and
/// `UpdateTransmitState`, whose calls [`transmit_state_updates`] counts.
///
/// For tests only. Its allocations are leaked, and only reached through raw
/// pointers, like the engine's objects.
pub struct MockEntity {
	storage: *mut usize,
}

/// Members of `CBaseEntity` that mock entities store from
/// [`MOCK_STATE_OFFSET`] on, as [`state_fields`] declares them, with the
/// types their datamap declares.
///
/// For tests only.
#[repr(C)]
pub struct MockState {
	/// `m_fEffects`.
	pub effects: c_int,

	/// `m_fFlags`.
	pub flags: c_int,

	/// `m_flFriction`.
	pub friction: f32,

	/// `m_flGravity`.
	pub gravity: f32,

	/// `m_hMoveParent`.
	pub move_parent: u32,

	/// `m_iTeamNum`.
	pub team: c_int,

	/// `m_spawnflags`.
	pub spawn_flags: c_int,

	/// `m_clrRender`.
	pub render_color: sys::color32,

	/// `m_nModelIndex`.
	pub model_index: i16,

	/// `m_MoveType`.
	pub move_type: u8,

	/// `m_nRenderMode`.
	pub render_mode: u8,

	/// `m_nTransmitStateOwnedCounter`.
	pub transmit_state_owners: u8,

	/// `m_iParent`.
	pub parent_name: sys::string_t,

	/// `m_ModelName`.
	pub model_name: sys::string_t,

	/// `m_vecAbsVelocity`.
	pub abs_velocity: sys::Vector,

	/// `m_vecVelocity`, 4 bytes past a multiple of 8, as the game's
	/// `m_vecAbsVelocity` lies.
	pub velocity: sys::Vector,
}

impl MockEntity {
	/// Builds an entity whose handle is `handle`, and resets the datamap,
	/// origin, collision group, teleport count, transmit state update count,
	/// `TakeHealth` calls, server class, and edict that mock entities on this
	/// thread report.
	pub fn new(handle: u32) -> Self {
		Self::with_layout(handle, Layout::new::<[usize; 64]>())
	}

	/// Builds an entity as [`new`](Self::new) does, in zeroed storage of
	/// `layout`, such as that of a class whose generated members a wrapper
	/// writes, padded to at least the 64 words a mock entity uses.
	pub fn with_layout(handle: u32, layout: Layout) -> Self {
		let slot_count = sdk_raw::entities::TF2_TELEPORT_SLOT
			.max(sdk_raw::entities::GET_DATA_DESC_MAP_SLOT)
			.max(sdk_raw::entities::ACCEPT_INPUT_SLOT)
			.max(sdk_raw::entities::health::TF2_GET_MAX_HEALTH_SLOT)
			.max(sdk_raw::entities::health::TAKE_HEALTH_SLOT)
			.max(sdk_raw::entities::health::IS_ALIVE_SLOT)
			.max(sdk_raw::transmit::UPDATE_TRANSMIT_STATE_SLOT)
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
		vtable[sdk_raw::entities::health::TF2_GET_MAX_HEALTH_SLOT] = get_max_health as *const ();
		vtable[sdk_raw::entities::health::TAKE_HEALTH_SLOT] = take_health as *const ();
		vtable[sdk_raw::entities::health::IS_ALIVE_SLOT] = is_alive as *const ();
		vtable[sdk_raw::transmit::UPDATE_TRANSMIT_STATE_SLOT] = update_transmit_state as *const ();

		let layout = Layout::from_size_align(
			layout.size().max(size_of::<[usize; 64]>()),
			layout.align().max(align_of::<usize>()),
		)
		.unwrap();

		// SAFETY: The layout is at least 64 words long.
		let storage = unsafe { alloc_zeroed(layout) }.cast::<usize>();

		if storage.is_null() {
			handle_alloc_error(layout);
		}

		let vtable = vtable.leak();

		// SAFETY: The storage is a leaked allocation of at least 64 words,
		// which holds the vtable pointer at its start and the handle at its
		// offset. The vtable is written as a pointer, so it keeps its
		// provenance.
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
					(&raw mut (*vtable).ICollideable_GetCollisionGroup).write(get_collision_group);
					(&raw mut (*vtable).ICollideable_WorldSpaceSurroundingBounds)
						.write(get_surrounding_bounds);
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
		COLLISION_GROUP.set(sdk_raw::entities::COLLISION_GROUP_NONE);
		NETWORKABLE.set(networkable);
		ORIGIN.set(sys::Vector {
			x: 1.0,
			y: 2.0,
			z: 3.0,
		});
		TELEPORTS.set(0);
		TRANSMIT_STATE_UPDATES.set(0);
		TAKE_HEALTH_CALLS.take();
		set_networking(null_mut(), null_mut());

		Self { storage }
	}

	/// The entity's address, as the game would pass it.
	pub fn as_ptr(&mut self) -> *mut sys::CBaseEntity {
		self.storage.cast()
	}

	/// Reads the byte at `offset` into the entity.
	///
	/// # Panics
	///
	/// If `offset` lies beyond the 64 words every mock entity has.
	pub fn byte(&mut self, offset: usize) -> u8 {
		assert!(offset < size_of::<[usize; 64]>());

		// SAFETY: The storage holds at least 64 words.
		unsafe { self.as_ptr().byte_add(offset).cast::<u8>().read() }
	}

	/// A handle to the entity, bound to this borrow of the mock.
	#[cfg(test)]
	pub(crate) fn entity(&mut self) -> crate::entities::Entity<'_> {
		let entity = std::ptr::NonNull::new(self.as_ptr()).unwrap();

		// SAFETY: The mock is leaked, so it outlives the borrow, and its vtable
		// answers what the wrappers call of a `CBaseEntity`.
		unsafe { crate::entities::Entity::from_raw(entity) }
	}

	/// Reads the `int` at `offset` into the entity.
	///
	/// # Panics
	///
	/// As for [`byte`](Self::byte), or if `offset` is not aligned for an
	/// `int`.
	pub fn int(&mut self, offset: usize) -> c_int {
		assert!(offset + size_of::<c_int>() <= size_of::<[usize; 64]>());
		assert!(offset.is_multiple_of(align_of::<c_int>()));

		// SAFETY: The storage holds at least 64 words, and the place is
		// aligned.
		unsafe { self.as_ptr().byte_add(offset).cast::<c_int>().read() }
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

	/// Writes the byte at `offset` into the entity.
	///
	/// # Panics
	///
	/// As for [`byte`](Self::byte).
	pub fn set_byte(&mut self, offset: usize, value: u8) {
		assert!(offset < size_of::<[usize; 64]>());

		// SAFETY: As for `byte`.
		unsafe { self.as_ptr().byte_add(offset).cast::<u8>().write(value) };
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

	/// Writes the `int` at `offset` into the entity.
	///
	/// # Panics
	///
	/// As for [`int`](Self::int).
	pub fn set_int(&mut self, offset: usize, value: c_int) {
		assert!(offset + size_of::<c_int>() <= size_of::<[usize; 64]>());
		assert!(offset.is_multiple_of(align_of::<c_int>()));

		// SAFETY: As for `int`.
		unsafe { self.as_ptr().byte_add(offset).cast::<c_int>().write(value) };
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

	/// The members of `CBaseEntity` the entity stores from
	/// [`MOCK_STATE_OFFSET`] on, zeroed when it is built.
	pub fn state(&mut self) -> &mut MockState {
		// SAFETY: The storage holds at least 64 words, zeroed when built, past
		// the state's end, aligned for pointers. Zero is valid for every member,
		// and the borrow of the mock keeps entity handles from reaching them
		// while it lives.
		unsafe {
			&mut *self
				.as_ptr()
				.byte_add(MOCK_STATE_OFFSET)
				.cast::<MockState>()
		}
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

/// `ICollideable::GetCollisionGroup`, which returns the group
/// [`set_collision_group`] or the last [`MockEntity::new`] set on this thread.
unsafe extern "C" fn get_collision_group(_: *const sys::ICollideable) -> c_int {
	COLLISION_GROUP.get()
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

/// TF2's `CBaseEntity::GetMaxHealth`, which adds [`MOCK_MAX_HEALTH_BONUS`] to
/// the mock entity's `m_iMaxHealth`, as TF2's players compute theirs.
unsafe extern "C" fn get_max_health(this: *const sys::CBaseEntity) -> c_int {
	// SAFETY: Every mock entity stores `m_iMaxHealth` at its offset.
	let stored = unsafe { this.byte_add(MOCK_MAX_HEALTH_OFFSET).cast::<c_int>().read() };

	stored + MOCK_MAX_HEALTH_BONUS
}

unsafe extern "C" fn get_networkable(_: *mut sys::IServerEntity) -> *mut sys::IServerNetworkable {
	NETWORKABLE.get()
}

unsafe extern "C" fn get_origin(_: *const sys::ICollideable) -> *const sys::Vector {
	ORIGIN.with(Cell::as_ptr).cast_const()
}

/// Writes a box reaching one unit from the origin each way.
unsafe extern "C" fn get_surrounding_bounds(
	_: *mut sys::ICollideable,
	mins: *mut sys::Vector,
	maxs: *mut sys::Vector,
) {
	let origin = ORIGIN.get();

	// SAFETY: The caller passes two writable vectors.
	unsafe {
		mins.write(sys::Vector {
			x: origin.x - 1.0,
			y: origin.y - 1.0,
			z: origin.z - 1.0,
		});

		maxs.write(sys::Vector {
			x: origin.x + 1.0,
			y: origin.y + 1.0,
			z: origin.z + 1.0,
		});
	}
}

unsafe extern "C" fn get_server_class(_: *mut sys::IServerNetworkable) -> *mut sys::ServerClass {
	SERVER_CLASS.get()
}

/// The fields of `CBaseEntity`'s map holding health that mock entities store:
/// `m_iHealth`, `m_iMaxHealth`, `m_lifeState` and `m_takedamage`.
///
/// For tests only. They are kept apart from [`base_entity_fields`], since
/// some wrappers check that a datamap declares `m_lifeState`.
pub fn health_fields() -> [sys::typedescription_t; 4] {
	let member = |name, field_type, offset, size: usize| {
		let mut member = field(name, field_type, offset);

		member.fieldSizeInBytes = c_int::try_from(size).unwrap();
		member
	};

	[
		member(
			c"m_iHealth",
			sys::_fieldtypes_FIELD_INTEGER,
			MOCK_HEALTH_OFFSET,
			size_of::<c_int>(),
		),
		member(
			c"m_iMaxHealth",
			sys::_fieldtypes_FIELD_INTEGER,
			MOCK_MAX_HEALTH_OFFSET,
			size_of::<c_int>(),
		),
		member(
			c"m_lifeState",
			sys::_fieldtypes_FIELD_CHARACTER,
			MOCK_LIFE_STATE_OFFSET,
			1,
		),
		member(
			c"m_takedamage",
			sys::_fieldtypes_FIELD_CHARACTER,
			MOCK_TAKE_DAMAGE_OFFSET,
			1,
		),
	]
}

/// `CBaseEntity::IsAlive`, which tells whether the mock entity's
/// `m_lifeState` is `LIFE_ALIVE`, as the game's does.
unsafe extern "C" fn is_alive(this: *mut sys::CBaseEntity) -> bool {
	// SAFETY: Every mock entity stores `m_lifeState` at its offset.
	unsafe { this.byte_add(MOCK_LIFE_STATE_OFFSET).cast::<u8>().read() == LIFE_ALIVE }
}

/// Sets what mock entities' `AcceptInput` returns on this thread.
///
/// For tests only.
pub fn set_accepts(accepts: bool) {
	ACCEPTS.set(accepts);
}

/// Sets the collision group mock entities report on this thread, such as
/// `COLLISION_GROUP_DEBRIS`, until the next [`MockEntity::new`] resets it to
/// `COLLISION_GROUP_NONE`.
///
/// For tests only.
pub fn set_collision_group(group: c_int) {
	COLLISION_GROUP.set(group);
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

/// The members of `CBaseEntity`'s map that mock entities store in their
/// [`MockState`], at the offsets [`MockEntity::state`] has them.
///
/// For tests only. They are kept apart from [`base_entity_fields`], as
/// [`health_fields`] are.
pub fn state_fields() -> [sys::typedescription_t; 16] {
	let member = |name, field_type, offset: usize, size: usize| {
		let mut member = field(name, field_type, MOCK_STATE_OFFSET + offset);

		member.fieldSizeInBytes = c_int::try_from(size).unwrap();
		member.fieldSize = 1;
		member
	};

	let int = size_of::<c_int>();
	let string = size_of::<sys::string_t>();
	let vector = size_of::<sys::Vector>();

	[
		member(
			c"m_fEffects",
			sys::_fieldtypes_FIELD_INTEGER,
			offset_of!(MockState, effects),
			int,
		),
		member(
			c"m_fFlags",
			sys::_fieldtypes_FIELD_INTEGER,
			offset_of!(MockState, flags),
			int,
		),
		member(
			c"m_flFriction",
			sys::_fieldtypes_FIELD_FLOAT,
			offset_of!(MockState, friction),
			int,
		),
		member(
			c"m_flGravity",
			sys::_fieldtypes_FIELD_FLOAT,
			offset_of!(MockState, gravity),
			int,
		),
		member(
			c"m_hMoveParent",
			sys::_fieldtypes_FIELD_EHANDLE,
			offset_of!(MockState, move_parent),
			int,
		),
		member(
			c"m_iTeamNum",
			sys::_fieldtypes_FIELD_INTEGER,
			offset_of!(MockState, team),
			int,
		),
		member(
			c"m_spawnflags",
			sys::_fieldtypes_FIELD_INTEGER,
			offset_of!(MockState, spawn_flags),
			int,
		),
		member(
			c"m_clrRender",
			sys::_fieldtypes_FIELD_COLOR32,
			offset_of!(MockState, render_color),
			size_of::<sys::color32>(),
		),
		member(
			c"m_nModelIndex",
			sys::_fieldtypes_FIELD_SHORT,
			offset_of!(MockState, model_index),
			size_of::<i16>(),
		),
		member(
			c"m_MoveType",
			sys::_fieldtypes_FIELD_CHARACTER,
			offset_of!(MockState, move_type),
			1,
		),
		member(
			c"m_nRenderMode",
			sys::_fieldtypes_FIELD_CHARACTER,
			offset_of!(MockState, render_mode),
			1,
		),
		member(
			c"m_nTransmitStateOwnedCounter",
			sys::_fieldtypes_FIELD_CHARACTER,
			offset_of!(MockState, transmit_state_owners),
			1,
		),
		member(
			c"m_iParent",
			sys::_fieldtypes_FIELD_STRING,
			offset_of!(MockState, parent_name),
			string,
		),
		member(
			c"m_ModelName",
			sys::_fieldtypes_FIELD_MODELNAME,
			offset_of!(MockState, model_name),
			string,
		),
		member(
			c"m_vecAbsVelocity",
			sys::_fieldtypes_FIELD_VECTOR,
			offset_of!(MockState, abs_velocity),
			vector,
		),
		member(
			c"m_vecVelocity",
			sys::_fieldtypes_FIELD_VECTOR,
			offset_of!(MockState, velocity),
			vector,
		),
	]
}

/// A datamap chain through `classes`, from the first to the last, each
/// declaring its fields, and from there to a `CBaseEntity` map declaring
/// [`base_entity_fields`] and [`state_fields`].
///
/// For tests only.
pub fn state_maps(
	classes: Vec<(&'static CStr, Vec<sys::typedescription_t>)>,
) -> *mut sys::datamap_t {
	let mut fields = Vec::from(base_entity_fields());

	fields.extend(state_fields());

	let base = sdk_raw::test_support::entities::data_map(c"CBaseEntity", fields, null_mut());

	classes
		.into_iter()
		.rev()
		.fold(base, |base, (class, fields)| {
			sdk_raw::test_support::entities::data_map(class, fields, base)
		})
}

/// `CBaseEntity::TakeHealth`, which records its arguments for
/// [`take_health_calls`], and adds the healing to the mock entity's
/// `m_iHealth`, truncated, without a maximum, returning what it added.
unsafe extern "C" fn take_health(
	this: *mut sys::CBaseEntity,
	amount: f32,
	damage_type: c_int,
) -> c_int {
	TAKE_HEALTH_CALLS.with_borrow_mut(|calls| calls.push((amount, damage_type)));

	let gained = amount as c_int;

	// SAFETY: Every mock entity stores `m_iHealth` at its offset.
	unsafe {
		let health = this.byte_add(MOCK_HEALTH_OFFSET).cast::<c_int>();

		health.write(health.read() + gained);
	}

	gained
}

/// The amount and `DMG_*` mask of each call of mock entities' `TakeHealth`
/// on this thread since the last [`MockEntity::new`].
///
/// For tests only.
pub fn take_health_calls() -> Vec<(f32, c_int)> {
	TAKE_HEALTH_CALLS.with_borrow(Clone::clone)
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

/// How many times mock entities on this thread updated their transmit state
/// (`UpdateTransmitState`) since the last [`MockEntity::new`].
///
/// For tests only.
pub fn transmit_state_updates() -> usize {
	TRANSMIT_STATE_UPDATES.get()
}

/// `CBaseEntity::UpdateTransmitState`, which counts the update for
/// [`transmit_state_updates`], and returns no flags.
unsafe extern "C" fn update_transmit_state(_: *mut sys::CBaseEntity) -> c_int {
	TRANSMIT_STATE_UPDATES.set(TRANSMIT_STATE_UPDATES.get() + 1);
	0
}
