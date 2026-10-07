//! Tests of `ServerTools`: finding, iterating, and removing entities through
//! the game's entity list, and reading their key values.

use super::*;
use crate::test_support::datatables::derived_server_class;

use crate::test_support::entities::{
	MOCK_EFLAGS_OFFSET, MockEntity, base_entity_fields, set_datamap, set_networking,
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

	/// Every key and value `SetKeyValue` received, in order.
	static KEYS_SET: RefCell<Vec<(CString, CString)>> = const { RefCell::new(Vec::new()) };

	/// Every name `FindEntityByName` was asked for, in order.
	static NAMES_FOUND: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
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

/// `IServerTools::FindEntityByName`, which records each name, and finds
/// [`ENTITY`] as the first entity named `cap_base`. The entities procedural
/// names are relative to, and the filter, must be null.
unsafe extern "C" fn entity_by_name(
	_: *mut sys::IServerTools,
	after: *mut sys::CBaseEntity,
	name: *const c_char,
	searching: *mut sys::CBaseEntity,
	activator: *mut sys::CBaseEntity,
	caller: *mut sys::CBaseEntity,
	filter: *mut sys::IEntityFindFilter,
) -> *mut sys::CBaseEntity {
	// SAFETY: The wrapper passes a NUL-terminated name.
	let name = unsafe { CStr::from_ptr(name) }.to_owned();
	let found = after.is_null() && name.as_c_str() == c"cap_base";

	assert!(searching.is_null() && activator.is_null() && caller.is_null());
	assert!(filter.is_null());
	NAMES_FOUND.with_borrow_mut(|names| names.push(name));

	if found { ENTITY.get() } else { null_mut() }
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
fn entities_are_found_by_name() {
	let mut mock = MockEntity::new(1 | 9 << 16);

	ENTITY.set(mock.as_ptr());
	NAMES_FOUND.take();

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_FindEntityByName).write(entity_by_name)
		})
	};

	let mut interface = sys::IServerTools {
		vtable_: &raw const *vtable,
	};

	// SAFETY: The mock outlives the handle.
	let tools =
		unsafe { ServerTools::from_raw(NonNull::from(&mut interface), Game::TeamFortress2) };
	let found = tools.find_by_name(None, c"cap_base");

	assert_eq!(found.map(Entity::as_ptr), Some(mock.as_ptr()));
	assert!(tools.find_by_name(found, c"cap_base").is_none());

	// Procedural names never reach the game, which needs an entity searching.
	assert!(tools.find_by_name(None, c"!activator").is_none());
	assert!(tools.find_by_name(None, c"!picker").is_none());
	assert_eq!(
		NAMES_FOUND.take(),
		[c"cap_base", c"cap_base"].map(CString::from)
	);
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

