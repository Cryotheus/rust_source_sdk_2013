//! Tests of `ServerTools`: finding, iterating, and removing entities through
//! the game's entity list, and reading their key values.

use super::*;

use crate::test_support::entities::{
	MOCK_EFLAGS_OFFSET, MockEntity, base_entity_fields, set_datamap,
};

use sdk_raw::entities::datamap::FTYPEDESC_KEY;
use sdk_raw::test_support::entities::data_map;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ptr::null_mut;

thread_local! {
	/// The only entity the mock tools find.
	static ENTITY: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };

	/// The entity list `GetEntityList` returns.
	static LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };

	/// How many times `RemoveEntity` ran.
	static REMOVALS: Cell<usize> = const { Cell::new(0) };

	/// Every key `GetKeyValue` was asked for, in order.
	static KEYS_READ: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
}

/// A field embedding `count` objects described by `map` at `offset`.
fn embedded(map: *mut sys::datamap_t, offset: usize, count: u16) -> sys::typedescription_t {
	// SAFETY: Zero is valid for every field of `typedescription_t`.
	let mut embedded: sys::typedescription_t = unsafe { std::mem::zeroed() };

	embedded.fieldType = sys::_fieldtypes_FIELD_EMBEDDED;
	embedded.fieldOffset[0] = offset as c_int;
	embedded.fieldSize = count;
	embedded.td = map;
	embedded
}

/// `IServerTools::FindEntityByHammerID`, which finds [`ENTITY`] as 1234.
unsafe extern "C" fn entity_by_hammer_id(
	_: *mut sys::IServerTools,
	id: c_int,
) -> *mut sys::CBaseEntity {
	if id == 1234 { ENTITY.get() } else { null_mut() }
}

/// `IServerTools::GetBaseEntityByEntIndex`, which finds [`ENTITY`] at index 1.
unsafe extern "C" fn entity_by_index(
	_: *mut sys::IServerTools,
	index: c_int,
) -> *mut sys::CBaseEntity {
	if index == 1 { ENTITY.get() } else { null_mut() }
}

/// `IServerTools::GetEntityList`, which returns [`LIST`].
unsafe extern "C" fn entity_list(_: *mut sys::IServerTools) -> *mut sys::CGlobalEntityList {
	LIST.get()
}

/// `IServerTools::FirstEntity`, which returns [`ENTITY`].
unsafe extern "C" fn first_entity(_: *mut sys::IServerTools) -> *mut sys::CBaseEntity {
	ENTITY.get()
}

/// `IServerTools::GetKeyValue`, which formats every key but `missing` as
/// `100`, as for an integer field.
unsafe extern "C" fn get_key_value(
	_: *mut sys::IServerTools,
	_: *mut sys::CBaseEntity,
	key: *const c_char,
	value: *mut c_char,
	capacity: c_int,
) -> bool {
	// SAFETY: The wrapper passes a NUL-terminated key.
	let key = unsafe { CStr::from_ptr(key) }.to_owned();
	let found = key.as_c_str() != c"missing";

	assert_eq!(capacity as usize, KEY_VALUE_CAPACITY);
	KEYS_READ.with_borrow_mut(|keys| keys.push(key));

	if found {
		// SAFETY: The wrapper passes a buffer of `capacity` bytes.
		unsafe { value.copy_from_nonoverlapping(c"100".as_ptr(), 4) };
	}

	found
}

/// A key field of `field_type` named `name`, at `offset` in its object.
fn key(name: &'static CStr, field_type: sys::fieldtype_t, offset: usize) -> sys::typedescription_t {
	// SAFETY: Zero is valid for every field of `typedescription_t`.
	let mut key: sys::typedescription_t = unsafe { std::mem::zeroed() };

	key.fieldType = field_type;
	key.fieldOffset[0] = offset as c_int;
	key.fieldSize = 1;
	key.flags = FTYPEDESC_KEY;
	key.externalName = name.as_ptr();
	key
}

#[test]
fn key_values_of_string_fields_are_read_from_the_field() {
	use sys::{
		_fieldtypes_FIELD_INTEGER as INTEGER, _fieldtypes_FIELD_MODELNAME as MODELNAME,
		_fieldtypes_FIELD_SOUNDNAME as SOUNDNAME, _fieldtypes_FIELD_STRING as STRING,
	};

	let mut mock = MockEntity::new(1 | 9 << 16);
	let raw = mock.as_ptr();
	let set_string = |offset: usize, string: &'static CStr| {
		// SAFETY: The mock's storage holds a `string_t` at each offset used.
		unsafe {
			raw.byte_add(offset)
				.cast::<sys::string_t>()
				.write(sys::string_t {
					pszValue: string.as_ptr(),
				})
		}
	};

	// An embedded object at 128, and an array of two at 192.
	let inner = data_map(c"CInner", vec![key(c"model", MODELNAME, 8)], null_mut());
	let mut base_fields = base_entity_fields().to_vec();

	base_fields.extend([
		key(c"damagefilter", STRING, 64),
		key(c"unset", STRING, 72),
		key(c"shadowed", STRING, 80),
		key(c"health", INTEGER, 88),
		key(c"message", SOUNDNAME, 96),
	]);

	let base = data_map(c"CBaseEntity", base_fields, null_mut());
	let derived = data_map(
		c"CTestEntity",
		vec![
			key(c"Shadowed", INTEGER, 104),
			embedded(inner, 192, 2),
			embedded(inner, 128, 1),
			key(c"model", STRING, 112),
		],
		base,
	);

	set_datamap(derived);
	set_string(64, c"Pooled_Filter_Name");
	set_string(80, c"base string");
	set_string(96, c"ambient.sound");
	set_string(112, c"models/derived.mdl");
	set_string(136, c"models/embedded.mdl");
	set_string(200, c"models/array.mdl");
	KEYS_READ.take();

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_GetKeyValue).write(get_key_value)
		})
	};

	let mut interface = sys::IServerTools {
		vtable_: &raw const *vtable,
	};

	// SAFETY: The mock outlives the handle.
	let tools =
		unsafe { ServerTools::from_raw(NonNull::from(&mut interface), Game::TeamFortress2) };
	let entity = mock.entity();
	let read = |key: &CStr| tools.key_value(entity, key);

	assert_eq!(
		read(c"damagefilter").as_deref(),
		Some(c"Pooled_Filter_Name")
	);
	assert_eq!(
		read(c"DamageFilter").as_deref(),
		Some(c"Pooled_Filter_Name")
	);
	assert_eq!(read(c"unset").as_deref(), Some(c""));
	assert_eq!(read(c"message").as_deref(), Some(c"ambient.sound"));

	// The single embedded object is searched before the later field, and the
	// array is not searched.
	assert_eq!(read(c"MODEL").as_deref(), Some(c"models/embedded.mdl"));
	assert_eq!(KEYS_READ.take(), Vec::<CString>::new());

	// Other types, and string fields a derived class's field shadows, are
	// formatted by the game.
	assert_eq!(read(c"health").as_deref(), Some(c"100"));
	assert_eq!(read(c"shadowed").as_deref(), Some(c"100"));
	assert_eq!(read(c"missing"), None);
	assert_eq!(
		KEYS_READ.take(),
		[c"health", c"shadowed", c"missing"].map(CStr::to_owned)
	);
}

