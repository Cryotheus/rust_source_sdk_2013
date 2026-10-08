//! A mock `IServerTools` the unit tests of creating, parenting, dissolving
//! and finding entities share: it creates, spawns and removes mock entities,
//! records the key values and move types it sets, pools strings into the
//! world's name, as the game's `targetname` key does, answers searches from a
//! list of entities, and looks handles up in an entity list.

use crate::entities::EntityHandle;
use crate::interfaces::ServerTools;
use crate::server::Game;
use crate::test_support::entities::{MOCK_EFLAGS_OFFSET, MOCK_NAME_OFFSET};
use crate::test_support::leak;
use sdk_raw::entities::EFL_KILLME;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int};
use std::ptr::{NonNull, null_mut};

/// A search the mock tools answered, with the entity it started after.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Search {
	/// `FindEntityByClassname`, with the class name.
	Class(*mut sys::CBaseEntity, CString),

	/// `FindEntityByModel`, with the model.
	Model(*mut sys::CBaseEntity, CString),

	/// `FindEntityInSphere`, with the center and radius.
	Sphere(*mut sys::CBaseEntity, [f32; 3], f32),
}

thread_local! {
	/// The classes `CreateEntityByName` was asked for, in order.
	static CLASSES: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };

	/// What `CreateEntityByName` returns, null for an unknown class.
	static CREATED: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };

	/// The entities searches find, in the order of the entity list.
	static FOUND: RefCell<Vec<*mut sys::CBaseEntity>> = const { RefCell::new(Vec::new()) };

	/// The key values `SetKeyValue` received for entities other than the
	/// world, in order.
	static KEYS: RefCell<Vec<(*mut sys::CBaseEntity, CString, CString)>> =
		const { RefCell::new(Vec::new()) };

	/// The entity list `GetEntityList` returns.
	static LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };

	/// The move types `SetMoveType` set, in order, with the move collide
	/// types of its overload that sets both.
	static MOVE_TYPES: RefCell<Vec<(*mut sys::CBaseEntity, c_int, Option<c_int>)>> =
		const { RefCell::new(Vec::new()) };

	/// The strings `SetKeyValue` pooled.
	static POOL: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };

	/// The key `SetKeyValue` refuses.
	static REFUSED: RefCell<Option<CString>> = const { RefCell::new(None) };

	/// The entities `RemoveEntity` removed, in order.
	static REMOVED: RefCell<Vec<*mut sys::CBaseEntity>> = const { RefCell::new(Vec::new()) };

	/// The searches answered, in order.
	static SEARCHES: RefCell<Vec<Search>> = const { RefCell::new(Vec::new()) };

	/// Whether `DispatchSpawn` marks the entity for deletion, as classes that
	/// find a key value wrong do.
	static SPAWN_REMOVES: Cell<bool> = const { Cell::new(false) };

	/// The entities `DispatchSpawn` spawned, in order.
	static SPAWNED: RefCell<Vec<*mut sys::CBaseEntity>> = const { RefCell::new(Vec::new()) };

	/// The entity `GetBaseEntityByEntIndex` finds at index 0.
	static WORLD: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };
}

/// Mock tools, whose state lives in this thread's locals, which
/// [`MockTools::new`] resets.
///
/// For tests only. The tools and the entity list are leaked.
pub(crate) struct MockTools {
	raw: *mut sys::IServerTools,
}

