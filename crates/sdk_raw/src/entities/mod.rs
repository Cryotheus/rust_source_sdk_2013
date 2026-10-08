//! Hand-written ABI of `CBaseEntity`: the vtable slots of methods that the
//! generated bindings do not give for every game, calls through them, and
//! header values describing entities, their handles, effects and collision
//! groups. Data description maps are in [`datamap`], the factories entities are
//! created with in [`factory`], health in [`health`], and think contexts in
//! [`think`].
//!
//! The generated `CBaseEntity` vtable is TF2's. Slots are numbered from the
//! primary vtable's first entry, counting each destructor slot, under the
//! target's C++ ABI.

pub mod datamap;
pub mod factory;
pub mod flags;
pub mod health;
pub mod spawn;
pub mod think;

use crate::edicts::MAX_EDICT_BITS;
use crate::util::pointee_size;
use crate::util::vtable::vtable_pointer;
use crate::{vcall, vtable_slot};
use datamap::DataMaps;
use std::ffi::{CStr, c_int};
use std::mem::{MaybeUninit, transmute};

/// `CBaseEntity::SetOwnerEntity`, which sets the entity's owner,
/// `m_hOwnerEntity`, to `owner`, or to none for null.
#[doc(alias("SetOwnerEntity"))]
pub type SetOwnerEntityFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, owner: *mut sys::CBaseEntity);

/// `CBaseEntity::Spawn`, which the game calls once an entity is created and
/// given its keys, and, for a player, each time they spawn.
#[doc(alias("Spawn"))]
pub type SpawnFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity);

/// `CBaseEntity::Teleport`, which moves an entity and sets each of its origin,
/// angles, and velocity that is not null.
#[doc(alias("Teleport"))]
pub type TeleportFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	origin: *const sys::Vector,
	angles: *const sys::QAngle,
	velocity: *const sys::Vector,
);

// `m_hOwnerEntity` is a `CNetworkHandle`, which holds nothing but its
// `CBaseHandle`, as the datamap's `FIELD_EHANDLE` field of it declares.
const _: () = {
	let entity = MaybeUninit::<sys::CBaseEntity>::uninit();

	// SAFETY: The place is only projected to, never read.
	let owner = unsafe { &raw const (*entity.as_ptr()).m_hOwnerEntity };

	assert!(pointee_size(owner) == size_of::<sys::CBaseHandle>());
	assert!(size_of::<sys::CBaseHandle>() == size_of::<u32>());
};

// TF2's slots of these methods match the generated vtable on each ABI.
const _: () = {
	use sys::CBaseEntity__bindgen_vtable as Vtable;

	assert!(ACCEPT_INPUT_SLOT == vtable_slot!(Vtable, CBaseEntity_AcceptInput));
	assert!(GET_DATA_DESC_MAP_SLOT == vtable_slot!(Vtable, CBaseEntity_GetDataDescMap));
	assert!(SET_OWNER_ENTITY_SLOT == vtable_slot!(Vtable, CBaseEntity_SetOwnerEntity));
	assert!(SPAWN_SLOT == vtable_slot!(Vtable, CBaseEntity_Spawn));

	// No method whose declaration depends on the game's defines precedes
	// `SetOwnerEntity` or `Spawn`, as the assertions below on `IsNextBot`
	// describe.
	assert!(SET_OWNER_ENTITY_SLOT < vtable_slot!(Vtable, CBaseEntity_IsNextBot));
	assert!(SPAWN_SLOT < vtable_slot!(Vtable, CBaseEntity_IsNextBot));
};

// Of the virtual methods `CBaseEntity` declares before `Teleport`, only
// `IsNextBot`, under `NEXT_BOT`, and three under `TF_DLL` depend on the game's
// defines: Clang's `-fdump-vtable-layouts` of `CBaseEntity` under the defines
// of each `server_*.vpc` project, in debug and release, on both ABIs, differ
// before `Teleport` in nothing else. These assertions only pin where the
// generated vtable, built with both defines, has those four methods, so that
// leaving them out gives the other games' slots.
const _: () = {
	use sys::CBaseEntity__bindgen_vtable as Vtable;

	let is_next_bot = vtable_slot!(Vtable, CBaseEntity_IsNextBot);
	let is_combat_item = vtable_slot!(Vtable, CBaseEntity_IsCombatItem);

	// `NEXT_BOT` declares `IsNextBot` directly after `IsNPC`.
	assert!(is_next_bot == vtable_slot!(Vtable, CBaseEntity_IsNPC) + 1);

	// `TF_DLL` declares three methods between `IsCombatItem` and
	// `IsBaseCombatWeapon`.
	assert!(vtable_slot!(Vtable, CBaseEntity_IsProjectileCollisionTarget) == is_combat_item + 1);
	assert!(vtable_slot!(Vtable, CBaseEntity_IsFuncLOD) == is_combat_item + 2);
	assert!(vtable_slot!(Vtable, CBaseEntity_IsBaseProjectile) == is_combat_item + 3);
	assert!(vtable_slot!(Vtable, CBaseEntity_IsBaseCombatWeapon) == is_combat_item + 4);

	// All four precede `Teleport`, so leaving out `TF_DLL`'s three gives the
	// `NEXT_BOT` slot, and leaving out `IsNextBot` too gives the plain one.
	assert!(is_next_bot < is_combat_item && is_combat_item + 4 < TF2_TELEPORT_SLOT);
	assert!(SDK2013_NEXT_BOT_TELEPORT_SLOT == TF2_TELEPORT_SLOT - 3);
	assert!(SDK2013_TELEPORT_SLOT == SDK2013_NEXT_BOT_TELEPORT_SLOT - 1);
};

