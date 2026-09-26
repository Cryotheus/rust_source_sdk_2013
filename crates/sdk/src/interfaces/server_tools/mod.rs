//! `IServerTools`, which enumerates and manipulates the server's entities.

use crate::entities::{Entity, EntityHandle, ProtectedEntity, TeleportError};
use crate::ffi::{NotThreadSafe, borrow_cstr, copy_cstr, cstring_from_buffer, vcall};
use crate::inputs::{self, InputError, InputValue};
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
	/// Keys of string fields, such as `damagefilter` and `model`, are read from
	/// the field, since `GetKeyValue` copies the bytes of the string's pointer
	/// instead of its text. An unset string reads as empty. Other values longer
	/// than 1023 bytes are truncated.
	#[doc(alias = "GetKeyValue")]
	pub fn key_value(self, entity: Entity<'_>, key: &CStr) -> Option<CString> {
		if let Some(field) = entity.string_key_field(key) {
			// SAFETY: The field belongs to the live entity, and is read without
			// forming a reference, as the game writes it too.
			let string = unsafe { field.read() };

			// SAFETY: String fields hold null, which `STRING` reads as empty, or a
			// string the game may read at any time, such as a pooled one. It is
			// copied immediately.
			return Some(unsafe { copy_cstr(string.pszValue) }.unwrap_or_default());
		}

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
	///
	/// Refuses the world, players, and soundscapes, which the game keeps using
	/// after they are freed. Other entities the game keeps pointers to, such as
	/// the game rules or a team, crash the server the same way once freed.
	#[doc(alias = "RemoveEntity")]
	#[doc(alias = "UTIL_Remove")]
	pub fn remove(self, entity: Entity<'_>) -> Result<(), ProtectedEntity> {
		if entity.is_protected() {
			return Err(ProtectedEntity);
		}

		if !entity.is_marked_for_deletion() {
			// SAFETY: As for `entity_by_index`. Removal is deferred.
			unsafe { vcall!(self.as_ptr() => IServerTools_RemoveEntity(entity.as_ptr())) };
		}

		Ok(())
	}

	/// Creates an entity of a class, without spawning it, as the game's
	/// `CreateEntityByName` does. Returns `None` for an unknown class.
	///
	/// Set its key values with [`Self::set_key_value`], then spawn it with
	/// [`Self::dispatch_spawn`].
	///
	/// # Safety
	///
	/// The class's constructor must free entities only through Source's
	/// deferred deletion (condition 4 of [`Server::new`]).
	#[doc(alias = "CreateEntityByName")]
	pub unsafe fn create_entity_by_name(self, class_name: &CStr) -> Option<Entity<'s>> {
		// SAFETY: As for `entity_by_index`, and the caller vouches for the
		// constructor. The game adds the entity to its entity list.
		let entity = unsafe {
			vcall!(self.as_ptr() => IServerTools_CreateEntityByName(class_name.as_ptr()))
		};

		// SAFETY: As above, and nothing frees it during `'s`.
		NonNull::new(entity).map(|entity| unsafe { Entity::from_raw(entity) })
	}

	/// Spawns an entity created with [`Self::create_entity_by_name`], as the
	/// game's `DispatchSpawn` does. Its `Spawn`, which usually precaches what
	/// it needs, runs before this returns.
	///
	/// # Safety
	///
	/// Everything the entity's `Spawn` runs must free entities only through
	/// Source's deferred deletion (condition 4 of [`Server::new`]), and the
	/// entity must not have been spawned before.
	#[doc(alias = "DispatchSpawn")]
	pub unsafe fn dispatch_spawn(self, entity: Entity<'_>) {
		// SAFETY: As for `entity_by_index`, and the caller vouches for `Spawn`.
		unsafe { vcall!(self.as_ptr() => IServerTools_DispatchSpawn(entity.as_ptr())) };
	}

	/// Sets one of an entity's key values, as a map's entity lump does before
	/// the entity spawns. Returns whether the entity knew the key.
	///
	/// The key and value are copied first, since the game writes into keys
	/// containing `#`.
	#[doc(alias = "SetKeyValue")]
	pub fn set_key_value(self, entity: Entity<'_>, key: &CStr, value: &CStr) -> bool {
		let key = key.to_owned();
		let value = value.to_owned();

		// SAFETY: As for `entity_by_index`. The game may write into the copies,
		// which are only used for the call.
		unsafe {
			vcall!(self.as_ptr() => IServerTools_SetKeyValue(entity.as_ptr(), key.as_ptr(), value.as_ptr()))
		}
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

	/// Sends an input to an entity through Source's `AcceptInput`, as map I/O
	/// and `ent_fire` do, and runs its handler before returning.
	///
	/// `activator` and `caller` are the entities the handler, and the outputs
	/// it fires, see as `!activator` and `!caller`. Some handlers dereference
	/// them without checking for null, so both are required; the game's own
	/// calls often pass the target for both. The output ID is 0, as for
	/// VScript's `AcceptInput`, so the `Use` input receives `USE_OFF`.
	///
	/// The input is found and the value checked against it first, as
	/// `AcceptInput` would, and the input is sent by the name it is declared
	/// with. `Ok` means `AcceptInput` found the input and converted the value;
	/// a map script's `Input<Name>` hook may still have skipped the handler,
	/// and the handler may ignore the value.
	///
	/// # Refused inputs
	///
	/// A handler runs arbitrary game code, including map outputs, VScript
	/// hooks, and other plugins' hooks, which condition 4 of
	/// [`Server::new`](crate::Server::new) covers. Some inputs are known to
	/// break that condition, or to crash the server, whatever the map does,
	/// and are refused; send them with [`Self::accept_input_unchecked`]:
	///
	/// - [`InputError::FreesEntities`]: inputs that run code the caller
	///   chooses or spawn entities from templates (`RunScriptCode`,
	///   `RunScriptFile`, `CallScriptFunction`, `ForceSpawn`,
	///   `ForceSpawnAtEntityOrigin`, and NPC makers' spawn inputs). A template
	///   entity failing to spawn empties the engine's pending-deletion list,
	///   and scripts can also restart the round.
	/// - [`InputError::PickerName`]: the string `"!picker"`, which the game
	///   resolves through the first player without checking that there is one.
	/// - [`InputError::ProtectedEntity`]: `Kill` and `KillHierarchy` on the
	///   world, a player, or a soundscape, which [`Self::remove`] refuses too.
	///
	/// The last is a precaution, not a guarantee: removing an entity also
	/// removes the entities parented to it, and outputs added with `AddOutput`
	/// can remove any entity later. Removing a player, the world, or another
	/// entity the game keeps pointers to, such as the game rules or a team,
	/// crashes the server once it is freed, as with [`Self::remove`].
	#[doc(alias = "AcceptInput")]
	#[doc(alias = "AcceptEntityInput")]
	pub fn accept_input(
		self,
		target: Entity<'_>,
		input: &CStr,
		value: InputValue<'_>,
		activator: Entity<'_>,
		caller: Entity<'_>,
	) -> Result<(), InputError> {
		let checked = inputs::check_input(target, input, value)?;

		inputs::check_guards(target, checked, value)?;

		// SAFETY: Inputs known to free entities immediately, and values known to
		// make the game dereference null, were refused, and neither entity is
		// null. Condition 4 of `Server::new` covers the code the remaining
		// inputs run.
		unsafe { self.send_input(target, checked, value, Some(activator), Some(caller)) }
	}

	/// Sends an input like [`Self::accept_input`], without refusing the inputs
	/// and values it refuses, and with an optional activator and caller.
	///
	/// # Safety
	///
	/// Everything the input runs before returning, including VScript and the
	/// entities it spawns, must free entities only through Source's deferred
	/// deletion, and must not restart the round or change the level. The
	/// input's handler must accept the given activator and caller, since some
	/// dereference them without checking for null, and the value must not make
	/// the game dereference a missing entity, as `"!picker"` does when the
	/// first player slot is empty.
	#[doc(alias = "AcceptInput")]
	pub unsafe fn accept_input_unchecked(
		self,
		target: Entity<'_>,
		input: &CStr,
		value: InputValue<'_>,
		activator: Option<Entity<'_>>,
		caller: Option<Entity<'_>>,
	) -> Result<(), InputError> {
		let checked = inputs::check_input(target, input, value)?;

		// SAFETY: The caller upholds the contract.
		unsafe { self.send_input(target, checked, value, activator, caller) }
	}

	/// # Safety
	///
	/// As for [`Entity::accept_input`], apart from the string, which this
	/// pools when the input may keep it.
	unsafe fn send_input(
		self,
		target: Entity<'_>,
		input: inputs::CheckedInput<'_>,
		value: InputValue<'_>,
		activator: Option<Entity<'_>>,
		caller: Option<Entity<'_>>,
	) -> Result<(), InputError> {
		let variant = inputs::to_variant(input, value, |string| self.pool_string(string))?;

		// SAFETY: The caller upholds the contract, and strings the input may
		// keep are pooled.
		if unsafe { target.accept_input(input.name(), variant, activator, caller) } {
			Ok(())
		} else {
			Err(InputError::Rejected)
		}
	}

	/// Adds a string to the game's string pool (`AllocPooledString`), where it
	/// stays until the level ends, or the next round restart for a string equal
	/// to a removed template entity's unique name.
	///
	/// No interface exposes the pool, so this sets the world's `targetname`
	/// key, which the game pools, reads the pooled name back, and restores the
	/// world's name, as SourceMod does. The name is not networked, and nothing
	/// else observes the change.
	fn pool_string(self, string: &CStr) -> Option<sys::string_t> {
		let world = self
			.entity_by_index(0)
			.filter(|world| !world.is_marked_for_deletion())?;
		let name = world.name_field()?;

		// SAFETY: The field belongs to the live world entity, and is read and
		// written without forming references, as the game writes it too.
		let previous = unsafe { name.read() };

		// SAFETY: As for `entity_by_index`. `KeyValue` only writes into a key
		// containing `#`, which this one does not.
		let set = unsafe {
			vcall!(self.as_ptr() => IServerTools_SetKeyValue(world.as_ptr(), c"targetname".as_ptr(), string.as_ptr()))
		};

		// SAFETY: As above.
		let pooled = unsafe {
			let pooled = name.read();

			name.write(previous);
			pooled
		};

		// SAFETY: Pooled strings stay allocated at least until the round ends,
		// which condition 4 of `Server::new` rules out during `'s`.
		let copy = unsafe { borrow_cstr(pooled.pszValue) }?;

		(set && copy.to_bytes().eq_ignore_ascii_case(string.to_bytes())).then_some(pooled)
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
	use crate::entities::FTYPEDESC_KEY;
	use crate::entities::test_support::{
		MockEntity, base_entity_fields, data_map, field, set_datamap,
	};
	use crate::ffi::test_support::{mock_vtable, unexpected_call};
	use std::cell::{Cell, RefCell};
	use std::ptr::null_mut;

	thread_local! {
		static ENTITY: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };
		static LIST: Cell<*mut sys::CGlobalEntityList> = const { Cell::new(null_mut()) };
		static REMOVALS: Cell<usize> = const { Cell::new(0) };
		static KEYS_READ: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
	}

	/// Formats every key but `missing` as `100`, as for an integer field.
	unsafe extern "C" fn get_key_value(
		_: *mut sys::IServerTools,
		_: *mut sys::CBaseEntity,
		key: *const c_char,
		value: *mut c_char,
		capacity: c_int,
	) -> bool {
		let key = unsafe { CStr::from_ptr(key) }.to_owned();
		let found = key.as_c_str() != c"missing";

		assert_eq!(capacity as usize, KEY_VALUE_CAPACITY);
		KEYS_READ.with_borrow_mut(|keys| keys.push(key));

		if found {
			unsafe { value.copy_from_nonoverlapping(c"100".as_ptr(), 4) };
		}

		found
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

	fn key(
		name: &'static CStr,
		field_type: sys::fieldtype_t,
		offset: usize,
	) -> sys::typedescription_t {
		let mut key = field();

		key.fieldType = field_type;
		key.fieldOffset[0] = offset as c_int;
		key.fieldSize = 1;
		key.flags = FTYPEDESC_KEY;
		key.externalName = name.as_ptr();
		key
	}

	fn embedded(map: *mut sys::datamap_t, offset: usize, count: u16) -> sys::typedescription_t {
		let mut embedded = field();

		embedded.fieldType = sys::_fieldtypes_FIELD_EMBEDDED;
		embedded.fieldOffset[0] = offset as c_int;
		embedded.fieldSize = count;
		embedded.td = map;
		embedded
	}

	#[test]
	fn key_values_of_string_fields_are_read_from_the_field() {
		use sys::{
			_fieldtypes_FIELD_INTEGER as INTEGER, _fieldtypes_FIELD_MODELNAME as MODELNAME,
			_fieldtypes_FIELD_SOUNDNAME as SOUNDNAME, _fieldtypes_FIELD_STRING as STRING,
		};

		let mut mock = MockEntity::new(1 | 9 << 16);
		let raw = mock.as_ptr();
		let set_string = |offset: usize, string: &'static CStr| unsafe {
			raw.byte_add(offset)
				.cast::<sys::string_t>()
				.write(sys::string_t {
					pszValue: string.as_ptr(),
				})
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

		let vtable = unsafe {
			mock_vtable::<sys::IServerTools__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| (&raw mut (*vtable).IServerTools_GetKeyValue).write(get_key_value),
			)
		};

		let mut interface = sys::IServerTools {
			vtable_: &raw const *vtable,
		};
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
}
