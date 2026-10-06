//! Entities in the server's entity list.
//!
//! [`ServerTools`](crate::interfaces::ServerTools) finds entities, including
//! server-only ones. Properties use the native entity interfaces instead of
//! key values, and networked variables are reached through
//! [`NetProp`](crate::datatables::NetProp). Health, life state and damage
//! modes are in [`health`], solid flags in [`solid`], and think contexts in
//! [`think`].

pub mod health;
pub mod solid;
pub mod think;

#[cfg(test)]
#[path = "tests/entities.rs"]
mod tests;

use crate::datatables::ServerClass;
use crate::edicts::Edict;
use crate::math::{QAngle, Vector};
use crate::{NotThreadSafe, Server};
use sdk_raw::entities::datamap::DataMaps;

use sdk_raw::entities::{
	COLLISION_GROUP_BREAKABLE_GLASS, COLLISION_GROUP_DEBRIS, COLLISION_GROUP_DEBRIS_TRIGGER,
	COLLISION_GROUP_DISSOLVING, COLLISION_GROUP_DOOR_BLOCKER, COLLISION_GROUP_IN_VEHICLE,
	COLLISION_GROUP_INTERACTIVE, COLLISION_GROUP_INTERACTIVE_DEBRIS, COLLISION_GROUP_NONE,
	COLLISION_GROUP_NPC, COLLISION_GROUP_NPC_ACTOR, COLLISION_GROUP_NPC_SCRIPTED,
	COLLISION_GROUP_PASSABLE_DOOR, COLLISION_GROUP_PLAYER, COLLISION_GROUP_PLAYER_MOVEMENT,
	COLLISION_GROUP_PROJECTILE, COLLISION_GROUP_PUSHAWAY, COLLISION_GROUP_VEHICLE,
	COLLISION_GROUP_VEHICLE_CLIP, COLLISION_GROUP_WEAPON, EFL_KILLME, ENT_ENTRY_MASK,
	INVALID_EHANDLE_INDEX, NUM_ENT_ENTRIES, NUM_SERIAL_NUM_SHIFT_BITS, TeleportSlot,
};

use sdk_raw::util::cstr::borrow_cstr;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};
use std::fmt::{self, Display, Formatter};
use std::marker::PhantomData;
use std::num::NonZero;
use std::ptr::{self, NonNull};
use std::sync::OnceLock;

/// Which entities an entity collides with (`Collision_Group_t`), and which
/// traces hit it, as the game rules' `ShouldCollide` decides, which games
/// refine.
///
/// These are the groups every game shares. Games define more past
/// [`LAST_SHARED_COLLISION_GROUP`](sdk_raw::entities::LAST_SHARED_COLLISION_GROUP),
/// as TF2's `TFCOLLISION_GROUP_*` are, which [`from_raw`](Self::from_raw) does
/// not convert.
#[doc(alias("Collision_Group_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollisionGroup {
	/// Collides with everything, as most entities do.
	#[doc(alias("COLLISION_GROUP_NONE"))]
	None,

	/// Collides only with entities of [`None`](Self::None), such as the
	/// world, and of [`Pushaway`](Self::Pushaway): players, bullets,
	/// projectiles and other debris pass through it.
	#[doc(alias("COLLISION_GROUP_DEBRIS"))]
	Debris,

	/// As [`Debris`](Self::Debris), but touches triggers.
	#[doc(alias("COLLISION_GROUP_DEBRIS_TRIGGER"))]
	DebrisTrigger,

	/// Collides with everything but debris, interactive or not.
	#[doc(alias("COLLISION_GROUP_INTERACTIVE_DEBRIS"))]
	InteractiveDebris,

	/// Collides with everything but interactive debris and debris.
	#[doc(alias("COLLISION_GROUP_INTERACTIVE"))]
	Interactive,

	/// Players.
	#[doc(alias("COLLISION_GROUP_PLAYER"))]
	Player,

	/// Breakable glass.
	#[doc(alias("COLLISION_GROUP_BREAKABLE_GLASS"))]
	BreakableGlass,

	/// Vehicles.
	#[doc(alias("COLLISION_GROUP_VEHICLE"))]
	Vehicle,

	/// The group player movement traces with, which TF2 uses to filter out
	/// other players and its buildings.
	#[doc(alias("COLLISION_GROUP_PLAYER_MOVEMENT"))]
	PlayerMovement,

	/// Non-player characters.
	#[doc(alias("COLLISION_GROUP_NPC"))]
	Npc,

	/// Entities inside a vehicle.
	#[doc(alias("COLLISION_GROUP_IN_VEHICLE"))]
	InVehicle,

	/// Weapons that need collision detection.
	#[doc(alias("COLLISION_GROUP_WEAPON"))]
	Weapon,

	/// Brushes that only block vehicles.
	#[doc(alias("COLLISION_GROUP_VEHICLE_CLIP"))]
	VehicleClip,

	/// Projectiles.
	#[doc(alias("COLLISION_GROUP_PROJECTILE"))]
	Projectile,

	/// Blocks the entities that may not come near moving doors.
	#[doc(alias("COLLISION_GROUP_DOOR_BLOCKER"))]
	DoorBlocker,

	/// Doors players do not collide with.
	#[doc(alias("COLLISION_GROUP_PASSABLE_DOOR"))]
	PassableDoor,

	/// Entities being dissolved.
	#[doc(alias("COLLISION_GROUP_DISSOLVING"))]
	Dissolving,

	/// Not solid, but pushed away by players' movement.
	#[doc(alias("COLLISION_GROUP_PUSHAWAY"))]
	Pushaway,

	/// Non-player characters in scripts, which ignore the player.
	#[doc(alias("COLLISION_GROUP_NPC_ACTOR"))]
	NpcActor,

	/// Non-player characters in scripts that should not collide with each
	/// other.
	#[doc(alias("COLLISION_GROUP_NPC_SCRIPTED"))]
	NpcScripted,
}