// The generated `SetOwnerEntity`, `Spawn` and `Teleport` have the
// hand-written signatures.
const _: fn(&sys::CBaseEntity__bindgen_vtable) -> SetOwnerEntityFn =
	|vtable| vtable.CBaseEntity_SetOwnerEntity;

const _: fn(&sys::CBaseEntity__bindgen_vtable) -> SpawnFn = |vtable| vtable.CBaseEntity_Spawn;

const _: fn(&sys::CBaseEntity__bindgen_vtable) -> TeleportFn = |vtable| vtable.CBaseEntity_Teleport;

/// `CBaseEntity::AcceptInput` in the primary vtable. TF2 declares its own
/// virtual methods after it, so the slot is the same for every game.
/// Derived from `game/server/baseentity.h` with the MSVC ABI model on Windows
/// and the Itanium ABI model on Linux, and verified against SourceMod's
/// `sdktools.games/game.tf.txt` gamedata.
#[doc(alias("AcceptInput"))]
pub const ACCEPT_INPUT_SLOT: usize = cfg_select! {
	target_os = "windows" => 38,
	target_os = "linux" => 39,
};

/// An exclusive bound on the offsets of `CBaseEntity`'s own fields, past which
/// an offset its datamap gives is not trusted.
pub const BASE_ENTITY_FIELD_OFFSET_LIMIT: usize = 8192;

/// `COLLISION_GROUP_BREAKABLE_GLASS` from `public/const.h`: breakable glass.
pub const COLLISION_GROUP_BREAKABLE_GLASS: c_int = 6;

/// `COLLISION_GROUP_DEBRIS` from `public/const.h`: collides only with entities
/// of [`COLLISION_GROUP_NONE`], such as the world, and of
/// [`COLLISION_GROUP_PUSHAWAY`].
pub const COLLISION_GROUP_DEBRIS: c_int = 1;

/// `COLLISION_GROUP_DEBRIS_TRIGGER` from `public/const.h`: as
/// [`COLLISION_GROUP_DEBRIS`], but touches triggers.
pub const COLLISION_GROUP_DEBRIS_TRIGGER: c_int = 2;

/// `COLLISION_GROUP_DISSOLVING` from `public/const.h`: entities being
/// dissolved.
pub const COLLISION_GROUP_DISSOLVING: c_int = 16;

/// `COLLISION_GROUP_DOOR_BLOCKER` from `public/const.h`: blocks the entities
/// that may not come near moving doors.
pub const COLLISION_GROUP_DOOR_BLOCKER: c_int = 14;

/// `COLLISION_GROUP_IN_VEHICLE` from `public/const.h`: entities inside a
/// vehicle.
pub const COLLISION_GROUP_IN_VEHICLE: c_int = 10;

/// `COLLISION_GROUP_INTERACTIVE` from `public/const.h`: collides with
/// everything but interactive debris and debris.
pub const COLLISION_GROUP_INTERACTIVE: c_int = 4;

/// `COLLISION_GROUP_INTERACTIVE_DEBRIS` from `public/const.h`: collides with
/// everything but debris, interactive or not.
pub const COLLISION_GROUP_INTERACTIVE_DEBRIS: c_int = 3;

/// `COLLISION_GROUP_NONE` from `public/const.h`: collides with everything, as
/// most entities do.
pub const COLLISION_GROUP_NONE: c_int = 0;

/// `COLLISION_GROUP_NPC` from `public/const.h`: non-player characters.
pub const COLLISION_GROUP_NPC: c_int = 9;

/// `COLLISION_GROUP_NPC_ACTOR` from `public/const.h`: non-player characters in
/// scripts, which ignore the player.
pub const COLLISION_GROUP_NPC_ACTOR: c_int = 18;

/// `COLLISION_GROUP_NPC_SCRIPTED` from `public/const.h`: non-player characters
/// in scripts that should not collide with each other.
pub const COLLISION_GROUP_NPC_SCRIPTED: c_int = 19;

/// `COLLISION_GROUP_PASSABLE_DOOR` from `public/const.h`: doors players do not
/// collide with.
pub const COLLISION_GROUP_PASSABLE_DOOR: c_int = 15;

/// `COLLISION_GROUP_PLAYER` from `public/const.h`: players.
pub const COLLISION_GROUP_PLAYER: c_int = 5;

/// `COLLISION_GROUP_PLAYER_MOVEMENT` from `public/const.h`: the group player
/// movement traces with, which TF2 uses to filter out other players and its
/// buildings.
pub const COLLISION_GROUP_PLAYER_MOVEMENT: c_int = 8;

/// `COLLISION_GROUP_PROJECTILE` from `public/const.h`: projectiles.
pub const COLLISION_GROUP_PROJECTILE: c_int = 13;

