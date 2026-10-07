//! A mock `IServerTools` that creates, keys, spawns and removes the entity a
//! test prepares, for the wrappers that spawn TF2's entities, and the
//! `ChangeTeam` they put it on its team through.

use super::super::entities::{MOCK_EFLAGS_OFFSET, MockEntity};
use super::super::leak;
use super::super::server::export;
use crate::interfaces::ServerTools;
use crate::server::Module;
use sdk_raw::entities::EFL_KILLME;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr::null_mut;

thread_local! {
	/// The entity the next `CreateEntityByName` returns, or null.
	static CREATED: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };

	/// What the mock tools and `ChangeTeam` were asked to do, in order.
	static EVENTS: RefCell<Vec<SpawnEvent>> = const { RefCell::new(Vec::new()) };

	/// The entities `FindEntityByClassname` finds, in order, with their class
	/// names.
	static FOUND: RefCell<Vec<(*mut sys::CBaseEntity, CString)>> = const { RefCell::new(Vec::new()) };

	/// The key `SetKeyValue` refuses, if any.
	static REFUSED: RefCell<Option<CString>> = const { RefCell::new(None) };

	/// Whether `DispatchSpawn` marks the entity for deletion, as a `Spawn`
	/// that removes its entity does.
	static REMOVES_ON_SPAWN: Cell<bool> = const { Cell::new(false) };
}

/// Something the mock tools or `ChangeTeam` were asked to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnEvent {
	/// `CreateEntityByName` with this class name.
	Created(CString),

	/// `SetKeyValue` with this key and value.
	KeyValue(CString, CString),

	/// `ChangeTeam` with this team.
	Team(c_int),

	/// `DispatchSpawn`.
	Spawned,

	/// `RemoveEntity`.
	Removed,
}

/// `CBaseEntity::ChangeTeam`, which records the team.
///
/// # Safety
///
/// None: it only records its argument. It is `unsafe` to fit the vtable
/// slot.
pub unsafe extern "C" fn change_team(_: *mut sys::CBaseEntity, team: c_int) {
	record(SpawnEvent::Team(team));
}

/// `IServerTools::CreateEntityByName`, which returns the entity
/// [`set_created`] set, once.
unsafe extern "C" fn create_entity_by_name(
	_: *mut sys::IServerTools,
	class_name: *const c_char,
) -> *mut sys::CBaseEntity {
	// SAFETY: The wrappers pass a NUL-terminated class name.
	let class_name = unsafe { CStr::from_ptr(class_name) };

	record(SpawnEvent::Created(class_name.to_owned()));
	CREATED.replace(null_mut())
}

/// `IServerTools::DispatchSpawn`, which records the spawn, and marks the
/// entity for deletion if [`remove_on_spawn`] asked it to.
unsafe extern "C" fn dispatch_spawn(_: *mut sys::IServerTools, entity: *mut sys::CBaseEntity) {
	record(SpawnEvent::Spawned);

	if REMOVES_ON_SPAWN.get() {
		// SAFETY: Every entity the tools create is a mock entity, which stores
		// its `m_iEFlags` at this offset.
		unsafe {
			let flags = entity.byte_add(MOCK_EFLAGS_OFFSET).cast::<c_int>();

			flags.write(flags.read() | EFL_KILLME);
		}
	}
}

/// Exports the mock tools, and forgets what earlier mock tools on this
/// thread were asked to do and set to do.
pub fn export_spawn_tools() {
	CREATED.set(null_mut());
	EVENTS.take();
	FOUND.take();
	REFUSED.take();
	REMOVES_ON_SPAWN.set(false);

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the vtable
	// being built.
	let vtable = Box::leak(unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_CreateEntityByName).write(create_entity_by_name);
			(&raw mut (*vtable).IServerTools_DispatchSpawn).write(dispatch_spawn);
			(&raw mut (*vtable).IServerTools_FindEntityByClassname).write(find_by_class_name);
			(&raw mut (*vtable).IServerTools_RemoveEntity).write(remove_entity);
			(&raw mut (*vtable).IServerTools_SetKeyValue).write(set_key_value);
		})
	});

	export(
		Module::GameServer,
		ServerTools::VERSION,
		leak(sys::IServerTools { vtable_: vtable }),
	);
}