impl CollisionGroup {
	/// Converts a shared `COLLISION_GROUP_*` value, or returns `None` for
	/// another, such as one a game defines for itself.
	pub const fn from_raw(value: c_int) -> Option<Self> {
		match value {
			COLLISION_GROUP_NONE => Some(Self::None),
			COLLISION_GROUP_DEBRIS => Some(Self::Debris),
			COLLISION_GROUP_DEBRIS_TRIGGER => Some(Self::DebrisTrigger),
			COLLISION_GROUP_INTERACTIVE_DEBRIS => Some(Self::InteractiveDebris),
			COLLISION_GROUP_INTERACTIVE => Some(Self::Interactive),
			COLLISION_GROUP_PLAYER => Some(Self::Player),
			COLLISION_GROUP_BREAKABLE_GLASS => Some(Self::BreakableGlass),
			COLLISION_GROUP_VEHICLE => Some(Self::Vehicle),
			COLLISION_GROUP_PLAYER_MOVEMENT => Some(Self::PlayerMovement),
			COLLISION_GROUP_NPC => Some(Self::Npc),
			COLLISION_GROUP_IN_VEHICLE => Some(Self::InVehicle),
			COLLISION_GROUP_WEAPON => Some(Self::Weapon),
			COLLISION_GROUP_VEHICLE_CLIP => Some(Self::VehicleClip),
			COLLISION_GROUP_PROJECTILE => Some(Self::Projectile),
			COLLISION_GROUP_DOOR_BLOCKER => Some(Self::DoorBlocker),
			COLLISION_GROUP_PASSABLE_DOOR => Some(Self::PassableDoor),
			COLLISION_GROUP_DISSOLVING => Some(Self::Dissolving),
			COLLISION_GROUP_PUSHAWAY => Some(Self::Pushaway),
			COLLISION_GROUP_NPC_ACTOR => Some(Self::NpcActor),
			COLLISION_GROUP_NPC_SCRIPTED => Some(Self::NpcScripted),
			_ => None,
		}
	}