/// `COLLISION_GROUP_PUSHAWAY` from `public/const.h`: not solid, but pushed away
/// by players' movement.
pub const COLLISION_GROUP_PUSHAWAY: c_int = 17;

/// `COLLISION_GROUP_VEHICLE` from `public/const.h`: vehicles.
pub const COLLISION_GROUP_VEHICLE: c_int = 7;

/// `COLLISION_GROUP_VEHICLE_CLIP` from `public/const.h`: brushes that only
/// block vehicles.
pub const COLLISION_GROUP_VEHICLE_CLIP: c_int = 12;

/// `COLLISION_GROUP_WEAPON` from `public/const.h`: weapons that need collision
/// detection.
pub const COLLISION_GROUP_WEAPON: c_int = 11;

/// `EF_BONEMERGE` from `public/const.h`, a bit of `m_fEffects`: the client
/// places the entity on its parent's bones, as cosmetics follow their wearer.
pub const EF_BONEMERGE: c_int = 0x001;

/// `EF_BONEMERGE_FASTCULL` from `public/const.h`, a bit of `m_fEffects`: with
/// [`EF_BONEMERGE`], the entity is culled with its parent's bounds rather than
/// by setting up the parent's bones.
pub const EF_BONEMERGE_FASTCULL: c_int = 0x080;

/// `EF_BRIGHTLIGHT` from `public/const.h`, a bit of `m_fEffects`: a bright
/// dynamic light at the entity's origin.
pub const EF_BRIGHTLIGHT: c_int = 0x002;

/// `EF_DIMLIGHT` from `public/const.h`, a bit of `m_fEffects`: a dim dynamic
/// light at the entity's origin, as a flashlight.
pub const EF_DIMLIGHT: c_int = 0x004;

/// `EF_ITEM_BLINK` from `public/const.h`, a bit of `m_fEffects`: the item
/// blinks, to be noticed.
pub const EF_ITEM_BLINK: c_int = 0x100;

/// `EF_NODRAW` from `public/const.h`, a bit of `m_fEffects`: the entity is not
/// drawn. `CBaseEntity::UpdateTransmitState` then sends it to no client, as
/// [`FL_EDICT_DONTSEND`](crate::edicts::FL_EDICT_DONTSEND) describes.
pub const EF_NODRAW: c_int = 0x020;

/// `EF_NOINTERP` from `public/const.h`, a bit of `m_fEffects`: the client does
/// not interpolate the entity's next frame.
pub const EF_NOINTERP: c_int = 0x008;

/// `EF_NORECEIVESHADOW` from `public/const.h`, a bit of `m_fEffects`: the
/// entity receives no shadows.
pub const EF_NORECEIVESHADOW: c_int = 0x040;

/// `EF_NOSHADOW` from `public/const.h`, a bit of `m_fEffects`: the entity casts
/// no shadow.
pub const EF_NOSHADOW: c_int = 0x010;

/// `EF_PARENT_ANIMATES` from `public/const.h`, a bit of `m_fEffects`: the
/// entity's parent is always assumed to animate.
pub const EF_PARENT_ANIMATES: c_int = 0x200;

/// `EFL_BOT_FROZEN` from `game/shared/shareddefs.h`: the bit of `m_iEFlags`
/// set on a bot that is frozen in place.
pub const EFL_BOT_FROZEN: c_int = 1 << 8;

/// `EFL_CHECK_UNTOUCH` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set while the game is to check which of the entity's touches
/// have ended (`SetCheckUntouch`).
pub const EFL_CHECK_UNTOUCH: c_int = 1 << 24;

/// `EFL_DIRTY_ABSANGVELOCITY` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set while the entity's angular velocity in the world is yet to
/// be computed from its move parent's.
pub const EFL_DIRTY_ABSANGVELOCITY: c_int = 1 << 13;

/// `EFL_DIRTY_ABSTRANSFORM` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set while the entity's origin and angles in the world are yet
/// to be computed from its move parent's (`CalcAbsolutePosition`).
pub const EFL_DIRTY_ABSTRANSFORM: c_int = 1 << 11;

/// `EFL_DIRTY_ABSVELOCITY` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set while the entity's velocity in the world is yet to be
/// computed from its move parent's.
pub const EFL_DIRTY_ABSVELOCITY: c_int = 1 << 12;

/// `EFL_DIRTY_SHADOWUPDATE` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` that only clients set, for their shadow manager to update the
/// entity's shadow.
pub const EFL_DIRTY_SHADOWUPDATE: c_int = 1 << 5;

/// `EFL_DIRTY_SPATIAL_PARTITION` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set while the entity's place in the spatial partition is yet
/// to be updated.
pub const EFL_DIRTY_SPATIAL_PARTITION: c_int = 1 << 15;

/// `EFL_DIRTY_SURROUNDING_COLLISION_BOUNDS` from `game/shared/shareddefs.h`:
/// the bit of `m_iEFlags` set while the box around the entity's collision
/// volume is yet to be computed again.
pub const EFL_DIRTY_SURROUNDING_COLLISION_BOUNDS: c_int = 1 << 14;

/// `EFL_DONTBLOCKLOS` from `game/shared/shareddefs.h`: the bit of `m_iEFlags`
/// set on an entity that does not block NPCs' line of sight.
pub const EFL_DONTBLOCKLOS: c_int = 1 << 25;