impl MockTools {
	/// Builds tools whose world, at index 0, is `world`, which create nothing,
	/// find nothing, refuse no key, and whose entity list is empty.
	pub(crate) fn new(world: *mut sys::CBaseEntity) -> Self {
		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes slots of
		// the vtable being built.
		let vtable = unsafe {
			mock_vtable::<sys::IServerTools__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerTools_CreateEntityByName)
						.write(create_entity_by_name);
					(&raw mut (*vtable).IServerTools_DispatchSpawn).write(dispatch_spawn);
					(&raw mut (*vtable).IServerTools_FindEntityByClassname)
						.write(find_by_class_name);
					(&raw mut (*vtable).IServerTools_FindEntityByModel).write(find_by_model);
					(&raw mut (*vtable).IServerTools_FindEntityInSphere).write(find_in_sphere);
					(&raw mut (*vtable).IServerTools_GetBaseEntityByEntIndex)
						.write(entity_by_index);
					(&raw mut (*vtable).IServerTools_GetEntityList).write(entity_list);
					(&raw mut (*vtable).IServerTools_RemoveEntity).write(remove_entity);
					(&raw mut (*vtable).IServerTools_SetKeyValue).write(set_key_value);
					(&raw mut (*vtable).IServerTools_SetMoveType).write(set_move_type);
					(&raw mut (*vtable).IServerTools_SetMoveType1).write(set_move_type_and_collide);
				},
			)
		};

		let list = Box::<sys::CGlobalEntityList>::new_zeroed();

		CLASSES.take();
		CREATED.set(null_mut());
		FOUND.take();
		KEYS.take();
		// SAFETY: Zero is valid for every member of the list, whose entries
		// then hold no entity.
		LIST.set(Box::into_raw(unsafe { list.assume_init() }));
		MOVE_TYPES.take();
		REFUSED.take();
		REMOVED.take();
		SEARCHES.take();
		SPAWN_REMOVES.set(false);
		SPAWNED.take();
		WORLD.set(world);

		Self {
			raw: leak(sys::IServerTools {
				vtable_: Box::into_raw(vtable),
			}),
		}
	}

	/// Lists `entity` in the entity list under `handle`'s index and serial
	/// number.
	pub(crate) fn list(&self, entity: *mut sys::CBaseEntity, handle: EntityHandle) {
		let index = handle.index().unwrap();

		// SAFETY: The list is leaked, and the index lies within its array.
		// Entries are written without forming references.
		unsafe {
			let info = (&raw mut (*LIST.get())._base.m_EntPtrArray)
				.cast::<sys::CEntInfo>()
				.add(index);

			(&raw mut (*info).m_pEntity).write(entity.cast());
			(&raw mut (*info).m_SerialNumber).write(handle.serial_number().try_into().unwrap());
		}
	}

	/// Makes `CreateEntityByName` return `entity`, or null for an unknown
	/// class.
	pub(crate) fn set_created(&self, entity: *mut sys::CBaseEntity) {
		CREATED.set(entity);
	}

	/// Makes searches find `entities`, in order, those that match.
	pub(crate) fn set_found(&self, entities: Vec<*mut sys::CBaseEntity>) {
		FOUND.set(entities);
	}

	/// Makes `SetKeyValue` refuse `key`.
	pub(crate) fn set_refused(&self, key: &CStr) {
		REFUSED.set(Some(key.to_owned()));
	}

	/// Sets whether `DispatchSpawn` marks the entity for deletion.
	pub(crate) fn set_spawn_removes(&self, removes: bool) {
		SPAWN_REMOVES.set(removes);
	}

	/// Takes the classes `CreateEntityByName` was asked for.
	pub(crate) fn take_classes(&self) -> Vec<CString> {
		CLASSES.take()
	}

	/// Takes the key values set on entities other than the world.
	pub(crate) fn take_keys(&self) -> Vec<(*mut sys::CBaseEntity, CString, CString)> {
		KEYS.take()
	}

	/// Takes the move types set, with the move collide types set with them.
	pub(crate) fn take_move_types(&self) -> Vec<(*mut sys::CBaseEntity, c_int, Option<c_int>)> {
		MOVE_TYPES.take()
	}

	/// Takes the entities removed.
	pub(crate) fn take_removed(&self) -> Vec<*mut sys::CBaseEntity> {
		REMOVED.take()
	}

	/// Takes the searches answered.
	pub(crate) fn take_searches(&self) -> Vec<Search> {
		SEARCHES.take()
	}

	/// Takes the entities spawned.
	pub(crate) fn take_spawned(&self) -> Vec<*mut sys::CBaseEntity> {
		SPAWNED.take()
	}

	/// The mock tools, which outlive every test.
	pub(crate) fn tools(&self) -> ServerTools<'static> {
		// SAFETY: The tools are leaked, and their vtable answers what the
		// wrappers that use them call.
		unsafe { ServerTools::from_raw(NonNull::new(self.raw).unwrap(), Game::TeamFortress2) }
	}
}

/// `IServerTools::CreateEntityByName`, which records the class, and returns
/// [`CREATED`].
unsafe extern "C" fn create_entity_by_name(
	_: *mut sys::IServerTools,
	class: *const c_char,
) -> *mut sys::CBaseEntity {
	// SAFETY: The wrappers pass a NUL-terminated class.
	let class = unsafe { CStr::from_ptr(class) }.to_owned();

	CLASSES.with_borrow_mut(|classes| classes.push(class));
	CREATED.get()
}

/// `IServerTools::DispatchSpawn`, which records the entity, and marks it for
/// deletion if [`SPAWN_REMOVES`] says to.
unsafe extern "C" fn dispatch_spawn(_: *mut sys::IServerTools, entity: *mut sys::CBaseEntity) {
	SPAWNED.with_borrow_mut(|spawned| spawned.push(entity));

	if SPAWN_REMOVES.get() {
		// SAFETY: The entity is a mock entity.
		unsafe { mark_for_deletion(entity) };
	}
}

/// `IServerTools::GetBaseEntityByEntIndex`, which finds only the world, at
/// index 0.
unsafe extern "C" fn entity_by_index(
	_: *mut sys::IServerTools,
	index: c_int,
) -> *mut sys::CBaseEntity {
	if index == 0 { WORLD.get() } else { null_mut() }
}

/// `IServerTools::GetEntityList`, which returns [`LIST`].
unsafe extern "C" fn entity_list(_: *mut sys::IServerTools) -> *mut sys::CGlobalEntityList {
	LIST.get()
}

/// The entity [`FOUND`] lists after `after`, or its first for null.
fn find_after(after: *mut sys::CBaseEntity) -> *mut sys::CBaseEntity {
	FOUND.with_borrow(|found| {
		let next = if after.is_null() {
			0
		} else {
			found.iter().position(|&entity| entity == after).unwrap() + 1
		};

		found.get(next).copied().unwrap_or(null_mut())
	})
}

