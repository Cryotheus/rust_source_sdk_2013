//! Hand-written ABI of TF2's damage handling: the `DMG_*` damage-type bits of
//! `game/shared/shareddefs.h` and the aliases TF2 gives them in
//! `game/shared/tf/tf_shareddefs.h`, including those of healing, `takedamageinfo.h`'s
//! `BASEDAMAGE_NOT_SPECIFIED`, the ammo type `CTakeDamageInfo::Init` leaves,
//! and the vtable slots of a TF2 player's damage methods.
//!
//! The generated bindings omit these `#define`s. The bits are those of
//! `CTakeDamageInfo::m_bitsDamageType`.

use crate::abi::CppDestructors;
use crate::vtable_slot;
use std::ffi::c_int;

pub use crate::entities::health::DMG_GENERIC;

/// The signature of `CTFPlayer::OnTakeDamage` and `OnTakeDamage_Alive`,
/// `int (const CTakeDamageInfo &)`, with the player as its receiver.
#[doc(alias("OnTakeDamage", "OnTakeDamage_Alive"))]
pub type TakeDamageFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, info: *const sys::CTakeDamageInfo) -> c_int;

// SourceMod's `gamedata/sdkhooks.games/engine.ep2v.txt` lists these slots in
// its `tf` section: 64 and 283 on Windows, one more on Linux, whose Itanium
// vtables start with two destructor slots instead of MSVC's one.
const _: () = {
	assert!(ON_TAKE_DAMAGE_SLOT == 63 + CppDestructors::VTABLE_SLOTS);
	assert!(ON_TAKE_DAMAGE_ALIVE_SLOT == 282 + CppDestructors::VTABLE_SLOTS);
};

// `CTFPlayer` keeps `CBaseEntity::OnTakeDamage`'s slot, and the generated
// method has the signature of `TakeDamageFn`.
const _: () = assert!(
	ON_TAKE_DAMAGE_SLOT == vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_OnTakeDamage)
);

const _: fn(&sys::CBaseEntity__bindgen_vtable) -> TakeDamageFn =
	|vtable| vtable.CBaseEntity_OnTakeDamage;

/// The value of `CTakeDamageInfo::m_flBaseDamage` that marks the base damage
/// as unspecified, `FLT_MAX`.
pub const BASEDAMAGE_NOT_SPECIFIED: f32 = f32::MAX;

/// Acid, which TF2 aliases as [`DMG_CRITICAL`].
pub const DMG_ACID: c_int = 1 << 20;

/// Hit by an airboat's gun, which TF2 aliases as [`DMG_USE_HITLOCATIONS`].
pub const DMG_AIRBOAT: c_int = 1 << 25;

/// Lets any damage type gib the victim on death.
pub const DMG_ALWAYSGIB: c_int = 1 << 13;

/// Explosive blast damage.
pub const DMG_BLAST: c_int = 1 << 6;

/// Shotgun pellets: not quite a bullet.
pub const DMG_BUCKSHOT: c_int = 1 << 29;

/// Gunshot damage.
pub const DMG_BULLET: c_int = 1 << 1;

/// Heat burns.
pub const DMG_BURN: c_int = 1 << 3;

/// Blunt impact, such as a crowbar or punch.
pub const DMG_CLUB: c_int = 1 << 7;

/// TF2's critical hit, an alias of [`DMG_ACID`].
pub const DMG_CRITICAL: c_int = DMG_ACID;

/// Crushing by a falling or moving object.
pub const DMG_CRUSH: c_int = 1 << 0;

/// Damage the SDK's `CEntityFlame` deals alongside [`DMG_BURN`].
pub const DMG_DIRECT: c_int = 1 << 28;

/// Drowning.
pub const DMG_DROWN: c_int = 1 << 14;

/// Falling too far.
pub const DMG_FALL: c_int = 1 << 5;

/// TF2's healing marked as exempt from its healing debuffs, an alias of
/// [`DMG_SLASH`]. `CTFPlayer::TakeHealth` has its check commented out, so the
/// bit changes nothing there.
pub const DMG_IGNORE_DEBUFFS: c_int = DMG_SLASH;

/// TF2's healing that may exceed the player's maximum health, as overheal
/// does, an alias of [`DMG_BULLET`] (`CTFPlayer::TakeHealth`).
pub const DMG_IGNORE_MAXHEALTH: c_int = DMG_BULLET;

/// Stops any damage type from gibbing the victim on death.
pub const DMG_NEVERGIB: c_int = 1 << 12;

/// Prevents the damage from applying a physics force.
pub const DMG_PREVENT_PHYSICS_FORCE: c_int = 1 << 11;

/// Electric shock.
pub const DMG_SHOCK: c_int = 1 << 8;

/// Cutting, clawing or stabbing.
pub const DMG_SLASH: c_int = 1 << 2;

/// Burning in an oven, which TF2 aliases as [`DMG_USEDISTANCEMOD`].
pub const DMG_SLOWBURN: c_int = 1 << 21;

/// TF2's damage that uses hit locations, an alias of [`DMG_AIRBOAT`].
pub const DMG_USE_HITLOCATIONS: c_int = DMG_AIRBOAT;

/// TF2's damage that its distance modifiers scale, an alias of
/// [`DMG_SLOWBURN`].
pub const DMG_USEDISTANCEMOD: c_int = DMG_SLOWBURN;

/// Hit by a vehicle.
pub const DMG_VEHICLE: c_int = 1 << 4;

/// The value of `CTakeDamageInfo::m_iAmmoType` for damage with no ammo type,
/// as `CTakeDamageInfo::Init` leaves it (`game/shared/takedamageinfo.cpp`).
pub const NO_AMMO_TYPE: c_int = -1;

/// The slot of `CTFPlayer::OnTakeDamage_Alive` in a TF2 player's primary
/// vtable, from the generated binding.
///
/// The generated method takes a `CTFPlayer` receiver. It is the player's
/// primary base, `CBaseEntity`, at the same address, so the method can be
/// called and hooked as a [`TakeDamageFn`].
#[doc(alias("OnTakeDamage_Alive"))]
pub const ON_TAKE_DAMAGE_ALIVE_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_OnTakeDamage_Alive);

/// The slot of `CTFPlayer::OnTakeDamage` in a TF2 player's primary vtable,
/// from the generated binding.
#[doc(alias("OnTakeDamage"))]
pub const ON_TAKE_DAMAGE_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_OnTakeDamage);