/// `EFL_DONTWALKON` from `game/shared/shareddefs.h`: the bit of `m_iEFlags`
/// set on an entity NPCs do not walk on.
pub const EFL_DONTWALKON: c_int = 1 << 26;

/// `EFL_DORMANT` from `game/shared/shareddefs.h`: the bit of `m_iEFlags` set
/// while the entity is dormant, and sends clients no updates.
pub const EFL_DORMANT: c_int = 1 << 1;

/// `EFL_FORCE_ALLOW_MOVEPARENT` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` that lets an entity without an edict move with a parent.
pub const EFL_FORCE_ALLOW_MOVEPARENT: c_int = 1 << 16;

/// `EFL_FORCE_CHECK_TRANSMIT` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` that has the entity sent to clients even without a model, as
/// the entities the client draws by itself need.
pub const EFL_FORCE_CHECK_TRANSMIT: c_int = 1 << 7;

/// `EFL_HAS_PLAYER_CHILD` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set while the entity, or an entity moving with it, is a player
/// (`RecalcHasPlayerChildBit`). [`EFL_KEEP_ON_RECREATE_ENTITIES`] has the same
/// value.
pub const EFL_HAS_PLAYER_CHILD: c_int = 1 << 4;

/// `EFL_IN_SKYBOX` from `game/shared/shareddefs.h`: the bit of `m_iEFlags` set
/// on an entity in the 3D skybox, which is then sent to clients as if they
/// could see it.
pub const EFL_IN_SKYBOX: c_int = 1 << 17;

/// `EFL_IS_BEING_LIFTED_BY_BARNACLE` from `game/shared/shareddefs.h`: the bit
/// of `m_iEFlags` set while a Half-Life 2 barnacle lifts the entity.
pub const EFL_IS_BEING_LIFTED_BY_BARNACLE: c_int = 1 << 20;

/// `EFL_KEEP_ON_RECREATE_ENTITIES` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` that keeps the entity, such as the world, when the game removes
/// and creates again only the map's entities. [`EFL_HAS_PLAYER_CHILD`] has the
/// same value.
pub const EFL_KEEP_ON_RECREATE_ENTITIES: c_int = 1 << 4;

/// `EFL_KILLME` from `game/shared/shareddefs.h`: the bit of `m_iEFlags` set
/// while the entity is marked for deferred deletion.
pub const EFL_KILLME: c_int = 1 << 0;

/// `EFL_NO_AUTO_EDICT_ATTACH` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set on an entity that attaches its edict itself, as players
/// and the world do, rather than as it is created.
pub const EFL_NO_AUTO_EDICT_ATTACH: c_int = 1 << 10;

/// `EFL_NO_DAMAGE_FORCES` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set on an entity that takes no forces from physics damage, as
/// its `nodamageforces` key value sets.
pub const EFL_NO_DAMAGE_FORCES: c_int = 1 << 31;

/// `EFL_NO_DISSOLVE` from `game/shared/shareddefs.h`: the bit of `m_iEFlags`
/// set on an entity that is not dissolved.
pub const EFL_NO_DISSOLVE: c_int = 1 << 27;

/// `EFL_NO_GAME_PHYSICS_SIMULATION` from `game/shared/shareddefs.h`: the bit
/// of `m_iEFlags` set while the game does not simulate the entity's movement.
/// With [`EFL_NO_THINK_FUNCTION`], the entity leaves the list of those the
/// game runs each frame.
pub const EFL_NO_GAME_PHYSICS_SIMULATION: c_int = 1 << 23;

/// `EFL_NO_MEGAPHYSCANNON_RAGDOLL` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set on an entity Half-Life 2's charged gravity gun cannot turn
/// into a ragdoll.
pub const EFL_NO_MEGAPHYSCANNON_RAGDOLL: c_int = 1 << 28;

/// `EFL_NO_PHYSCANNON_INTERACTION` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set on an entity Half-Life 2's gravity gun cannot pick up or
/// punt.
pub const EFL_NO_PHYSCANNON_INTERACTION: c_int = 1 << 30;

/// `EFL_NO_ROTORWASH_PUSH` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set on an entity Half-Life 2's helicopters' rotor wash does not
/// push.
pub const EFL_NO_ROTORWASH_PUSH: c_int = 1 << 21;

/// `EFL_NO_THINK_FUNCTION` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set while the entity has no think scheduled, as [`think`]
/// describes.
pub const EFL_NO_THINK_FUNCTION: c_int = 1 << 22;

/// `EFL_NO_WATER_VELOCITY_CHANGE` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set on an entity whose velocity the game does not change as it
/// enters water.
pub const EFL_NO_WATER_VELOCITY_CHANGE: c_int = 1 << 29;

/// `EFL_NOCLIP_ACTIVE` from `game/shared/shareddefs.h`: the bit of `m_iEFlags`
/// set while the `noclip` command is active for the player.
pub const EFL_NOCLIP_ACTIVE: c_int = 1 << 2;

/// `EFL_NOTIFY` from `game/shared/shareddefs.h`: the bit of `m_iEFlags` set
/// while another entity watches the entity's events, as the game's
/// teleporting does.
pub const EFL_NOTIFY: c_int = 1 << 6;

