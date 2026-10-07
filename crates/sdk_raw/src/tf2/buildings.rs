//! TF2's numbers for its buildings (`CBaseObject`): their types, modes,
//! states and object flags from `game/shared/tf/tf_shareddefs.h`, and the
//! spawn flags and solidity values of `game/server/tf/tf_obj*.h`. Also the
//! signatures and vtable slots of the `CBaseObject` methods that run as a
//! building dies, finishes being built, starts upgrading, and is hit by a
//! wrench, which `metamod_source`'s building hooks hook.
//!
//! [`objects`](super::objects) holds the ABI of where buildings may be
//! placed, and the search for building classes' vtables.

use crate::vtable_slot;
use std::ffi::c_int;

/// The signature of `CBaseObject::FinishedBuilding`, `void ()`, with the
/// building as its receiver, which the game calls as a building finishes
/// being built, or redeployed after being carried.
///
/// The generated method takes a `CBaseObject` receiver. It is the building's
/// primary base, `CBaseEntity`, at the same address, as
/// [`entities::health`](crate::entities::health) asserts, so the method can
/// be called and hooked with an entity receiver. The same goes for the other
/// methods of this module.
#[doc(alias("FinishedBuilding"))]
pub type FinishedBuildingFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity);

/// The signature of `CBaseObject::InputWrenchHit`,
/// `bool (CTFPlayer *, CTFWrench *, Vector)`, with the building as its
/// receiver, which runs as an Engineer's wrench hits a building of their team,
/// and returns whether the hit did anything: removed a sapper, sped up
/// construction, repaired, refilled or upgraded the building.
///
/// `player` is the Engineer's entity, and `wrench` their wrench's: TF2's
/// `CTFWrench` derives from `CTFWeaponBase` through its primary bases, which
/// [`weapons`](crate::tf2::weapons) asserts start at the entity. `position`
/// is where the hit landed. `Vector` is trivially copyable, so both ABIs pass
/// it as C passes the `#[repr(C)]` struct.
#[doc(alias("InputWrenchHit"))]
pub type InputWrenchHitFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	player: *mut sys::CBaseEntity,
	wrench: *mut sys::CBaseEntity,
	position: sys::Vector,
) -> bool;

/// The signature of `CBaseObject::Killed`, `void (const CTakeDamageInfo &)`,
/// with the building as its receiver, which destroys the building with the
/// damage that killed it: it fires the `object_destroyed` event, or
/// `object_detonated` when the building is its own inflictor, explodes the
/// building into gibs, and removes it.
#[doc(alias("Killed"))]
pub type KilledFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, info: *const sys::CTakeDamageInfo);

/// The signature of `CBaseObject::StartUpgrading`, `void ()`, with the
/// building as its receiver, which raises the building's level by one and
/// starts its upgrade animation. The game also calls it for each level a
/// redeployed building gets back, and for those a map gives its own
/// buildings.
#[doc(alias("StartUpgrading"))]
pub type StartUpgradingFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity);

// The generated method takes the damage by reference and returns nothing.
const _: fn(
	&sys::CBaseObject__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CBaseObject, *const sys::CTakeDamageInfo) =
	|vtable| vtable.CBaseObject_Killed;

// The generated method takes the player, the wrench and the hit's position by
// value, and returns a `bool`.
const _: fn(
	&sys::CBaseObject__bindgen_vtable,
) -> unsafe extern "C" fn(
	*mut sys::CBaseObject,
	*mut sys::CTFPlayer,
	*mut sys::CTFWrench,
	sys::Vector,
) -> bool = |vtable| vtable.CBaseObject_InputWrenchHit;

// The generated method takes no argument and returns nothing.
const _: fn(&sys::CBaseObject__bindgen_vtable) -> unsafe extern "C" fn(*mut sys::CBaseObject) =
	|vtable| vtable.CBaseObject_FinishedBuilding;

// The generated method takes no argument and returns nothing.
const _: fn(&sys::CBaseObject__bindgen_vtable) -> unsafe extern "C" fn(*mut sys::CBaseObject) =
	|vtable| vtable.CBaseObject_StartUpgrading;

