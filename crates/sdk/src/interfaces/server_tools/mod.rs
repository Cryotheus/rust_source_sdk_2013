//! `IServerTools`, which enumerates and manipulates the server's entities.

#[cfg(test)]
#[path = "../../tests/interfaces/server_tools.rs"]
mod tests;

use crate::NotThreadSafe;
use crate::entities::health::HealthInput;
use crate::entities::{Entity, EntityHandle, HammerId, ProtectedEntity, TeleportError};
use crate::inputs::{self, InputError, InputValue};
use crate::interfaces::TempEntities;
use crate::math::{QAngle, Vector};
use crate::server::{Game, Interface, Module, Server};
use sdk_raw::util::cstr::{borrow_cstr, copy_cstr, cstring_from_buffer};
use sdk_raw::vcall;
use std::ffi::{CStr, CString, c_char, c_int};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};

/// The largest key value [`ServerTools::key_value`] reads, including its terminator.
const KEY_VALUE_CAPACITY: usize = 1024;

/// Iterator over every entity, from [`ServerTools::entities`].
#[derive(Debug, Clone)]
pub struct Entities<'s> {
	tools: ServerTools<'s>,
	state: IterState,
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

/// How far an [`Entities`] iterator has come through the entity list.
#[derive(Debug, Clone, Copy)]
enum IterState {
	/// Nothing has been yielded; the next call starts with `FirstEntity`.
	First,

	/// The entity yielded last, which `NextEntity` continues from.
	After(NonNull<sys::CBaseEntity>),

	/// The game returned null, so the iterator is exhausted.
	Done,
}