/// `EFL_SERVER_ONLY` from `game/shared/shareddefs.h`: the bit of `m_iEFlags`
/// set on an entity that is not networked, so has no edict.
pub const EFL_SERVER_ONLY: c_int = 1 << 9;

/// `EFL_SETTING_UP_BONES` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set while the entity's model sets up its bones.
pub const EFL_SETTING_UP_BONES: c_int = 1 << 3;

/// `EFL_TOUCHING_FLUID` from `game/shared/shareddefs.h`: the bit of
/// `m_iEFlags` set while the entity's VPhysics object touches a fluid, which
/// tells whether it floats.
pub const EFL_TOUCHING_FLUID: c_int = 1 << 19;

/// `EFL_USE_PARTITION_WHEN_NOT_SOLID` from `game/shared/shareddefs.h`: the bit
/// of `m_iEFlags` that keeps the entity in the spatial partition while it is
/// not solid, as triggers need.
pub const EFL_USE_PARTITION_WHEN_NOT_SOLID: c_int = 1 << 18;

/// `ENT_ENTRY_MASK` from `public/const.h`: the bits of a `CBaseHandle`'s raw
/// value holding the entity's slot in the entity list.
pub const ENT_ENTRY_MASK: u32 = (1 << NUM_SERIAL_NUM_BITS) - 1;

/// `FSOLID_CUSTOMBOXTEST` from `public/const.h`: the engine's swept box
/// traces against the entity ask its `TestCollision`, whatever its solid type.
pub const FSOLID_CUSTOMBOXTEST: u16 = 0x0002;

/// `FSOLID_CUSTOMRAYTEST` from `public/const.h`: the engine's ray traces,
/// lines and points, against the entity ask its `TestCollision`, whatever its
/// solid type. `CBaseEntity`'s reports no hit.
pub const FSOLID_CUSTOMRAYTEST: u16 = 0x0001;

/// `FSOLID_FORCE_WORLD_ALIGNED` from `public/const.h`: the entity collides
/// as a world-aligned box, even with a `SOLID_BSP` or `SOLID_VPHYSICS` model.
pub const FSOLID_FORCE_WORLD_ALIGNED: u16 = 0x0040;

/// `FSOLID_NOT_SOLID` from `public/const.h`: the entity is not solid.
pub const FSOLID_NOT_SOLID: u16 = 0x0004;

/// `FSOLID_NOT_STANDABLE` from `public/const.h`: nothing can stand on the
/// entity.
pub const FSOLID_NOT_STANDABLE: u16 = 0x0010;

/// `FSOLID_ROOT_PARENT_ALIGNED` from `public/const.h`: the entity's
/// collisions are in its root parent's local space.
pub const FSOLID_ROOT_PARENT_ALIGNED: u16 = 0x0100;

/// `FSOLID_TRIGGER` from `public/const.h`: the entity runs touch functions,
/// as triggers do.
pub const FSOLID_TRIGGER: u16 = 0x0008;

/// `FSOLID_TRIGGER_TOUCH_DEBRIS` from `public/const.h`: the trigger touches
/// debris.
pub const FSOLID_TRIGGER_TOUCH_DEBRIS: u16 = 0x0200;

/// `FSOLID_USE_TRIGGER_BOUNDS` from `public/const.h`: the entity has trigger
/// bounds of its own, apart from its box.
pub const FSOLID_USE_TRIGGER_BOUNDS: u16 = 0x0080;

/// `FSOLID_VOLUME_CONTENTS` from `public/const.h`: the entity has contents
/// throughout its volume, as water does.
pub const FSOLID_VOLUME_CONTENTS: u16 = 0x0020;

/// `CBaseEntity::GetDataDescMap` in the Source SDK 2013 primary vtable.
/// Derived from `game/server/cbase.h` with the MSVC ABI model on Windows and
/// the Itanium ABI model on Linux.
#[doc(alias("GetDataDescMap"))]
pub const GET_DATA_DESC_MAP_SLOT: usize = cfg_select! {
	target_os = "windows" => 11,
	target_os = "linux" => 12,
};

/// `INVALID_EHANDLE_INDEX` from `public/const.h`: the raw value of a
/// `CBaseHandle` that refers to no entity.
pub const INVALID_EHANDLE_INDEX: u32 = 0xFFFF_FFFF;

/// `INVALID_NETWORKED_EHANDLE_VALUE` from `public/const.h`: the value
/// `SendProxy_EHandleToInt` networks for a handle that refers to no entity.
pub const INVALID_NETWORKED_EHANDLE_VALUE: u32 = (1 << NUM_NETWORKED_EHANDLE_BITS) - 1;

/// `LAST_SHARED_COLLISION_GROUP` from `public/const.h`: the first collision
/// group a game defines for itself, past those every game shares, as TF2's
/// `TFCOLLISION_GROUP_*` are.
pub const LAST_SHARED_COLLISION_GROUP: c_int = 20;

/// `NUM_ENT_ENTRIES` from `public/const.h`: the number of slots in the entity
/// list, networked or not.
pub const NUM_ENT_ENTRIES: usize = 1 << NUM_ENT_ENTRY_BITS;

/// `NUM_ENT_ENTRY_BITS` from `public/const.h`: the bits that index the entity
/// list.
pub const NUM_ENT_ENTRY_BITS: u32 = MAX_EDICT_BITS + 2;