/// `IServerTools::NextEntity`, for a list holding one entity.
unsafe extern "C" fn next_entity(
	_: *mut sys::IServerTools,
	_: *mut sys::CBaseEntity,
) -> *mut sys::CBaseEntity {
	null_mut()
}

/// `IServerTools::RemoveEntity`, which counts removals and marks the entity
/// for deletion, as the game's deferred removal does.
unsafe extern "C" fn remove_entity(_: *mut sys::IServerTools, entity: *mut sys::CBaseEntity) {
	assert_eq!(entity, ENTITY.get());
	REMOVALS.set(REMOVALS.get() + 1);

	// SAFETY: The entity is the live mock, whose storage holds its flags at
	// this offset.
	unsafe { entity.byte_add(MOCK_EFLAGS_OFFSET).cast::<c_int>().write(1) };
}

#[test]
fn tools_find_iterate_and_remove_entities() {
	let mut mock = MockEntity::new(1 | 9 << 16);

	ENTITY.set(mock.as_ptr());
	REMOVALS.set(0);

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_FirstEntity).write(first_entity);
			(&raw mut (*vtable).IServerTools_NextEntity).write(next_entity);
			(&raw mut (*vtable).IServerTools_GetBaseEntityByEntIndex).write(entity_by_index);
			(&raw mut (*vtable).IServerTools_FindEntityByHammerID).write(entity_by_hammer_id);
			(&raw mut (*vtable).IServerTools_GetEntityList).write(entity_list);
			(&raw mut (*vtable).IServerTools_RemoveEntity).write(remove_entity);
		})
	};

	let mut interface = sys::IServerTools {
		vtable_: &raw const *vtable,
	};

	// SAFETY: The mock outlives the handle.
	let tools =
		unsafe { ServerTools::from_raw(NonNull::from(&mut interface), Game::TeamFortress2) };

	// Slot 1 holds the entity at serial number 9.
	let mut list = Box::<sys::CGlobalEntityList>::new_zeroed();

	// SAFETY: The list is zeroed, which is valid for its entries, and the
	// slot lies within its array.
	unsafe {
		let info = (&raw mut (*list.as_mut_ptr())._base.m_EntPtrArray)
			.cast::<sys::CEntInfo>()
			.add(1);

		(&raw mut (*info).m_pEntity).write(mock.as_ptr().cast());
		(&raw mut (*info).m_SerialNumber).write(9);
	}

	LIST.set(list.as_mut_ptr());

	assert!(tools.entity_by_index(-1).is_none());
	assert!(tools.entity_by_index(2048).is_none());
	assert_eq!(
		tools.entity_by_index(1).map(Entity::as_ptr),
		Some(mock.as_ptr())
	);
	assert_eq!(
		tools.entities().map(Entity::as_ptr).collect::<Vec<_>>(),
		[mock.as_ptr()]
	);
	assert_eq!(
		tools
			.find_by_hammer_id(HammerId::new(1234).unwrap())
			.map(Entity::as_ptr),
		Some(mock.as_ptr())
	);
	assert!(
		tools
			.find_by_hammer_id(HammerId::new(99).unwrap())
			.is_none()
	);

	let handle = tools.entity_by_index(1).unwrap().handle();

	assert_eq!(
		tools.entity_by_handle(handle).map(Entity::as_ptr),
		Some(mock.as_ptr())
	);
	assert_eq!(
		tools.entity_by_handle(EntityHandle::from_raw(1 | 8 << 16)),
		None
	);
	assert_eq!(tools.entity_by_handle(EntityHandle::from_raw(9000)), None);
	assert_eq!(tools.entity_by_handle(EntityHandle::INVALID), None);

	let entity = tools.entity_by_index(1).unwrap();

	tools.remove(entity).unwrap();
	tools.remove(entity).unwrap();

	assert!(entity.is_marked_for_deletion());
	assert_eq!(REMOVALS.get(), 1);
	assert_eq!(
		tools.teleport(entity, Some(Vector::new(0.0, 0.0, 0.0)), None, None),
		Err(TeleportError::MarkedForDeletion)
	);

	ENTITY.set(null_mut());
	LIST.set(null_mut());
}
