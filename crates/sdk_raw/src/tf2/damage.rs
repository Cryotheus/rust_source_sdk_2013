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

/// A blast on the surface of water, which cannot harm what is underwater,
/// and which TF2 aliases as [`DMG_MELEE`].
pub const DMG_BLAST_SURFACE: c_int = 1 << 27;

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

/// Dissolving, which TF2 aliases as
/// [`DMG_DONT_COUNT_DAMAGE_TOWARDS_CRIT_RATE`].
pub const DMG_DISSOLVE: c_int = 1 << 26;

/// TF2's damage that does not raise its attacker's chance of random critical
/// hits, an alias of [`DMG_DISSOLVE`].
pub const DMG_DONT_COUNT_DAMAGE_TOWARDS_CRIT_RATE: c_int = DMG_DISSOLVE;

/// Drowning.
pub const DMG_DROWN: c_int = 1 << 14;

/// A laser or other high-energy beam, which TF2 aliases as
/// [`DMG_RADIUS_MAX`].
pub const DMG_ENERGYBEAM: c_int = 1 << 10;

/// Falling too far.
pub const DMG_FALL: c_int = 1 << 5;

/// TF2's radius damage that falls to half at the edge of its radius, an
/// alias of [`DMG_RADIATION`].
pub const DMG_HALF_FALLOFF: c_int = DMG_RADIATION;

/// TF2's damage that sets its victim on fire, an alias of [`DMG_PLASMA`].
pub const DMG_IGNITE: c_int = DMG_PLASMA;

/// TF2's healing marked as exempt from its healing debuffs, an alias of
/// [`DMG_SLASH`]. `CTFPlayer::TakeHealth` has its check commented out, so the
/// bit changes nothing there.
pub const DMG_IGNORE_DEBUFFS: c_int = DMG_SLASH;

/// TF2's healing that may exceed the player's maximum health, as overheal
/// does, an alias of [`DMG_BULLET`] (`CTFPlayer::TakeHealth`).
pub const DMG_IGNORE_MAXHEALTH: c_int = DMG_BULLET;

/// TF2's melee damage, an alias of [`DMG_BLAST_SURFACE`].
pub const DMG_MELEE: c_int = DMG_BLAST_SURFACE;

/// Stops any damage type from gibbing the victim on death.
pub const DMG_NEVERGIB: c_int = 1 << 12;

/// TF2's damage whose distance modifier adds less at close range, an alias
/// of [`DMG_POISON`].
pub const DMG_NOCLOSEDISTANCEMOD: c_int = DMG_POISON;

/// Shot by a plasma weapon, which TF2 aliases as [`DMG_IGNITE`].
pub const DMG_PLASMA: c_int = 1 << 24;

/// Blood poisoning, which TF2 aliases as [`DMG_NOCLOSEDISTANCEMOD`].
pub const DMG_POISON: c_int = 1 << 17;

/// Prevents the damage from applying a physics force.
pub const DMG_PREVENT_PHYSICS_FORCE: c_int = 1 << 11;

/// Radiation, which TF2 aliases as [`DMG_HALF_FALLOFF`].
pub const DMG_RADIATION: c_int = 1 << 18;

/// TF2's radius damage that does not fall off over its radius, an alias of
/// [`DMG_ENERGYBEAM`].
pub const DMG_RADIUS_MAX: c_int = DMG_ENERGYBEAM;

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

/// The signature of `CTFPlayer::DamageEffect`, `void (float, int)`, with the
/// player as its receiver, the damage taken and its `DMG_*` bits.
///
/// The generated method takes a `CTFPlayer` receiver. It is the player's
/// primary base, `CBaseEntity`, at the same address, so the method can be
/// called and hooked with an entity receiver.
#[doc(alias("DamageEffect"))]
pub type DamageEffectFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, damage: f32, damage_type: c_int);

// `DamageEffect` follows `ForceRespawn` (337 on Windows, see
// `crate::tf2::respawn`) by nine slots, as `CBasePlayer` declares it after
// `InitialSpawn`, `InitHUD`, `ShowViewPortPanel`, `PlayerDeathThink`, `Jump`,
// `Duck`, `PreThink` and `PostThink`, none of them overloaded, so MSVC keeps
// their order, and `CommitSuicide` (454, which SourceMod's gamedata lists)
// further on matches the generated vtable too. Linux's Itanium vtables start
// with two destructor slots instead of MSVC's one.
const _: () = {
	assert!(DAMAGE_EFFECT_SLOT == 345 + CppDestructors::VTABLE_SLOTS);
	assert!(DAMAGE_EFFECT_SLOT == crate::tf2::respawn::FORCE_RESPAWN_SLOT + 9);
};

// The generated method has the signature of `DamageEffectFn`.
const _: fn(
	&sys::CTFPlayer__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CTFPlayer, f32, c_int) = |vtable| vtable.CTFPlayer_DamageEffect;

/// The slot of `CTFPlayer::DamageEffect` in a TF2 player's primary vtable,
/// from the generated binding.
///
/// `CTFPlayer::OnTakeDamage` calls it through the vtable for each hit the
/// player took, after the damage and its rules, and after the player's death
/// if the hit killed them (`tf_player.cpp:9604`), to show the hit's effects
/// (`tf_player.cpp:10131-10162`): a red flash of the screen for crushing damage
/// (`DMG_CRUSH`), a blue one for drowning (`DMG_DROWN`), each a `Fade` user
/// message, blood for slashing (`DMG_SLASH`), or the sound of a bullet's
/// impact (`DMG_BULLET`), only the first of these the hit has. The last two
/// are left out for a disguised Spy. `CTFBot` keeps the same function at the
/// slot.
#[doc(alias("DamageEffect"))]
pub const DAMAGE_EFFECT_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_DamageEffect);