/// `NUM_NETWORKED_EHANDLE_BITS` from `public/const.h`: the bits of a networked
/// handle, its edict index in the low [`MAX_EDICT_BITS`] and its serial number
/// above them.
pub const NUM_NETWORKED_EHANDLE_BITS: u32 =
	MAX_EDICT_BITS + NUM_NETWORKED_EHANDLE_SERIAL_NUMBER_BITS;

/// `NUM_NETWORKED_EHANDLE_SERIAL_NUMBER_BITS` from `public/const.h`: the low
/// bits of a handle's serial number that a networked handle keeps.
pub const NUM_NETWORKED_EHANDLE_SERIAL_NUMBER_BITS: u32 = 10;

/// `NUM_SERIAL_NUM_BITS` from `public/const.h`: the bits of a `CBaseHandle`'s
/// serial number.
pub const NUM_SERIAL_NUM_BITS: u32 = 16;

/// `NUM_SERIAL_NUM_SHIFT_BITS` from `public/const.h`: how far a
/// `CBaseHandle`'s raw value shifts its serial number up, above the entity's
/// slot.
pub const NUM_SERIAL_NUM_SHIFT_BITS: u32 = 32 - NUM_SERIAL_NUM_BITS;

/// `CBaseEntity::Teleport` in a Source SDK 2013 game DLL built with
/// `NEXT_BOT` but not `TF_DLL`, as `server_hl2mp.vpc` builds HL2:DM and the
/// mods based on it. `IsNextBot`, which `NEXT_BOT` declares, precedes it, so
/// the slot is one past [`SDK2013_TELEPORT_SLOT`]. Derived from
/// `game/server/baseentity.h` with the MSVC ABI model on Windows and the
/// Itanium ABI model on Linux, and verified against SourceMod's
/// `sdktools.games/game.hl2mp.txt` gamedata.
///
/// Valve's `baseentity.h` has declared `IsNextBot` only since March 2025, so
/// a game DLL built with `NEXT_BOT` from older sources has `Teleport` at
/// [`SDK2013_TELEPORT_SLOT`]. SourceMod's gamedata had HL2:DM's `Teleport`
/// there too until August 2026.
#[doc(alias("Teleport"))]
pub const SDK2013_NEXT_BOT_TELEPORT_SLOT: usize = cfg_select! {
	target_os = "windows" => 111,
	target_os = "linux" => 112,
};

/// `CBaseEntity::Teleport` in a Source SDK 2013 game DLL built with neither
/// `TF_DLL` nor `NEXT_BOT`, as `server_hl2.vpc`, `server_episodic.vpc`, and
/// `server_lostcoast.vpc` build it, or with `NEXT_BOT` from sources older
/// than `IsNextBot`. Derived from `game/server/baseentity.h` with the MSVC ABI
/// model on Windows and the Itanium ABI model on Linux.
///
/// A game DLL built with `NEXT_BOT` from sources that declare `IsNextBot` has
/// `SUB_AllowedToFade` here instead, and `Teleport` at
/// [`SDK2013_NEXT_BOT_TELEPORT_SLOT`].
#[doc(alias("Teleport"))]
pub const SDK2013_TELEPORT_SLOT: usize = cfg_select! {
	target_os = "windows" => 110,
	target_os = "linux" => 111,
};

/// `CBaseEntity::SetOwnerEntity` in the primary vtable. No virtual method a
/// game declares under its own defines precedes it, so the slot is the same
/// for every game. Derived from `game/server/baseentity.h` with the MSVC ABI
/// model on Windows and the Itanium ABI model on Linux, and verified against
/// TF2's 64-bit Windows `server.dll`, whose `CTFAmmoPack` vtable has
/// `CBaseEntity::SetOwnerEntity` there.
#[doc(alias("SetOwnerEntity"))]
pub const SET_OWNER_ENTITY_SLOT: usize = cfg_select! {
	target_os = "windows" => 18,
	target_os = "linux" => 19,
};

/// `CBaseEntity::Spawn` in the primary vtable. No virtual method a game
/// declares under its own defines precedes it, so the slot is the same for
/// every game. Derived from `game/server/baseentity.h` with the MSVC ABI model
/// on Windows and the Itanium ABI model on Linux, and verified against
/// SourceMod's `sdkhooks.games/engine.ep2v.txt` gamedata.
#[doc(alias("Spawn"))]
pub const SPAWN_SLOT: usize = cfg_select! {
	target_os = "windows" => 24,
	target_os = "linux" => 25,
};

/// `CBaseEntity::Teleport` in TF2's game DLL, built with `TF_DLL` and
/// `NEXT_BOT`. Verified against SourceMod's `sdktools.games/game.tf.txt`
/// gamedata.
#[doc(alias("Teleport"))]
pub const TF2_TELEPORT_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_Teleport);