/// `IServerTools::FindEntityByClassname`, which finds the entities
/// [`register_class_name`] registered, in order.
unsafe extern "C" fn find_by_class_name(
	_: *mut sys::IServerTools,
	after: *mut sys::CBaseEntity,
	class_name: *const c_char,
) -> *mut sys::CBaseEntity {
	// SAFETY: The wrappers pass a NUL-terminated class name.
	let class_name = unsafe { CStr::from_ptr(class_name) };

	FOUND.with_borrow(|found| {
		let start = found
			.iter()
			.position(|(entity, _)| *entity == after)
			.map_or(0, |index| index + 1);

		found[start.min(found.len())..]
			.iter()
			.find(|(_, name)| name.as_c_str() == class_name)
			.map_or(null_mut(), |(entity, _)| *entity)
	})
}

/// Replaces the vtable of `mock` with a copy that also answers each slot of
/// `slots` with its function.
pub fn patch_slots(mock: &mut MockEntity, slots: &[(usize, *const ())]) {
	// The length of the vtable `MockEntity::with_layout` builds.
	let length = sdk_raw::entities::TF2_TELEPORT_SLOT
		.max(sdk_raw::entities::GET_DATA_DESC_MAP_SLOT)
		.max(sdk_raw::entities::ACCEPT_INPUT_SLOT)
		.max(sdk_raw::entities::health::TF2_GET_MAX_HEALTH_SLOT)
		.max(sdk_raw::entities::health::TAKE_HEALTH_SLOT)
		.max(sdk_raw::entities::health::IS_ALIVE_SLOT)
		+ 1;

	let patched_length = slots
		.iter()
		.map(|&(slot, _)| slot + 1)
		.fold(length, usize::max);

	let pointer = mock.as_ptr();
	let mut vtable = vec![unexpected_call as *const (); patched_length];

	// SAFETY: A mock entity starts with the pointer to its vtable, of `length`
	// slots.
	unsafe {
		let original = pointer.cast::<*const *const ()>().read();

		for (slot, entry) in vtable.iter_mut().enumerate().take(length) {
			*entry = original.add(slot).read();
		}
	}

	for &(slot, function) in slots {
		vtable[slot] = function;
	}

	// SAFETY: A mock entity starts with the pointer to its vtable, and the new
	// vtable is leaked.
	unsafe {
		pointer
			.cast::<*const *const ()>()
			.write(vtable.leak().as_ptr())
	};
}

/// Records an event.
fn record(event: SpawnEvent) {
	EVENTS.with_borrow_mut(|events| events.push(event));
}

/// Makes `SetKeyValue` refuse `key`.
pub fn refuse_key(key: &CStr) {
	REFUSED.set(Some(key.to_owned()));
}

/// Makes the mock `FindEntityByClassname` find `entity` by `class_name`,
/// after the entities registered before it.
pub fn register_class_name(entity: *mut sys::CBaseEntity, class_name: &CStr) {
	FOUND.with_borrow_mut(|found| found.push((entity, class_name.to_owned())));
}

/// `IServerTools::RemoveEntity`, which records the removal.
unsafe extern "C" fn remove_entity(_: *mut sys::IServerTools, _: *mut sys::CBaseEntity) {
	record(SpawnEvent::Removed);
}

/// Makes `DispatchSpawn` mark the entity it spawns for deletion.
pub fn remove_on_spawn(remove: bool) {
	REMOVES_ON_SPAWN.set(remove);
}

/// Makes the next `CreateEntityByName` return `entity`, or nothing for null.
pub fn set_created(entity: *mut sys::CBaseEntity) {
	CREATED.set(entity);
}

/// `IServerTools::SetKeyValue`, which records the key value, unless
/// [`refuse_key`] named its key.
unsafe extern "C" fn set_key_value(
	_: *mut sys::IServerTools,
	_: *mut sys::CBaseEntity,
	key: *const c_char,
	value: *const c_char,
) -> bool {
	// SAFETY: The wrappers pass NUL-terminated keys and values.
	let (key, value) = unsafe { (CStr::from_ptr(key), CStr::from_ptr(value)) };

	if REFUSED.with_borrow(|refused| refused.as_deref() == Some(key)) {
		return false;
	}

	record(SpawnEvent::KeyValue(key.to_owned(), value.to_owned()));
	true
}

/// Takes what the mock tools and `ChangeTeam` were asked to do on this thread
/// since the last call.
pub fn take_events() -> Vec<SpawnEvent> {
	EVENTS.take()
}