/// `DISPENSER_STATE_IDLE`: a dispenser that is not upgrading.
pub const DISPENSER_STATE_IDLE: c_int = 0;

/// `DISPENSER_STATE_UPGRADING`: a dispenser playing its upgrade animation.
pub const DISPENSER_STATE_UPGRADING: c_int = 1;

/// The slot of `CBaseObject::FinishedBuilding` in a TF2 building's primary
/// vtable, from the generated binding.
#[doc(alias("FinishedBuilding"))]
pub const FINISHED_BUILDING_SLOT: usize = vtable_slot!(
	sys::CBaseObject__bindgen_vtable,
	CBaseObject_FinishedBuilding
);

/// The slot of `CBaseObject::InputWrenchHit` in a TF2 building's primary
/// vtable, from the generated binding.
#[doc(alias("InputWrenchHit"))]
pub const INPUT_WRENCH_HIT_SLOT: usize =
	vtable_slot!(sys::CBaseObject__bindgen_vtable, CBaseObject_InputWrenchHit);

/// The slot of `CBaseObject::Killed` in a TF2 building's primary vtable, from
/// the generated binding.
#[doc(alias("Killed"))]
pub const KILLED_SLOT: usize = vtable_slot!(sys::CBaseObject__bindgen_vtable, CBaseObject_Killed);

/// `MODE_SAPPER_ANTI_ROBOT`: a sapper of Mann vs. Machine's anti-robot kind,
/// placed on a robot.
pub const MODE_SAPPER_ANTI_ROBOT: c_int = 1;

/// `MODE_SAPPER_ANTI_ROBOT_RADIUS`: an anti-robot sapper that also saps the
/// robots around it.
pub const MODE_SAPPER_ANTI_ROBOT_RADIUS: c_int = 2;

/// `MODE_SAPPER_NORMAL`: a sapper placed on a building.
pub const MODE_SAPPER_NORMAL: c_int = 0;

/// `MODE_SENTRYGUN_DISPOSABLE`: a disposable sentry, which Mann vs. Machine's
/// upgrades let engineers build besides their own.
pub const MODE_SENTRYGUN_DISPOSABLE: c_int = 1;

/// `MODE_SENTRYGUN_NORMAL`: an engineer's sentry, mini-sentries included.
pub const MODE_SENTRYGUN_NORMAL: c_int = 0;

/// `MODE_TELEPORTER_ENTRANCE`: a teleporter entrance.
pub const MODE_TELEPORTER_ENTRANCE: c_int = 0;

/// `MODE_TELEPORTER_EXIT`: a teleporter exit.
pub const MODE_TELEPORTER_EXIT: c_int = 1;

/// `OBJ_ATTACHMENT_SAPPER`: a spy's sapper.
pub const OBJ_ATTACHMENT_SAPPER: c_int = 3;

/// `OBJ_DISPENSER`: a dispenser, payload carts' included.
pub const OBJ_DISPENSER: c_int = 0;

/// `OBJ_LAST`: the number of building types.
pub const OBJ_LAST: c_int = 4;

/// `OBJ_SENTRYGUN`: a sentry gun.
pub const OBJ_SENTRYGUN: c_int = 2;

/// `OBJ_TELEPORTER`: a teleporter entrance or exit.
pub const OBJ_TELEPORTER: c_int = 1;

/// `OF_ALLOW_REPEAT_PLACEMENT`: an object flag that lets the builder place
/// another of the building from the same blueprint.
pub const OF_ALLOW_REPEAT_PLACEMENT: c_int = 0x01;

/// `OF_DOESNT_HAVE_A_MODEL`: an object flag of buildings without a model,
/// which sentries do not target.
pub const OF_DOESNT_HAVE_A_MODEL: c_int = 0x04;

/// `OF_MUST_BE_BUILT_ON_ATTACHMENT`: an object flag of buildings placed on
/// another, as sappers are.
pub const OF_MUST_BE_BUILT_ON_ATTACHMENT: c_int = 0x02;

/// `OF_PLAYER_DESTRUCTION`: an object flag of buildings Player Destruction's
/// rules destroy as their builder dies.
pub const OF_PLAYER_DESTRUCTION: c_int = 0x08;

