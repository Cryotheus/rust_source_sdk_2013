//! `IServerTools`, which enumerates and manipulates the server's entities.

use crate::entities::{Entity, EntityHandle, TeleportError};
use crate::ffi::{NotThreadSafe, cstring_from_buffer, vcall};
use crate::math::{QAngle, Vector};
use crate::server::{Game, Interface, Module, Server};
use std::ffi::{CStr, CString, c_char, c_int};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};

/// The largest key value [`ServerTools::key_value`] reads, including its terminator.
const KEY_VALUE_CAPACITY: usize = 1024;

/// Entity enumeration and manipulation meant for tools (`IServerTools`).
///
/// Unlike edict lookups, this reaches server-only entities too.
#[doc(alias = "IServerTools")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerTools<'s> {
	raw: NonNull<sys::IServerTools>,
	game: Game,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> ServerTools<'s> {
	/// The version string this interface is requested by.
	pub const VERSION: &'static CStr = c"VSERVERTOOLS003";

	/// # Safety
	///
	/// `raw` must be the live `VSERVERTOOLS003` object, alive for `'s`, of a
	/// game DLL built for `game`, and every call must happen on the server's
	/// main thread.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::IServerTools>, game: Game) -> Self {
		Self {
			raw,
			game,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the interface pointer, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::IServerTools {
		self.raw.as_ptr()
	}

	/// Looks up a networked entity by edict index.
	///
	/// Server-only entities have no edict index; find them with
	/// [`Self::entities`] or [`Self::entity_by_handle`].
	#[doc(alias = "GetBaseEntityByEntIndex")]
	pub fn entity_by_index(self, index: c_int) -> Option<Entity<'s>> {
		if !(0..crate::edicts::MAX_EDICTS).contains(&index) {
			return None;
		}

		// SAFETY: `Server::new` guarantees the interface is live.
		let entity =
			unsafe { vcall!(self.as_ptr() => IServerTools_GetBaseEntityByEntIndex(index)) };

		// SAFETY: Entities are not freed immediately during `'s`.
		NonNull::new(entity).map(|entity| unsafe { Entity::from_raw(entity) })
	}

	/// Looks up the entity a handle refers to, or `None` if it no longer exists.
	#[doc(alias = "LookupEntity")]
	pub fn entity_by_handle(self, handle: EntityHandle) -> Option<Entity<'s>> {
		let index = handle
			.index()
			.filter(|&index| index < EntityHandle::SLOTS)?;

		// SAFETY: As for `entity_by_index`.
		let list = NonNull::new(unsafe { vcall!(self.as_ptr() => IServerTools_GetEntityList()) })?;

		// SAFETY: `gEntList` is a static of the game DLL, and the index is
		// within `m_EntPtrArray`. Entries are read without forming references.
		let (entity, serial_number) = unsafe {
			let info = (&raw const (*list.as_ptr())._base.m_EntPtrArray)
				.cast::<sys::CEntInfo>()
				.add(index);

			(
				(&raw const (*info).m_pEntity).read(),
				(&raw const (*info).m_SerialNumber).read(),
			)
		};

		if u32::try_from(serial_number).ok()? != handle.serial_number() {
			return None;
		}

		// SAFETY: `CBaseEntity`'s primary base derives from `IHandleEntity`, so
		// the pointers coincide, and the server's list only holds entities.
		NonNull::new(entity).map(|entity| unsafe { Entity::from_raw(entity.cast()) })
	}

	/// Iterates over every entity, including server-only ones and those
	/// pending deletion.
	#[doc(alias = "FirstEntity")]
	#[doc(alias = "NextEntity")]
	pub fn entities(self) -> Entities<'s> {
		Entities {
			tools: self,
			state: IterState::First,
		}
	}

	/// Finds the next entity after `after` whose class name matches
	/// `class_name`, which may end in a `*` wildcard.
	#[doc(alias = "FindEntityByClassname")]
	pub fn find_by_class_name(
		self,
		after: Option<Entity<'_>>,
		class_name: &CStr,
	) -> Option<Entity<'s>> {
		let after = after.map_or(ptr::null_mut(), Entity::as_ptr);

		// SAFETY: As for `entity_by_index`, and `after` is live or null.
		let entity = unsafe {
			vcall!(self.as_ptr() => IServerTools_FindEntityByClassname(after, class_name.as_ptr()))
		};

		// SAFETY: As for `entity_by_index`.
		NonNull::new(entity).map(|entity| unsafe { Entity::from_raw(entity) })
	}

	/// Reads one of an entity's key values, as formatted by its datamap.
	///
	/// Values longer than 1023 bytes are truncated.
	#[doc(alias = "GetKeyValue")]
	pub fn key_value(self, entity: Entity<'_>, key: &CStr) -> Option<CString> {
		let mut buffer = [0 as c_char; KEY_VALUE_CAPACITY];

		// SAFETY: As for `entity_by_index`, and the buffer length is passed.
		let found = unsafe {
			vcall!(self.as_ptr() => IServerTools_GetKeyValue(entity.as_ptr(), key.as_ptr(), buffer.as_mut_ptr(), KEY_VALUE_CAPACITY as c_int))
		};

		found.then(|| cstring_from_buffer(&buffer))
	}

	/// Requests Source's deferred removal of an entity.
	///
	/// The entity stays allocated, and may still appear in iteration, until
	/// the engine frees it at the end of the frame. Physics callbacks may also
	/// defer setting its deletion flag; check
	/// [`Entity::is_marked_for_deletion`] for the current state.
	#[doc(alias = "RemoveEntity")]
	#[doc(alias = "UTIL_Remove")]
	pub fn remove(self, entity: Entity<'_>) {
		if entity.is_marked_for_deletion() {
			return;
		}

		// SAFETY: As for `entity_by_index`. Removal is deferred.
		unsafe { vcall!(self.as_ptr() => IServerTools_RemoveEntity(entity.as_ptr())) };
	}

	/// Moves an entity through Source's `Teleport` method, which also updates
	/// its physics state and may move child entities. Each argument left as
	/// `None` is unchanged.
	#[doc(alias = "Teleport")]
	pub fn teleport(
		self,
		entity: Entity<'_>,
		origin: Option<Vector>,
		angles: Option<QAngle>,
		velocity: Option<Vector>,
	) -> Result<(), TeleportError> {
		entity.teleport(self.game.teleport_vtable_slot(), origin, angles, velocity)
	}
}