/// Where a game DLL's primary `CBaseEntity` vtable has `Teleport`, which the
/// virtual methods declared before it under `TF_DLL` and `NEXT_BOT` move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeleportSlot {
	/// [`TF2_TELEPORT_SLOT`], the generated vtable's, that of a game DLL built
	/// with `TF_DLL` and `NEXT_BOT`.
	TeamFortress2,

	/// [`SDK2013_TELEPORT_SLOT`], that of a Source SDK 2013 game DLL built
	/// with neither `TF_DLL` nor `NEXT_BOT`, or with `NEXT_BOT` from sources
	/// older than `IsNextBot`.
	SourceSdk2013,

	/// [`SDK2013_NEXT_BOT_TELEPORT_SLOT`], that of a Source SDK 2013 game DLL
	/// built with `NEXT_BOT` but not `TF_DLL`, from sources that declare
	/// `IsNextBot`.
	SourceSdk2013NextBot,
}

impl TeleportSlot {
	/// The index of `Teleport` in the primary vtable.
	pub const fn index(self) -> usize {
		match self {
			Self::TeamFortress2 => TF2_TELEPORT_SLOT,
			Self::SourceSdk2013 => SDK2013_TELEPORT_SLOT,
			Self::SourceSdk2013NextBot => SDK2013_NEXT_BOT_TELEPORT_SLOT,
		}
	}
}

/// Calls `CBaseEntity::AcceptInput`, which runs the input's handler before
/// returning, and returns whether the entity found the input and converted
/// the value to the input's type.
///
/// The method is called through the generated vtable, whose slot of it,
/// [`ACCEPT_INPUT_SLOT`], every game DLL shares. `variant_t` has a
/// user-provided copy constructor, through its `CHandle`, so both ABIs pass it
/// by address: `value` points to the caller's copy, which `AcceptInput` may
/// convert in place.
///
/// # Safety
///
/// - `entity` must point to a live `CBaseEntity` of the loaded game DLL, and
///   the call must be made on the server's main thread.
/// - `activator` and `caller` must each be null or point to a live
///   `CBaseEntity`, and the input's handler must accept them: some handlers
///   dereference them without checking.
/// - `value` must point to a valid `variant_t` for the call, and a string it
///   holds must stay valid for as long as the handler may keep it, as a
///   pooled string does.
/// - The input, and every game function it runs, must free entities only
///   through deferred deletion.
#[doc(alias("AcceptInput"))]
pub unsafe fn accept_input(
	entity: *mut sys::CBaseEntity,
	input: &CStr,
	activator: *mut sys::CBaseEntity,
	caller: *mut sys::CBaseEntity,
	value: *mut sys::variant_t,
	output_id: c_int,
) -> bool {
	// SAFETY: The entity is live, and every game DLL's vtable has
	// `AcceptInput` where the generated one does. The name is only read during
	// the call, and the caller upholds the rest.
	unsafe {
		vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_AcceptInput(
			input.as_ptr(),
			activator,
			caller,
			value,
			output_id,
		))
	}
}

/// Calls `CBaseEntity::GetDataDescMap`, which returns the first of the data
/// description maps of the entity's class, as [`DataMaps::new`] takes them.
///
/// The method is called through the generated vtable, whose slot of it,
/// [`GET_DATA_DESC_MAP_SLOT`], every game DLL shares.
///
/// # Safety
///
/// `entity` must point to a live `CBaseEntity` of the loaded game DLL.
#[doc(alias("GetDataDescMap"))]
pub unsafe fn data_desc_map(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: The entity is live, and every game DLL's vtable has
	// `GetDataDescMap` where the generated one does.
	unsafe { vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_GetDataDescMap()) }
}

/// Finds the offset of the field named `name`, of `field_type` and `size`
/// bytes, that `CBaseEntity`'s own map in `maps` declares.
///
/// Returns `None` unless the field lies under
/// [`BASE_ENTITY_FIELD_OFFSET_LIMIT`], at an offset aligned for its size: to
/// the largest power of two dividing it, up to a pointer's alignment, so a
/// 12-byte `Vector` of floats needs 4 bytes.
pub fn find_base_entity_field(
	mut maps: DataMaps<'_>,
	name: &CStr,
	field_type: sys::fieldtype_t,
	size: usize,
) -> Option<usize> {
	let map = maps.find(|map| map.class_name() == Some(c"CBaseEntity"))?;

	let field = map.fields().iter().find(|field| {
		field.fieldType == field_type
			&& usize::try_from(field.fieldSizeInBytes) == Ok(size)
			&& field.name() == Some(name)
	})?;

	let offset = field.offset()?;
	let alignment = size.isolate_lowest_one().clamp(1, align_of::<*const ()>());

	(offset < BASE_ENTITY_FIELD_OFFSET_LIMIT && offset.is_multiple_of(alignment)).then_some(offset)
}

/// Finds the offset of the entity's VPhysics object, `m_pPhysicsObject`,
/// which `VPhysicsGetObject` returns, as `CBaseEntity`'s own map in `maps`
/// declares it: one `FIELD_CUSTOM` member (`DEFINE_PHYSPTR`), whose size the
/// map leaves out.
///
/// Returns `None` unless the field lies under
/// [`BASE_ENTITY_FIELD_OFFSET_LIMIT`], aligned for a pointer.
#[doc(alias("m_pPhysicsObject", "VPhysicsGetObject", "DEFINE_PHYSPTR"))]
pub fn find_physics_object_field(mut maps: DataMaps<'_>) -> Option<usize> {
	let map = maps.find(|map| map.class_name() == Some(c"CBaseEntity"))?;

	let field = map.fields().iter().find(|field| {
		field.fieldType == sys::_fieldtypes_FIELD_CUSTOM
			&& field.fieldSize == 1
			&& field.name() == Some(c"m_pPhysicsObject")
	})?;

	let offset = field.offset()?;

	(offset < BASE_ENTITY_FIELD_OFFSET_LIMIT && offset.is_multiple_of(align_of::<*const ()>()))
		.then_some(offset)
}