	/// The group's `COLLISION_GROUP_*` value.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::None => COLLISION_GROUP_NONE,
			Self::Debris => COLLISION_GROUP_DEBRIS,
			Self::DebrisTrigger => COLLISION_GROUP_DEBRIS_TRIGGER,
			Self::InteractiveDebris => COLLISION_GROUP_INTERACTIVE_DEBRIS,
			Self::Interactive => COLLISION_GROUP_INTERACTIVE,
			Self::Player => COLLISION_GROUP_PLAYER,
			Self::BreakableGlass => COLLISION_GROUP_BREAKABLE_GLASS,
			Self::Vehicle => COLLISION_GROUP_VEHICLE,
			Self::PlayerMovement => COLLISION_GROUP_PLAYER_MOVEMENT,
			Self::Npc => COLLISION_GROUP_NPC,
			Self::InVehicle => COLLISION_GROUP_IN_VEHICLE,
			Self::Weapon => COLLISION_GROUP_WEAPON,
			Self::VehicleClip => COLLISION_GROUP_VEHICLE_CLIP,
			Self::Projectile => COLLISION_GROUP_PROJECTILE,
			Self::DoorBlocker => COLLISION_GROUP_DOOR_BLOCKER,
			Self::PassableDoor => COLLISION_GROUP_PASSABLE_DOOR,
			Self::Dissolving => COLLISION_GROUP_DISSOLVING,
			Self::Pushaway => COLLISION_GROUP_PUSHAWAY,
			Self::NpcActor => COLLISION_GROUP_NPC_ACTOR,
			Self::NpcScripted => COLLISION_GROUP_NPC_SCRIPTED,
		}
	}
}