/// Entity enumeration and manipulation meant for tools (`IServerTools`).
///
/// Unlike edict lookups, this reaches server-only entities too.
#[doc(alias("IServerTools"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ServerTools<'s> {
	raw: NonNull<sys::IServerTools>,
	game: Game,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> ServerTools<'s> {
	/// The version string this interface is requested by.
	pub const VERSION: &'static CStr = sdk_raw::interfaces::server_tools::VERSION;

	/// Wraps the interface of a game DLL built for `game`, which selects
	/// game-specific vtable slots such as `Teleport`'s.
	///
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
	/// break that condition, or to crash the server or its clients, whatever
	/// the map does, and are refused; send them with
	/// [`Self::accept_input_unchecked`]:
	///
	/// - [`InputError::FreesEntities`]: inputs that run code the caller
	///   chooses or spawn entities from templates (`RunScriptCode`,
	///   `RunScriptFile`, `CallScriptFunction`, `ForceSpawn`,
	///   `ForceSpawnAtEntityOrigin`, and NPC makers' spawn inputs). A template
	///   entity failing to spawn empties the engine's pending-deletion list,
	///   and scripts can also restart the round.
	/// - [`InputError::PickerName`]: the string `"!picker"`, which the game
	///   resolves through the first player without checking that there is one.
	/// - [`InputError::UncheckedLookup`]: inputs that use a lookup of their
	///   value unchecked, such as TF2's `SpeakResponseConcept`, which reads out
	///   of bounds for a concept name the game does not know.
	/// - [`InputError::InvalidMaxHealth`]: `SetHealth` on one of TF2's
	///   buildings, which sets its maximum health too, and `AddOutput` with
	///   the `max_health` key, with a value of 0 or less, which game code on
	///   the server and on clients divides integers by, or that TF2 cannot
	///   convert back from a building's float health.
	/// - [`InputError::InvalidKeyValue`]: `AddOutput` with the other key
	///   values [`Self::set_key_value`] refuses, such as a team number the
	///   game indexes an array with unchecked.
	/// - [`InputError::InvalidTeam`]: `SetTeam` on an entity other than a
	///   player, whose `ChangeTeam` checks the team itself, with a team
	///   outside 0 to 3, which the game indexes per-team arrays with
	///   unchecked, and any `SetTeam` on a team entity, which clients look up
	///   by its team number.
	/// - [`InputError::ProtectedEntity`]: `Kill` and `KillHierarchy` on the
	///   world, a player, or a soundscape, which [`Self::remove`] refuses too.
	///
	/// The last is a precaution, not a guarantee: removing an entity also
	/// removes the entities parented to it, and outputs added with `AddOutput`
	/// can remove any entity later. Removing a player, the world, or another
	/// entity the game keeps pointers to, such as the game rules or a team,
	/// crashes the server once it is freed, as with [`Self::remove`].
	#[doc(alias("AcceptInput", "AcceptEntityInput"))]
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
	#[doc(alias("AcceptInput"))]
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

	/// Returns the interface pointer, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::IServerTools {
		self.raw.as_ptr()
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
	#[doc(alias("CreateEntityByName"))]
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
	#[doc(alias("DispatchSpawn"))]
	pub unsafe fn dispatch_spawn(self, entity: Entity<'_>) {
		// SAFETY: As for `entity_by_index`, and the caller vouches for `Spawn`.
		unsafe { vcall!(self.as_ptr() => IServerTools_DispatchSpawn(entity.as_ptr())) };
	}

	/// Iterates over every entity, including server-only ones and those
	/// pending deletion.
	#[doc(alias("FirstEntity", "NextEntity"))]
	pub fn entities(self) -> Entities<'s> {
		Entities {
			tools: self,
			state: IterState::First,
		}
	}

	/// Looks up the entity a handle refers to, or `None` if it no longer exists.
	#[doc(alias("LookupEntity"))]
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

	/// Looks up a networked entity by edict index. Returns `None` for a
	/// negative index, one of at least [`MAX_EDICTS`](crate::edicts::MAX_EDICTS),
	/// or one that holds no entity.
	///
	/// Server-only entities have no edict index; find them with
	/// [`Self::entities`] or [`Self::entity_by_handle`].
	#[doc(alias("GetBaseEntityByEntIndex"))]
	pub fn entity_by_index(self, index: c_int) -> Option<Entity<'s>> {
		if !(0..sdk_raw::edicts::MAX_EDICTS).contains(&index) {
			return None;
		}

		// SAFETY: `Server::new` guarantees the interface is live.
		let entity =
			unsafe { vcall!(self.as_ptr() => IServerTools_GetBaseEntityByEntIndex(index)) };

		// SAFETY: Entities are not freed immediately during `'s`.
		NonNull::new(entity).map(|entity| unsafe { Entity::from_raw(entity) })
	}

	/// Finds the next entity after `after`, or from the start of the entity
	/// list if it is `None`, whose class name matches `class_name`, which may
	/// end in a `*` wildcard. Returns `None` if no later entity matches.
	#[doc(alias("FindEntityByClassname"))]
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

	/// Finds the first entity in the entity list whose Hammer ID is `id`,
	/// including server-only entities and those pending deletion, or `None` if
	/// no entity has it.
	///
	/// Entities a `point_template` spawns share their template entity's ID. To
	/// find every entity with an ID, filter [`Self::entities`] by
	/// [`Entity::hammer_id`].
	#[doc(alias("FindEntityByHammerID"))]
	pub fn find_by_hammer_id(self, id: HammerId) -> Option<Entity<'s>> {
		// SAFETY: As for `entity_by_index`. The game only compares each entity's
		// ID with this one.
		let entity =
			unsafe { vcall!(self.as_ptr() => IServerTools_FindEntityByHammerID(id.get())) };

		// SAFETY: As for `entity_by_index`.
		NonNull::new(entity).map(|entity| unsafe { Entity::from_raw(entity) })
	}

	/// Reads one of an entity's key values, as formatted by its datamap.
	/// Returns `None` if the game finds no key with the name, or cannot format
	/// the key's type.
	///
	/// Keys of string fields, such as `damagefilter` and `model`, are read from
	/// the field, since `GetKeyValue` copies the bytes of the string's pointer
	/// instead of its text. An unset string reads as empty. Other values longer
	/// than 1023 bytes are truncated.
	#[doc(alias("GetKeyValue"))]
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

	/// Adds a string to the game's string pool (`AllocPooledString`), where it
	/// stays until the level ends, or the next round restart for a string equal
	/// to a removed template entity's unique name.
	///
	/// No interface exposes the pool, so this sets the world's `targetname`
	/// key, which the game pools, reads the pooled name back, and restores the
	/// world's name, as SourceMod does. The name is not networked, and nothing
	/// else observes the change.
	///
	/// Returns `None` if the world is missing or marked for deletion, its name
	/// field is not found, or the game did not pool the string as given.
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
	#[doc(alias("RemoveEntity", "UTIL_Remove"))]
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

	/// Sends one of the inputs through which maps change an entity's health,
	/// with [`Self::accept_input`], which runs the game logic of the entity's
	/// class, such as breaking a `func_breakable` or killing a player, before
	/// returning.
	///
	/// The [health module](crate::entities::health#setting-health) lists the
	/// classes that declare these inputs and what each does, and compares
	/// them with [`Entity::set_health`], which runs no game logic. Entities
	/// of other classes refuse them with [`InputError::UnknownInput`], and
	/// [`InputError::InvalidMaxHealth`] refuses [`HealthInput::Set`] with a
	/// maximum health TF2's buildings cannot have, as [`Self::accept_input`]
	/// describes.
	#[doc(alias("SetHealth", "AddHealth", "RemoveHealth", "SetMaxHealth"))]
	pub fn send_health_input(
		self,
		target: Entity<'_>,
		input: HealthInput,
		activator: Entity<'_>,
		caller: Entity<'_>,
	) -> Result<(), InputError> {
		self.accept_input(
			target,
			input.name(),
			InputValue::Int(input.amount()),
			activator,
			caller,
		)
	}

	/// Converts `value` for a checked input, pooling a string the input may
	/// keep, and sends it through `AcceptInput`. Fails with
	/// [`InputError::NotPooled`] if such a string could not be pooled, or
	/// [`InputError::Rejected`] if `AcceptInput` returns false.
	///
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

	/// Sets one of an entity's key values, as a map's entity lump does before
	/// the entity spawns. Returns whether the game set it: `false` if the
	/// entity does not know the key, or if the value is refused, as below.
	///
	/// The key and value are copied first, since the game writes into keys
	/// containing `#`: it ignores the key from its first `#`.
	///
	/// # Refused values
	///
	/// Values known to crash the server or its clients, or to corrupt their
	/// memory, are not set, and `false` is returned. [`Self::accept_input`]
	/// refuses the `AddOutput` input, which sets a key value too, for the same
	/// values, with [`InputError::InvalidMaxHealth`] for `max_health` and
	/// [`InputError::InvalidKeyValue`] for the others.
	///
	/// Keys are compared as the game compares them: ignoring ASCII case, and
	/// what follows their first `#`, except for the keys of per-team data
	/// below, which their classes compare as given. Values are read with
	/// `atoi`, so one beyond `int`'s range, for which `atoi` is undefined, is
	/// refused for each key listed. Team numbers must lie within 0 to 3, the
	/// teams of TF2 and of Source SDK 2013's templates, which the game indexes
	/// its per-team arrays with unchecked; that excludes TF2's Halloween team,
	/// 5.
	///
	/// - `max_health`: 0 or less, which game code divides integers by, such as
	///   TF2's Horseless Headless Horsemann on the server. On one of TF2's
	///   buildings, also a value that rounds to 2^31 as a float, which TF2
	///   cannot convert back from the building's float health.
	/// - `teamnumber`: an invalid team number. On a player, every value, since
	///   it would stay in its old team's list of players, which keeps pointing
	///   to it once it is freed. On a team entity, every value, since clients
	///   look teams up by their number without checking that they are found.
	/// - On a `team_control_point` (`CTeamControlPoint`): `point_index`
	///   outside 0 to 7, `point_default_owner` an invalid team number, and
	///   the keys `team_capsound_<team>`, `team_model_`, `team_timedpoints_`,
	///   `team_bodygroup_`, `team_icon_` and `team_overlay_` with an invalid
	///   team number, which the entity writes past its per-team data during the
	///   call. `team_previouspoint_<team>_<index>` is read with `sscanf`, which
	///   leaves the team uninitialized if it is missing, and also needs an
	///   index within 0 to 2, or none.
	/// - On a `trigger_capture_area` (`CTriggerAreaCapture`): the keys
	///   `team_numcap_<team>`, `team_cancap_`, `team_spawn_` and
	///   `team_startcap_`, with an invalid team number.
	/// - On a `team_control_point_master` (`CTeamControlPointMaster`):
	///   `team_base_icon_<team>` with a team outside 0 to 31.
	/// - On TF2's `tf_robot_destruction_robot_spawn`: `type` outside 0 to 2.
	///
	/// Other key values may still break the game, such as `hitboxset`, which
	/// the server and clients index the hitbox sets of the entity's model
	/// with, unchecked, and which is not refused, since the number of sets
	/// depends on the model.
	#[doc(alias("SetKeyValue"))]
	pub fn set_key_value(self, entity: Entity<'_>, key: &CStr, value: &CStr) -> bool {
		if inputs::check_key_value(entity, key, value).is_err() {
			return false;
		}

		let key = key.to_owned();
		let value = value.to_owned();

		// SAFETY: As for `entity_by_index`. The game may write into the copies,
		// which are only used for the call.
		unsafe {
			vcall!(self.as_ptr() => IServerTools_SetKeyValue(entity.as_ptr(), key.as_ptr(), value.as_ptr()))
		}
	}

	/// The game's temporary entities, which send effects to clients once, or
	/// `None` if the game has none.
	#[doc(alias("GetTempEntsSystem"))]
	pub fn temp_entities(self) -> Option<TempEntities<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let raw = unsafe { vcall!(self.as_ptr() => IServerTools_GetTempEntsSystem()) };

		// SAFETY: The game returns null or its `te`, a static of the game DLL,
		// which stays loaded for `'s`, and the caller is on the main thread.
		NonNull::new(raw).map(|raw| unsafe { TempEntities::from_raw(raw) })
	}

	/// Moves an entity through Source's `Teleport` method, which also updates
	/// its physics state and may move child entities. Each argument left as
	/// `None` is unchanged.
	///
	/// Fails without moving the entity if a given component is not finite, or
	/// the entity is marked for deletion.
	#[doc(alias("Teleport"))]
	pub fn teleport(
		self,
		entity: Entity<'_>,
		origin: Option<Vector>,
		angles: Option<QAngle>,
		velocity: Option<Vector>,
	) -> Result<(), TeleportError> {
		// SAFETY: `from_raw`'s caller guarantees the game DLL was built for
		// `self.game`, as `bind` does from `Server::new` condition 2, so its
		// slot is where that DLL's primary `CBaseEntity` vtable has `Teleport`.
		unsafe { entity.teleport(self.game.teleport_vtable_slot(), origin, angles, velocity) }
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