#[test]
fn key_values_the_game_cannot_handle_are_not_set() {
	let mut mock = MockEntity::new(1 | 9 << 16);
	let base = data_map(c"CBaseEntity", base_entity_fields().to_vec(), null_mut());
	let chain = |class| data_map(class, vec![], base);
	let sentry = data_map(c"CObjectSentrygun", vec![], chain(c"CBaseObject"));
	let player = data_map(c"CTFPlayer", vec![], chain(c"CBasePlayer"));

	// A team's class derives from `CTeam`'s send table.
	let team_class = derived_server_class(c"CTFTeam", c"DT_TFTeam", c"DT_Team");

	KEYS_SET.take();

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_SetKeyValue).write(set_key_value)
		})
	};

	let mut interface = sys::IServerTools {
		vtable_: &raw const *vtable,
	};

	// SAFETY: The mock outlives the handle.
	let tools =
		unsafe { ServerTools::from_raw(NonNull::from(&mut interface), Game::TeamFortress2) };
	let entity = mock.entity();
	let mut reached = Vec::new();

	// Sets each key to each value on an entity of `map`'s chain, and checks
	// whether it reaches the game.
	let mut check = |map, cases: &[(&CStr, &CStr, bool)]| {
		set_datamap(map);

		for &(key, value, sets) in cases {
			assert_eq!(
				tools.set_key_value(entity, key, value),
				sets,
				"{key:?} {value:?}"
			);

			if sets {
				reached.push((key.to_owned(), value.to_owned()));
			}
		}
	};

	// `CBaseEntity::KeyValue` cuts the key at its first `#` and compares it
	// ignoring case, and reads the value with `atoi`, which is undefined
	// beyond `int`. Team numbers index arrays of TF2's four teams.
	check(
		base,
		&[
			(c"max_health", c"0", false),
			(c"MAX_HEALTH#2", c" -1", false),
			(c"Max_Health#", c"0x10", false),
			(c"max_health", c"2147483648", false),
			(c"max_health#2", c"2147483647", true),
			(c"max_healthy", c"0", true),
			(c"#max_health", c"0", true),
			(c"teamnumber", c"4", false),
			(c"TeamNumber#1", c"-1", false),
			(c"teamnumber", c"3", true),
			// Keys only some classes read are left to others.
			(c"type", c"9", true),
			(c"team_capsound_9", c"x", true),
			(c"point_index", c"-1", true),
		],
	);

	// A TF2 building's maximum health must round below 2^31 as a float.
	check(
		sentry,
		&[
			(c"max_health", c"2147483584", false),
			(c"max_health", c"2147483583 hp", true),
		],
	);

	// Players and teams keep their team numbers.
	check(player, &[(c"teamnumber", c"2", false)]);
	set_networking(team_class, null_mut());
	check(base, &[(c"teamnumber", c"2", false)]);
	set_networking(null_mut(), null_mut());

	// `CTeamControlPoint::KeyValue` matches the prefixes of its per-team keys
	// case-sensitively, and indexes four teams with the number after them,
	// and previous points with `sscanf(rest, "%d_%d")`, which leaves the team
	// uninitialized and the index 0 if their conversions fail.
	check(
		chain(c"CTeamControlPoint"),
		&[
			(c"team_capsound_4", c"x", false),
			(c"team_icon_-1", c"x", false),
			(c"team_model_3", c"x", true),
			(c"Team_Icon_9", c"x", true),
			(c"team_previouspoint_", c"x", false),
			(c"team_previouspoint_4_0", c"x", false),
			(c"team_previouspoint_1_3", c"x", false),
			(c"team_previouspoint_1_99999999999", c"x", false),
			(c"team_previouspoint_1_2", c"x", true),
			(c"team_previouspoint_ 3x7", c"x", true),
			(c"POINT_INDEX", c"8", false),
			(c"point_index", c"7", true),
			(c"point_default_owner", c"4", false),
			(c"point_default_owner", c"1", false),
			(c"point_default_owner", c"-1", false),
			(c"point_default_owner", c"3", true),
			(c"point_default_owner", c"0", true),
		],
	);
	check(
		chain(c"CTriggerAreaCapture"),
		&[
			(c"team_startcap_4", c"1", false),
			(c"team_numcap_3", c"1", true),
		],
	);
	check(
		chain(c"CTeamControlPointMaster"),
		&[
			(c"team_base_icon_32", c"x", false),
			(c"team_base_icon_31", c"x", true),
		],
	);
	check(
		chain(c"CTFRobotDestruction_RobotSpawn"),
		&[(c"type", c"3", false), (c"type", c"2", true)],
	);

	assert_eq!(KEYS_SET.take(), reached);
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

/// `IServerTools::SetKeyValue`, which records the key and value, and reports
/// the key known.
unsafe extern "C" fn set_key_value(
	_: *mut sys::IServerTools,
	_: *mut sys::CBaseEntity,
	key: *const c_char,
	value: *const c_char,
) -> bool {
	// SAFETY: The wrapper passes NUL-terminated copies.
	let set = unsafe {
		(
			CStr::from_ptr(key).to_owned(),
			CStr::from_ptr(value).to_owned(),
		)
	};

	KEYS_SET.with_borrow_mut(|keys| keys.push(set));
	true
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