/// An entity in the server's entity list, as referred to by a `CBaseEntity *`.
///
/// Removing an entity frees it at the end of the frame, so a handle is bound
/// to the scope that produced it. To refer to an entity across callbacks, keep
/// its [`EntityHandle`] and look it up again.
#[doc(alias("CBaseEntity"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity<'s> {
	raw: NonNull<sys::CBaseEntity>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> Entity<'s> {
	/// Wraps a pointer to an entity the engine or game passed to a callback,
	/// such as the receiver of a hooked virtual method, for the callback's
	/// scope.
	///
	/// # Safety
	///
	/// `raw` must point to an entity in the entity list of the server
	/// `_server` belongs to, which stays allocated for `'s`, and the call must
	/// obey [`Server::new`]'s main-thread and reentrancy contract.
	pub unsafe fn from_live(_server: Server<'s>, raw: NonNull<sys::CBaseEntity>) -> Self {
		// SAFETY: The caller vouches for the entity's lifetime during `'s`.
		unsafe { Self::from_raw(raw) }
	}

	/// Wraps a pointer to an entity.
	///
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

	/// Calls Source's `AcceptInput`, which runs the input's handler before
	/// returning, and returns whether it found the input and converted the
	/// value to the input's type.
	///
	/// # Safety
	///
	/// The input, and everything it runs, must free entities only through
	/// deferred deletion. Its handler must accept the activator and caller.
	/// A string value must be pooled, since handlers may keep it.
	pub(crate) unsafe fn accept_input(
		self,
		input: &CStr,
		mut value: sys::variant_t,
		activator: Option<Entity<'_>>,
		caller: Option<Entity<'_>>,
	) -> bool {
		// SAFETY: The entities are live during `'s`, on the main thread, and the
		// value is a local the call may convert in place. The output ID is 0,
		// as for the game's own calls and VScript's `AcceptInput`. The caller
		// upholds the rest.
		unsafe {
			sdk_raw::entities::accept_input(
				self.as_ptr(),
				input,
				activator.map_or(ptr::null_mut(), Entity::as_ptr),
				caller.map_or(ptr::null_mut(), Entity::as_ptr),
				&raw mut value,
				0,
			)
		}
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::CBaseEntity {
		self.raw.as_ptr()
	}

	/// The entity's class name, such as `tf_player`, or an empty string if
	/// Source reports none.
	#[doc(alias("GetClassname"))]
	pub fn class_name(self) -> &'s CStr {
		let Some(networkable) = self.networkable() else {
			return c"";
		};

		// SAFETY: The networkable belongs to the live entity. Class names are
		// pooled strings, which live until the level ends.
		unsafe { borrow_cstr(vcall!(networkable.as_ptr() => IServerNetworkable_GetClassName())) }
			.unwrap_or_default()
	}

	/// The entity's data description maps, from its own class to its bases.
	pub(crate) fn data_maps(self) -> DataMaps<'s> {
		// SAFETY: The entity is live. Its maps are statics of the game DLL,
		// complete since the first entity of its class exists, and the DLL stays
		// loaded for `'s` (`Server::new`).
		unsafe { DataMaps::new(sdk_raw::entities::data_desc_map(self.as_ptr())) }
	}

	/// The entity's edict, or `None` for a server-only entity.
	#[doc(alias("GetEdict"))]
	pub fn edict(self) -> Option<Edict<'s>> {
		let networkable = self.networkable()?;

		// SAFETY: The networkable belongs to the live entity.
		let edict = unsafe { vcall!(networkable.as_ptr() => IServerNetworkable_GetEdict()) };

		// SAFETY: The engine's edict table outlives `'s`.
		NonNull::new(edict).map(|edict| unsafe { Edict::from_raw(edict) })
	}

	/// Finds the offset of a field `CBaseEntity`'s own map declares.
	pub(crate) fn find_base_entity_field(
		self,
		name: &CStr,
		field_type: sys::fieldtype_t,
		size: usize,
	) -> Option<usize> {
		sdk_raw::entities::find_base_entity_field(self.data_maps(), name, field_type, size)
	}

	/// The ID Hammer gave the entity in the map's source file
	/// (`m_iHammerID`), or `None` if it has none. [`HammerId`] describes which
	/// entities have one: those a plugin or script creates usually do not,
	/// but the copies a `point_template` spawns share their template's.
	///
	/// The member is read directly, at the offset the `CBaseEntity` datamap
	/// gives for it. `None` also means that datamap lacks it.
	#[doc(alias("m_iHammerID"))]
	pub fn hammer_id(self) -> Option<HammerId> {
		static HAMMER_ID_OFFSET: OnceLock<Option<usize>> = OnceLock::new();

		let offset = (*HAMMER_ID_OFFSET.get_or_init(|| {
			self.find_base_entity_field(
				c"m_iHammerID",
				sys::_fieldtypes_FIELD_INTEGER,
				size_of::<c_int>(),
			)
		}))?;

		// SAFETY: The offset was validated against the entity datamap, which
		// every entity shares through its `CBaseEntity` base. The member is read
		// without forming a reference, as the game writes it too.
		HammerId::new(unsafe { self.as_ptr().byte_add(offset).cast::<c_int>().read() })
	}

	/// The handle that identifies this entity across frames.
	#[doc(alias("GetRefEHandle"))]
	pub fn handle(self) -> EntityHandle {
		// SAFETY: The entity is live.
		let handle = unsafe { vcall!(self.server_entity() => IServerEntity_GetRefEHandle()) };

		// SAFETY: The handle is a member of the live entity.
		NonNull::new(handle.cast_mut()).map_or(EntityHandle::INVALID, |handle| {
			EntityHandle(unsafe { (&raw const (*handle.as_ptr()).m_Index).read() })
		})
	}

	/// Whether a data description map of the entity's class or one of its
	/// bases is named `class`, such as `CTFPlayer`.
	pub(crate) fn has_data_map_class(self, class: &CStr) -> bool {
		self.data_maps().any(|map| map.class_name() == Some(class))
	}

	/// The entity's slot in the entity list, which is its edict index if it is
	/// networked, or `None` if its handle is invalid.
	#[doc(alias("entindex"))]
	pub fn index(self) -> Option<usize> {
		self.handle().index()
	}

	/// Whether Source has marked this entity for deferred deletion.
	///
	/// # Panics
	///
	/// If the entity's datamaps do not include `CBaseEntity`'s, or it does not
	/// declare `m_iEFlags` as an `int` at an aligned, plausible offset.
	#[doc(alias("EFL_KILLME", "IsMarkedForDeletion"))]
	pub fn is_marked_for_deletion(self) -> bool {
		static EFLAGS_OFFSET: OnceLock<usize> = OnceLock::new();

		let offset = *EFLAGS_OFFSET.get_or_init(|| {
			self.find_base_entity_field(
				c"m_iEFlags",
				sys::_fieldtypes_FIELD_INTEGER,
				size_of::<c_int>(),
			)
			.expect("Source's CBaseEntity datamap does not contain m_iEFlags")
		});

		// SAFETY: The offset was validated against the entity datamap, which
		// every entity shares through its `CBaseEntity` base.
		let flags = unsafe { self.as_ptr().byte_add(offset).cast::<c_int>().read() };

		flags & EFL_KILLME != 0
	}

	/// Whether the game keeps using this entity after it is freed, so that
	/// removing it before the level ends crashes the server.
	///
	/// That is the world, players, and soundscapes: removing a soundscape
	/// shifts the soundscape system's entity list, which the lists it built
	/// for each area of the map then index past.
	pub(crate) fn is_protected(self) -> bool {
		self.index() == Some(0)
			|| self.data_maps().any(|map| {
				matches!(
					map.class_name().map(CStr::to_bytes),
					Some(b"CBasePlayer" | b"CEnvSoundscape")
				)
			})
	}

	/// The entity's name field, `m_iName`, which the `targetname` key sets.
	///
	/// Returns `None` if the `CBaseEntity` datamap has no such field.
	pub(crate) fn name_field(self) -> Option<*mut sys::string_t> {
		static NAME_OFFSET: OnceLock<Option<usize>> = OnceLock::new();

		let offset = (*NAME_OFFSET.get_or_init(|| {
			self.find_base_entity_field(
				c"m_iName",
				sys::_fieldtypes_FIELD_STRING,
				size_of::<sys::string_t>(),
			)
		}))?;

		// SAFETY: The offset was validated against the entity datamap, which
		// every entity shares through its `CBaseEntity` base.
		Some(unsafe { self.as_ptr().byte_add(offset).cast() })
	}

	/// The entity's `IServerNetworkable`, or `None` if Source reports none.
	fn networkable(self) -> Option<NonNull<sys::IServerNetworkable>> {
		// SAFETY: As for `handle`.
		NonNull::new(unsafe { vcall!(self.server_entity() => IServerEntity_GetNetworkable()) })
	}

	/// The handle of the entity's owner (`m_hOwnerEntity`), or `None` if it
	/// has none. The owner may have been removed since, which looking the
	/// handle up tells.
	///
	/// The game sets an owner on what an entity creates or drops, such as a
	/// player's projectiles and, in TF2, the ammo packs a player drops, which
	/// TF2 limits by owner. The member is read directly, at the offset the
	/// `CBaseEntity` datamap gives for it.
	///
	/// Fails with [`OwnerError::UnsupportedLayout`] if that datamap does not
	/// declare it as an entity handle at a plausible offset.
	#[doc(alias("GetOwnerEntity", "m_hOwnerEntity"))]
	pub fn owner(self) -> Result<Option<EntityHandle>, OwnerError> {
		static OWNER_OFFSET: OnceLock<usize> = OnceLock::new();

		let offset = match OWNER_OFFSET.get() {
			Some(&offset) => offset,

			None => {
				let offset = self
					.find_base_entity_field(
						c"m_hOwnerEntity",
						sys::_fieldtypes_FIELD_EHANDLE,
						size_of::<u32>(),
					)
					.ok_or(OwnerError::UnsupportedLayout)?;

				*OWNER_OFFSET.get_or_init(|| offset)
			}
		};

		// SAFETY: The offset was validated against the entity datamap, which
		// every entity shares through its `CBaseEntity` base, for a handle,
		// whose `CBaseHandle` holds only its raw value. The member is read
		// without forming a reference, as the game writes it too.
		let handle = EntityHandle(unsafe { self.as_ptr().byte_add(offset).cast::<u32>().read() });

		Ok(handle.is_valid().then_some(handle))
	}

	/// Reads the absolute origin without key-value conversion.
	///
	/// Source's collision property returns its owner's `GetAbsOrigin()` here.
	/// Returns `None` if the entity has no collideable or it reports no origin.
	#[doc(alias("GetAbsOrigin", "GetCollisionOrigin"))]
	pub fn position(self) -> Option<Vector> {
		// SAFETY: As for `handle`.
		let collideable = NonNull::new(unsafe {
			vcall!(self.server_entity() => IServerEntity_GetCollideable())
		})?;

		// SAFETY: The collideable belongs to the live entity.
		let origin = NonNull::new(
			unsafe { vcall!(collideable.as_ptr() => ICollideable_GetCollisionOrigin()) }.cast_mut(),
		)?;

		// SAFETY: The origin is a member of the live entity, copied immediately.
		Some(unsafe { origin.as_ptr().read() }.into())
	}

	/// The class describing how the entity is networked, or `None` if Source
	/// reports none.
	#[doc(alias("GetServerClass"))]
	pub fn server_class(self) -> Option<ServerClass<'s>> {
		let networkable = self.networkable()?;

		// SAFETY: The networkable belongs to the live entity.
		let class = unsafe { vcall!(networkable.as_ptr() => IServerNetworkable_GetServerClass()) };

		// SAFETY: Server classes are statics of the game DLL.
		NonNull::new(class).map(|class| unsafe { ServerClass::from_raw(class) })
	}

	/// The entity as its primary base, `IServerEntity`, whose generated vtable
	/// starts with the methods of its own bases, `IHandleEntity` and
	/// `IServerUnknown`.
	fn server_entity(self) -> *mut sys::IServerEntity {
		// SAFETY: The entity is live, and its base is only projected to.
		unsafe { &raw mut (*self.as_ptr())._base }
	}

	/// Sets the entity's owner (`m_hOwnerEntity`), or clears it for `None`,
	/// through the game's `SetOwnerEntity`, which records the change for
	/// networking.
	///
	/// The game's collision and trace filters let an entity and its owner
	/// pass through each other, so changing the owner rechecks the collision
	/// filters of the entity's VPhysics objects. TF2 keeps at most 3 of a
	/// player's dropped ammo packs, by owner, so clearing a pack's owner
	/// takes it out of that count.
	///
	/// The game warns that changing collision rules while VPhysics simulates
	/// or runs one of its callbacks is likely to crash, which no callback a
	/// plugin is given is known to run within.
	#[doc(alias("SetOwnerEntity", "m_hOwnerEntity"))]
	pub fn set_owner(self, owner: Option<Entity<'_>>) {
		// SAFETY: The entities are live during `'s`, on the main thread, and
		// every game DLL's vtable has `SetOwnerEntity` where the generated one
		// does. The game defers its own damage and removals during VPhysics
		// callbacks, and buffers the touches it runs from them, so the plugin
		// callbacks `'s` lies in are not known to run within one.
		unsafe {
			sdk_raw::entities::set_owner_entity(
				self.as_ptr(),
				owner.map_or(ptr::null_mut(), Entity::as_ptr),
			)
		};
	}

	/// The `string_t` field `GetKeyValue` reads for a key, or `None` if the key
	/// finds no field, or one of another type.
	///
	/// Fields are searched as `ExtractKeyvalue` does, as
	/// [`DataMaps::find_key_field`] describes.
	pub(crate) fn string_key_field(self, key: &CStr) -> Option<*const sys::string_t> {
		let (field, offset) = self.data_maps().find_key_field(key.to_bytes())?;

		let is_string = matches!(
			field.fieldType,
			sys::_fieldtypes_FIELD_STRING
				| sys::_fieldtypes_FIELD_MODELNAME
				| sys::_fieldtypes_FIELD_SOUNDNAME
		);

		// SAFETY: The offset was found in the entity's own datamap chain.
		(is_string && offset.is_multiple_of(align_of::<sys::string_t>()))
			.then(|| unsafe { self.as_ptr().byte_add(offset).cast_const().cast() })
	}

	/// Moves the entity through Source's `Teleport` method, found at `slot` of
	/// its vtable. Each argument left as `None` is unchanged.
	///
	/// # Safety
	///
	/// `slot` must be where the loaded game DLL's primary `CBaseEntity` vtable
	/// has `Teleport`, such as `Game::teleport_vtable_slot` of the game the
	/// server's game DLL was built for.
	pub(crate) unsafe fn teleport(
		self,
		slot: TeleportSlot,
		origin: Option<Vector>,
		angles: Option<QAngle>,
		velocity: Option<Vector>,
	) -> Result<(), TeleportError> {
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

		// SAFETY: The entity is live during `'s`, on the main thread, and the
		// caller guarantees `Teleport` is at `slot` of the game DLL's vtable,
		// which the entity's class keeps. Every pointer is null or a local, and
		// what the move runs frees entities only through deferred deletion
		// (`Server::new` condition 4).
		unsafe {
			sdk_raw::entities::teleport(
				self.as_ptr(),
				slot,
				origin.as_ref().map_or(ptr::null(), ptr::from_ref),
				angles.as_ref().map_or(ptr::null(), ptr::from_ref),
				velocity.as_ref().map_or(ptr::null(), ptr::from_ref),
			)
		};

		Ok(())
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
#[doc(alias("CBaseHandle", "EHANDLE"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityHandle(u32);

impl EntityHandle {
	/// `INVALID_EHANDLE_INDEX`, which refers to no entity.
	#[doc(alias("INVALID_EHANDLE_INDEX"))]
	pub const INVALID: Self = Self(INVALID_EHANDLE_INDEX);

	/// `NUM_ENT_ENTRIES`, the number of slots in the entity list.
	#[doc(alias("NUM_ENT_ENTRIES"))]
	pub const SLOTS: usize = NUM_ENT_ENTRIES;

	/// Wraps a handle's raw value, the `m_Index` a `CBaseHandle` stores.
	pub const fn from_raw(raw: u32) -> Self {
		Self(raw)
	}

	/// The entity's slot in the entity list, or `None` for an invalid handle.
	#[doc(alias("GetEntryIndex"))]
	pub const fn index(self) -> Option<usize> {
		if self.is_valid() {
			Some((self.0 & ENT_ENTRY_MASK) as usize)
		} else {
			None
		}
	}

	/// Whether the handle is not [`INVALID`](Self::INVALID). A valid handle
	/// can still refer to an entity that has since been removed.
	#[doc(alias("IsValid"))]
	pub const fn is_valid(self) -> bool {
		self.0 != Self::INVALID.0
	}

	/// The serial number the entity's slot had when the entity was created,
	/// which tells it apart from later entities in the same slot.
	#[doc(alias("GetSerialNumber"))]
	pub const fn serial_number(self) -> u32 {
		self.0 >> NUM_SERIAL_NUM_SHIFT_BITS
	}

	/// The handle's raw value, the `m_Index` a `CBaseHandle` stores.
	#[doc(alias("ToInt"))]
	pub const fn to_raw(self) -> u32 {
		self.0
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

/// The ID Hammer gave an entity in the map's source file, which the compiled
/// map keeps as the entity's `hammerid` key, and the game as `m_iHammerID`.
///
/// The map compiler gives one to every entity it writes, the world included,
/// and the game creates them again from the map with the same IDs when a
/// round restarts. Entities added to a compiled map by other tools, such as
/// Stripper:Source, have none. Neither do entities created while the game
/// runs, unless given one through the `hammerid` key, as the copies a
/// `point_template` spawns are: they share their template entity's ID.
#[doc(alias("hammerid", "m_iHammerID"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HammerId(NonZero<c_int>);

impl HammerId {
	/// Returns `None` for 0, which the game stores for entities without one.
	pub const fn new(id: c_int) -> Option<Self> {
		match NonZero::new(id) {
			Some(id) => Some(Self(id)),
			None => None,
		}
	}

	/// The ID as the game stores it, which is never 0.
	pub const fn get(self) -> c_int {
		self.0.get()
	}
}

impl Display for HammerId {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		Display::fmt(&self.0, f)
	}
}

/// An entity's owner could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum OwnerError {
	/// `CBaseEntity`'s datamap does not declare `m_hOwnerEntity` as an entity
	/// handle at a plausible offset, so the game DLL does not match the SDK.
	#[error("the game's CBaseEntity datamap does not declare m_hOwnerEntity as the SDK does")]
	UnsupportedLayout,
}

/// Why [`ServerTools::remove`] refused an entity: the game keeps using it
/// after it is freed.
///
/// [`ServerTools::remove`]: crate::interfaces::ServerTools::remove
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the game keeps using this entity after it is freed")]
pub struct ProtectedEntity;

/// An entity cannot be teleported as requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TeleportError {
	/// A component of the origin, angles, or velocity is infinite or NaN.
	#[error("the destination contains a non-finite component")]
	NonFinite,

	/// The entity is [marked for deletion](Entity::is_marked_for_deletion).
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,
}