/// `SENTRY_STATE_ATTACKING`: a sentry firing at its enemy.
pub const SENTRY_STATE_ATTACKING: c_int = 2;

/// `SENTRY_STATE_INACTIVE`: a sentry being built, carried or disabled.
pub const SENTRY_STATE_INACTIVE: c_int = 0;

/// `SENTRY_STATE_SEARCHING`: a sentry turning, looking for an enemy.
pub const SENTRY_STATE_SEARCHING: c_int = 1;

/// `SENTRY_STATE_UPGRADING`: a sentry playing its upgrade animation.
pub const SENTRY_STATE_UPGRADING: c_int = 3;

/// `SF_BASEOBJ_INVULN`: the spawn flag of map-placed buildings that take no
/// damage.
pub const SF_BASEOBJ_INVULN: c_int = 1 << 1;

/// `SF_DISPENSER_DONT_HEAL_DISGUISED_SPIES`: the spawn flag of map-placed
/// dispensers that do not heal the other team's disguised spies.
pub const SF_DISPENSER_DONT_HEAL_DISGUISED_SPIES: c_int = SF_BASEOBJ_INVULN << 2;

/// `SF_DISPENSER_IGNORE_LOS`: the spawn flag of map-placed dispensers that
/// heal players they cannot see.
pub const SF_DISPENSER_IGNORE_LOS: c_int = SF_BASEOBJ_INVULN << 1;

/// `SF_SENTRY_INFINITE_AMMO`: the spawn flag of map-placed sentries that
/// never run out of ammunition.
pub const SF_SENTRY_INFINITE_AMMO: c_int = SF_BASEOBJ_INVULN << 2;

/// `SF_SENTRY_UPGRADEABLE`: the spawn flag of map-placed sentries engineers
/// may upgrade.
pub const SF_SENTRY_UPGRADEABLE: c_int = SF_BASEOBJ_INVULN << 1;

/// `SOLID_TO_PLAYER_NO`: a building that blocks the movement of the other
/// team's players only, as the `SolidToPlayer` key and input number it.
pub const SOLID_TO_PLAYER_NO: c_int = 2;

/// `SOLID_TO_PLAYER_USE_DEFAULT`: a building as solid to players as its
/// kind's defaults in `scripts/objects.txt` make it.
pub const SOLID_TO_PLAYER_USE_DEFAULT: c_int = 0;

/// `SOLID_TO_PLAYER_YES`: a building that blocks the movement of every
/// player.
pub const SOLID_TO_PLAYER_YES: c_int = 1;

/// The slot of `CBaseObject::StartUpgrading` in a TF2 building's primary
/// vtable, from the generated binding.
#[doc(alias("StartUpgrading"))]
pub const START_UPGRADING_SLOT: usize =
	vtable_slot!(sys::CBaseObject__bindgen_vtable, CBaseObject_StartUpgrading);

/// `TELEPORTER_STATE_BUILDING`: a teleporter being built.
pub const TELEPORTER_STATE_BUILDING: c_int = 0;

/// `TELEPORTER_STATE_IDLE`: a teleporter without its other end.
pub const TELEPORTER_STATE_IDLE: c_int = 1;

/// `TELEPORTER_STATE_READY`: a teleporter whose ends are both built, and
/// charged.
pub const TELEPORTER_STATE_READY: c_int = 2;

/// `TELEPORTER_STATE_RECEIVING`: an exit about to receive a player.
pub const TELEPORTER_STATE_RECEIVING: c_int = 4;

/// `TELEPORTER_STATE_RECEIVING_RELEASE`: an exit releasing the player it
/// received.
pub const TELEPORTER_STATE_RECEIVING_RELEASE: c_int = 5;

/// `TELEPORTER_STATE_RECHARGING`: a teleporter recharging after a teleport.
pub const TELEPORTER_STATE_RECHARGING: c_int = 6;

/// `TELEPORTER_STATE_SENDING`: an entrance teleporting a player away.
pub const TELEPORTER_STATE_SENDING: c_int = 3;

/// `TELEPORTER_STATE_UPGRADING`: a teleporter playing its upgrade animation.
pub const TELEPORTER_STATE_UPGRADING: c_int = 7;