// SAFETY: `sys::IServerTools` is the class exported under `VSERVERTOOLS003`.
unsafe impl<'s> Interface<'s> for ServerTools<'s> {
	type Raw = sys::IServerTools;

	const MODULE: Module = Module::GameServer;
	const VERSION: &'static CStr = Self::VERSION;

	unsafe fn bind(raw: NonNull<sys::IServerTools>, server: &Server<'s>) -> Self {
		// SAFETY: The caller upholds the contract, and `Server::new` guarantees
		// the game DLL was built for `server.game()`.
		unsafe { Self::from_raw(raw, server.game()) }
	}
}

/// Iterator over every entity, from [`ServerTools::entities`].
#[derive(Debug, Clone)]
pub struct Entities<'s> {
	tools: ServerTools<'s>,
	state: IterState,
}

#[derive(Debug, Clone, Copy)]
enum IterState {
	First,
	After(NonNull<sys::CBaseEntity>),
	Done,
}

impl<'s> Iterator for Entities<'s> {
	type Item = Entity<'s>;

	fn next(&mut self) -> Option<Self::Item> {
		let tools = self.tools.as_ptr();

		// SAFETY: As for `ServerTools::entity_by_index`. The previous entity is
		// still allocated, since entities are not freed immediately during `'s`.
		let entity = match self.state {
			IterState::First => unsafe { vcall!(tools => IServerTools_FirstEntity()) },
			IterState::After(previous) => unsafe {
				vcall!(tools => IServerTools_NextEntity(previous.as_ptr()))
			},
			IterState::Done => return None,
		};

		match NonNull::new(entity) {
			Some(entity) => {
				self.state = IterState::After(entity);

				// SAFETY: As for `ServerTools::entity_by_index`.
				Some(unsafe { Entity::from_raw(entity) })
			}

			None => {
				self.state = IterState::Done;
				None
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::entities::test_support::MockEntity;
	use crate::ffi::test_support::{mock_vtable, unexpected_call};
	use std::cell::Cell;
	use std::ptr::null_mut;

	thread_local! {
		static ENTITY: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };
		static LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };
		static REMOVALS: Cell<usize> = const { Cell::new(0) };
	}

	unsafe extern "C" fn first_entity(_: *mut sys::IServerTools) -> *mut sys::CBaseEntity {
		ENTITY.get()
	}

	unsafe extern "C" fn next_entity(
		_: *mut sys::IServerTools,
		_: *mut sys::CBaseEntity,
	) -> *mut sys::CBaseEntity {
		null_mut()
	}

	unsafe extern "C" fn entity_by_index(
		_: *mut sys::IServerTools,
		index: c_int,
	) -> *mut sys::CBaseEntity {
		if index == 1 { ENTITY.get() } else { null_mut() }
	}

	unsafe extern "C" fn entity_list(_: *mut sys::IServerTools) -> *mut sys::CGlobalEntityList {
		LIST.get()
	}

	unsafe extern "C" fn remove_entity(_: *mut sys::IServerTools, entity: *mut sys::CBaseEntity) {
		assert_eq!(entity, ENTITY.get());
		REMOVALS.set(REMOVALS.get() + 1);
		unsafe {
			entity
				.byte_add(crate::entities::test_support::MOCK_EFLAGS_OFFSET)
				.cast::<c_int>()
				.write(1)
		};
	}

	#[test]
	fn tools_find_iterate_and_remove_entities() {
		let mut mock = MockEntity::new(1 | 9 << 16);
		ENTITY.set(mock.as_ptr());
		REMOVALS.set(0);

		let vtable = unsafe {
			mock_vtable::<sys::IServerTools__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IServerTools_FirstEntity).write(first_entity);
					(&raw mut (*vtable).IServerTools_NextEntity).write(next_entity);
					(&raw mut (*vtable).IServerTools_GetBaseEntityByEntIndex)
						.write(entity_by_index);
					(&raw mut (*vtable).IServerTools_GetEntityList).write(entity_list);
					(&raw mut (*vtable).IServerTools_RemoveEntity).write(remove_entity);
				},
			)
		};

		let mut interface = sys::IServerTools {
			vtable_: &raw const *vtable,
		};
		let tools =
			unsafe { ServerTools::from_raw(NonNull::from(&mut interface), Game::TeamFortress2) };

		// Slot 1 holds the entity at serial number 9.
		let mut list = Box::<sys::CGlobalEntityList>::new_zeroed();
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

		tools.remove(entity);
		tools.remove(entity);

		assert!(entity.is_marked_for_deletion());
		assert_eq!(REMOVALS.get(), 1);
		assert_eq!(
			tools.teleport(entity, Some(Vector::new(0.0, 0.0, 0.0)), None, None),
			Err(TeleportError::MarkedForDeletion)
		);

		ENTITY.set(null_mut());
		LIST.set(null_mut());
	}
}