/// Finds the offset of the entity's solid flags, `m_usSolidFlags`, the
/// `unsigned short` of its collision property, `m_Collision`, which
/// `CBaseEntity`'s own map in `maps` embeds.
///
/// Returns `None` unless `CCollisionProperty`'s map declares the flags, and
/// they lie under [`BASE_ENTITY_FIELD_OFFSET_LIMIT`], aligned.
#[doc(alias("m_usSolidFlags", "m_Collision"))]
pub fn find_solid_flags_field(mut maps: DataMaps<'_>) -> Option<usize> {
	let map = maps.find(|map| map.class_name() == Some(c"CBaseEntity"))?;

	let collision = map.fields().iter().find(|field| {
		field.fieldType == sys::_fieldtypes_FIELD_EMBEDDED
			&& field.fieldSize == 1
			&& field.name() == Some(c"m_Collision")
	})?;

	let flags = collision
		.embedded()
		.find(|map| map.class_name() == Some(c"CCollisionProperty"))?
		.fields()
		.iter()
		.find(|field| {
			field.fieldType == sys::_fieldtypes_FIELD_SHORT
				&& usize::try_from(field.fieldSizeInBytes) == Ok(size_of::<u16>())
				&& field.name() == Some(c"m_usSolidFlags")
		})?;

	let offset = collision.offset()?.checked_add(flags.offset()?)?;

	(offset < BASE_ENTITY_FIELD_OFFSET_LIMIT && offset.is_multiple_of(align_of::<u16>()))
		.then_some(offset)
}

/// Calls `CBaseEntity::SetOwnerEntity`, which sets the entity's owner,
/// `m_hOwnerEntity`, to `owner`, or to none for null, and records the change
/// for networking.
///
/// When the owner changes, the method calls `CollisionRulesChanged`, which
/// rechecks the collision filters of the entity's VPhysics objects: the
/// game's filters (`PassServerEntityFilter`) let an entity and its owner pass
/// through each other, and traces that skip an entity skip what it owns.
/// Classes may override the method, as `CNodeEnt`, which keeps no owner,
/// does.
///
/// The method is called through the generated vtable, whose slot of it,
/// [`SET_OWNER_ENTITY_SLOT`], every game DLL shares.
///
/// # Safety
///
/// - `entity` must point to a live `CBaseEntity` of the loaded game DLL, and
///   the call must be made on the server's main thread.
/// - `owner` must be null or point to a live `CBaseEntity`.
/// - The call must not be made while VPhysics simulates or runs one of its
///   callbacks (`PhysIsInCallback`), during which `CollisionRulesChanged`
///   warns that changing collision rules is likely to crash.
#[doc(alias("SetOwnerEntity"))]
pub unsafe fn set_owner_entity(entity: *mut sys::CBaseEntity, owner: *mut sys::CBaseEntity) {
	// SAFETY: The entity is live, and every game DLL's vtable has
	// `SetOwnerEntity` where the generated one does. The caller upholds the
	// rest.
	unsafe {
		vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_SetOwnerEntity(owner))
	}
}

/// Calls `CBaseEntity::Teleport`, at `slot` of the entity's primary vtable,
/// with each of the origin, angles, and velocity to set, or null to leave it.
///
/// TF2's slot is called through the generated vtable, and either Source SDK
/// 2013 slot through a [`TeleportFn`] read from it.
///
/// # Safety
///
/// - `entity` must point to a live `CBaseEntity` of the loaded game DLL,
///   whose primary vtable has `Teleport` at `slot`, and the call must be made
///   on the server's main thread.
/// - `origin`, `angles`, and `velocity` must each be null or valid for reads
///   during the call.
/// - The game functions moving the entity runs, such as those of its physics
///   and children, must free entities only through deferred deletion.
#[doc(alias("Teleport"))]
pub unsafe fn teleport(
	entity: *mut sys::CBaseEntity,
	slot: TeleportSlot,
	origin: *const sys::Vector,
	angles: *const sys::QAngle,
	velocity: *const sys::Vector,
) {
	match slot {
		// SAFETY: The entity is live, and its vtable is TF2's, as generated.
		// The caller upholds the rest.
		TeleportSlot::TeamFortress2 => unsafe {
			vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_Teleport(
				origin, angles, velocity,
			))
		},

		TeleportSlot::SourceSdk2013 | TeleportSlot::SourceSdk2013NextBot => {
			// SAFETY: The live entity starts with the pointer to its primary
			// vtable, which has `Teleport`, with the generated signature, at
			// this slot.
			let teleport = unsafe {
				let slots = vtable_pointer::<*const ()>(entity);

				transmute::<*const (), TeleportFn>(slots.add(slot.index()).read())
			};

			// SAFETY: As above; the caller upholds the rest.
			unsafe { teleport(entity, origin, angles, velocity) }
		}
	}
}