/// `IServerTools::FindEntityByClassname`, which records the search, and finds
/// the next entity of [`FOUND`].
unsafe extern "C" fn find_by_class_name(
	_: *mut sys::IServerTools,
	after: *mut sys::CBaseEntity,
	class: *const c_char,
) -> *mut sys::CBaseEntity {
	// SAFETY: The wrappers pass a NUL-terminated class.
	let class = unsafe { CStr::from_ptr(class) }.to_owned();

	SEARCHES.with_borrow_mut(|searches| searches.push(Search::Class(after, class)));
	find_after(after)
}

/// `IServerTools::FindEntityByModel`, which records the search, and finds the
/// next entity of [`FOUND`].
unsafe extern "C" fn find_by_model(
	_: *mut sys::IServerTools,
	after: *mut sys::CBaseEntity,
	model: *const c_char,
) -> *mut sys::CBaseEntity {
	// SAFETY: The wrappers pass a NUL-terminated model.
	let model = unsafe { CStr::from_ptr(model) }.to_owned();

	SEARCHES.with_borrow_mut(|searches| searches.push(Search::Model(after, model)));
	find_after(after)
}

/// `IServerTools::FindEntityInSphere`, which records the search, and finds
/// the next entity of [`FOUND`].
unsafe extern "C" fn find_in_sphere(
	_: *mut sys::IServerTools,
	after: *mut sys::CBaseEntity,
	center: *const sys::Vector,
	radius: f32,
) -> *mut sys::CBaseEntity {
	// SAFETY: The wrappers pass a live center.
	let sys::Vector { x, y, z } = unsafe { center.read() };

	SEARCHES.with_borrow_mut(|searches| searches.push(Search::Sphere(after, [x, y, z], radius)));
	find_after(after)
}

/// Marks a mock entity for deletion, as the game's deferred removal does.
///
/// # Safety
///
/// `entity` must be a live mock entity.
unsafe fn mark_for_deletion(entity: *mut sys::CBaseEntity) {
	// SAFETY: Every mock entity stores `m_iEFlags` at its offset.
	unsafe {
		entity
			.byte_add(MOCK_EFLAGS_OFFSET)
			.cast::<c_int>()
			.write(EFL_KILLME)
	};
}

/// `IServerTools::RemoveEntity`, which records the entity, and marks it for
/// deletion.
unsafe extern "C" fn remove_entity(_: *mut sys::IServerTools, entity: *mut sys::CBaseEntity) {
	REMOVED.with_borrow_mut(|removed| removed.push(entity));

	// SAFETY: The entity is a mock entity.
	unsafe { mark_for_deletion(entity) };
}

/// `IServerTools::SetKeyValue`, which records the key value of an entity
/// other than the world, refuses [`REFUSED`], and pools a `targetname` into
/// the entity's name, ignoring case, as `CGameStringPool` does.
unsafe extern "C" fn set_key_value(
	_: *mut sys::IServerTools,
	entity: *mut sys::CBaseEntity,
	key: *const c_char,
	value: *const c_char,
) -> bool {
	// SAFETY: The wrappers pass NUL-terminated keys and values.
	let (key, value) = unsafe { (CStr::from_ptr(key), CStr::from_ptr(value)) };

	if entity != WORLD.get() {
		KEYS.with_borrow_mut(|keys| keys.push((entity, key.to_owned(), value.to_owned())));
	}

	if REFUSED.with_borrow(|refused| refused.as_deref() == Some(key)) {
		return false;
	}

	if key == c"targetname" {
		let pooled = POOL.with_borrow_mut(|pool| {
			let index = pool
				.iter()
				.position(|pooled| pooled.to_bytes().eq_ignore_ascii_case(value.to_bytes()))
				.unwrap_or_else(|| {
					pool.push(value.to_owned());
					pool.len() - 1
				});

			pool[index].as_ptr()
		});

		// SAFETY: Every entity the wrappers set keys of is a mock entity, which
		// stores its name at this offset.
		unsafe {
			entity
				.byte_add(MOCK_NAME_OFFSET)
				.cast::<sys::string_t>()
				.write(sys::string_t { pszValue: pooled })
		};
	}

	true
}

/// `IServerTools::SetMoveType`, which records the move type.
unsafe extern "C" fn set_move_type(
	_: *mut sys::IServerTools,
	entity: *mut sys::CBaseEntity,
	move_type: c_int,
) {
	MOVE_TYPES.with_borrow_mut(|types| types.push((entity, move_type, None)));
}

/// `IServerTools::SetMoveType`'s overload that also sets the move collide
/// type, which records both.
unsafe extern "C" fn set_move_type_and_collide(
	_: *mut sys::IServerTools,
	entity: *mut sys::CBaseEntity,
	move_type: c_int,
	collide: c_int,
) {
	MOVE_TYPES.with_borrow_mut(|types| types.push((entity, move_type, Some(collide))));
}
