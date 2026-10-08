//! Hand-written ABI of TF2's weapons, and the ids their classes report
//! (`ETFWeaponType` in `game/shared/tf/tf_shareddefs.h`), which the bindings
//! leave out.
//!
//! A `CBaseEntity *` to a TF2 weapon is also a `CTFWeaponBase *` and a
//! `CBaseCombatWeapon *`, the types the generated vtables of those classes
//! take: every class from them down to `CBaseEntity` starts with its primary
//! base, as this module asserts against the generated layouts. The Itanium
//! bindings describe `CBaseCombatWeapon` and `CBaseAnimating` as opaque blobs,
//! whose single, polymorphic primary bases that ABI also places first. The
//! players holding weapons are covered by [the `tf2` module](crate::tf2).

use std::ffi::c_int;
use std::mem::offset_of;

const _: () = {
	assert!(offset_of!(sys::CTFWeaponBase, _base) == 0 && offset_of!(sys::CEconEntity, _base) == 0);

	#[cfg(target_os = "windows")]
	assert!(
		offset_of!(sys::CBaseCombatWeapon, _base) == 0
			&& offset_of!(sys::CBaseAnimating, _base) == 0
	);
};

/// The item definition index that names no item,
/// `(item_definition_index_t)-1`.
///
/// This is `INVALID_ITEM_DEF_INDEX` from
/// `game/shared/econ/econ_item_constants.h`.
pub const INVALID_ITEM_DEF_INDEX: sys::item_definition_index_t = sys::item_definition_index_t::MAX;

/// `TF_WEAPON_BAT`, the weapon id of `CTFBat`.
pub const TF_WEAPON_BAT: c_int = 1;

/// `TF_WEAPON_BAT_FISH`, the weapon id of `CTFBat_Fish`.
pub const TF_WEAPON_BAT_FISH: c_int = 72;

/// `TF_WEAPON_BAT_GIFTWRAP`, the weapon id of `CTFBat_Giftwrap`.
pub const TF_WEAPON_BAT_GIFTWRAP: c_int = 82;

/// `TF_WEAPON_BAT_WOOD`, the weapon id of `CTFBat_Wood`.
pub const TF_WEAPON_BAT_WOOD: c_int = 2;

/// `TF_WEAPON_BONESAW`, the weapon id of `CTFBonesaw`.
pub const TF_WEAPON_BONESAW: c_int = 11;

/// `TF_WEAPON_BOTTLE`, the weapon id of `CTFBottle`.
pub const TF_WEAPON_BOTTLE: c_int = 3;

/// `TF_WEAPON_BREAKABLE_SIGN`, the weapon id of `CTFBreakableSign`.
pub const TF_WEAPON_BREAKABLE_SIGN: c_int = 104;

/// `TF_WEAPON_BUFF_ITEM`, the weapon id of `CTFBuffItem`.
pub const TF_WEAPON_BUFF_ITEM: c_int = 62;

/// `TF_WEAPON_BUILDER`, the weapon id of `CTFWeaponBuilder`.
pub const TF_WEAPON_BUILDER: c_int = 49;

/// `TF_WEAPON_CANNON`, the weapon id of `CTFCannon`.
pub const TF_WEAPON_CANNON: c_int = 91;

/// `TF_WEAPON_CHARGED_SMG`, the weapon id of `CTFChargedSMG`.
pub const TF_WEAPON_CHARGED_SMG: c_int = 103;

/// `TF_WEAPON_CLEAVER`, the weapon id of `CTFCleaver`.
pub const TF_WEAPON_CLEAVER: c_int = 86;

/// `TF_WEAPON_CLUB`, the weapon id of `CTFClub`.
pub const TF_WEAPON_CLUB: c_int = 5;

/// `TF_WEAPON_COMPOUND_BOW`, the weapon id of `CTFCompoundBow`.
pub const TF_WEAPON_COMPOUND_BOW: c_int = 61;

/// `TF_WEAPON_COUNT`: the number of weapon ids.
pub const TF_WEAPON_COUNT: c_int = 110;

/// `TF_WEAPON_CROSSBOW`, the weapon id of `CTFCrossbow`.
pub const TF_WEAPON_CROSSBOW: c_int = 73;

/// `TF_WEAPON_CROWBAR`, the weapon id of `CTFCrowbar`.
pub const TF_WEAPON_CROWBAR: c_int = 6;

/// `TF_WEAPON_DISPENSER`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_DISPENSER: c_int = 56;

/// `TF_WEAPON_DISPENSER_GUN`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_DISPENSER_GUN: c_int = 68;

/// `TF_WEAPON_DRG_POMSON`, the weapon id of `CTFDRGPomson`.
pub const TF_WEAPON_DRG_POMSON: c_int = 81;

/// `TF_WEAPON_FIREAXE`, the weapon id of `CTFFireAxe`.
pub const TF_WEAPON_FIREAXE: c_int = 4;

/// `TF_WEAPON_FISTS`, the weapon id of `CTFFists`.
pub const TF_WEAPON_FISTS: c_int = 8;

/// `TF_WEAPON_FLAME_BALL`, the weapon id of `CTFWeaponFlameBall`.
pub const TF_WEAPON_FLAME_BALL: c_int = 109;

/// `TF_WEAPON_FLAMETHROWER`, the weapon id of `CTFFlameThrower`.
pub const TF_WEAPON_FLAMETHROWER: c_int = 25;

/// `TF_WEAPON_FLAMETHROWER_ROCKET`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_FLAMETHROWER_ROCKET: c_int = 52;

/// `TF_WEAPON_FLAREGUN`, the weapon id of `CTFFlareGun` and
/// `CTFProjectile_Flare`.
pub const TF_WEAPON_FLAREGUN: c_int = 58;

/// `TF_WEAPON_FLAREGUN_REVENGE`, the weapon id of `CTFFlareGun_Revenge`.
pub const TF_WEAPON_FLAREGUN_REVENGE: c_int = 84;

/// `TF_WEAPON_GRAPPLINGHOOK`, the weapon id of `CTFGrapplingHook`.
pub const TF_WEAPON_GRAPPLINGHOOK: c_int = 101;

/// `TF_WEAPON_GRENADE_CALTROP`, the weapon id of `CTFGrenadeCaltrop` and
/// `CTFGrenadeCaltropProjectile`.
pub const TF_WEAPON_GRENADE_CALTROP: c_int = 34;

/// `TF_WEAPON_GRENADE_CLEAVER`, the weapon id of `CTFProjectile_Cleaver`.
pub const TF_WEAPON_GRENADE_CLEAVER: c_int = 87;

/// `TF_WEAPON_GRENADE_CONCUSSION`, the weapon id of `CTFGrenadeConcussion` and
/// `CTFGrenadeConcussionProjectile`.
pub const TF_WEAPON_GRENADE_CONCUSSION: c_int = 27;

/// `TF_WEAPON_GRENADE_DEMOMAN`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_GRENADE_DEMOMAN: c_int = 53;

/// `TF_WEAPON_GRENADE_EMP`, the weapon id of `CTFGrenadeEmp` and
/// `CTFGrenadeEmpProjectile`.
pub const TF_WEAPON_GRENADE_EMP: c_int = 33;

/// `TF_WEAPON_GRENADE_GAS`, the weapon id of `CTFGrenadeGas` and
/// `CTFGrenadeGasProjectile`.
pub const TF_WEAPON_GRENADE_GAS: c_int = 32;

/// `TF_WEAPON_GRENADE_HEAL`, the weapon id of `CTFGrenadeHeal` and
/// `CTFGrenadeHealProjectile`.
pub const TF_WEAPON_GRENADE_HEAL: c_int = 37;

/// `TF_WEAPON_GRENADE_JAR`, the weapon id of `CTFProjectile_Jar`.
pub const TF_WEAPON_GRENADE_JAR: c_int = 39;

/// `TF_WEAPON_GRENADE_JAR_GAS`, the weapon id of `CTFProjectile_JarGas`.
pub const TF_WEAPON_GRENADE_JAR_GAS: c_int = 108;

/// `TF_WEAPON_GRENADE_JAR_MILK`, the weapon id of `CTFProjectile_JarMilk`.
pub const TF_WEAPON_GRENADE_JAR_MILK: c_int = 40;

/// `TF_WEAPON_GRENADE_MIRV`, the weapon id of `CTFGrenadeMirv` and
/// `CTFGrenadeMirvProjectile`.
pub const TF_WEAPON_GRENADE_MIRV: c_int = 29;

/// `TF_WEAPON_GRENADE_MIRV_DEMOMAN`, the weapon id of `CTFGrenadeMirv_Demoman`.
pub const TF_WEAPON_GRENADE_MIRV_DEMOMAN: c_int = 30;

/// `TF_WEAPON_GRENADE_MIRVBOMB`, the weapon id of `CTFGrenadeMirvBomb`.
pub const TF_WEAPON_GRENADE_MIRVBOMB: c_int = 51;

/// `TF_WEAPON_GRENADE_NAIL`, the weapon id of `CTFGrenadeNail` and
/// `CTFGrenadeNailProjectile`.
pub const TF_WEAPON_GRENADE_NAIL: c_int = 28;

/// `TF_WEAPON_GRENADE_NAPALM`, the weapon id of `CTFGrenadeNapalm` and
/// `CTFGrenadeNapalmProjectile`.
pub const TF_WEAPON_GRENADE_NAPALM: c_int = 31;

/// `TF_WEAPON_GRENADE_NORMAL`, the weapon id of `CTFGrenadeNormal` and
/// `CTFGrenadeNormalProjectile`.
pub const TF_WEAPON_GRENADE_NORMAL: c_int = 26;

/// `TF_WEAPON_GRENADE_ORNAMENT_BALL`, the weapon id of `CTFBall_Ornament`.
pub const TF_WEAPON_GRENADE_ORNAMENT_BALL: c_int = 83;

/// `TF_WEAPON_GRENADE_PIPEBOMB`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_GRENADE_PIPEBOMB: c_int = 35;

/// `TF_WEAPON_GRENADE_SMOKE_BOMB`, the weapon id of `CTFGrenadeSmokeBomb`.
pub const TF_WEAPON_GRENADE_SMOKE_BOMB: c_int = 36;

/// `TF_WEAPON_GRENADE_STICKY_BALL`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_GRENADE_STICKY_BALL: c_int = 89;

/// `TF_WEAPON_GRENADE_STUNBALL`, the weapon id of `CTFStunBall`.
pub const TF_WEAPON_GRENADE_STUNBALL: c_int = 38;

/// `TF_WEAPON_GRENADE_THROWABLE`, the weapon id of `CTFProjectile_Throwable`.
pub const TF_WEAPON_GRENADE_THROWABLE: c_int = 93;

/// `TF_WEAPON_GRENADE_WATERBALLOON`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_GRENADE_WATERBALLOON: c_int = 95;

/// `TF_WEAPON_GRENADELAUNCHER`, the weapon id of `CTFGrenadeLauncher`.
pub const TF_WEAPON_GRENADELAUNCHER: c_int = 23;

/// `TF_WEAPON_HANDGUN_SCOUT_PRIMARY`, the weapon id of
/// `CTFPistol_ScoutPrimary`.
pub const TF_WEAPON_HANDGUN_SCOUT_PRIMARY: c_int = 71;

/// `TF_WEAPON_HANDGUN_SCOUT_SECONDARY`, the weapon id of
/// `CTFPistol_ScoutSecondary`.
pub const TF_WEAPON_HANDGUN_SCOUT_SECONDARY: c_int = 75;

/// `TF_WEAPON_HARVESTER_SAW`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_HARVESTER_SAW: c_int = 96;

/// `TF_WEAPON_INVIS`, the weapon id of `CTFWeaponInvis`.
pub const TF_WEAPON_INVIS: c_int = 57;

/// `TF_WEAPON_JAR`, the weapon id of `CTFJar`.
pub const TF_WEAPON_JAR: c_int = 60;

/// `TF_WEAPON_JAR_GAS`, the weapon id of `CTFJarGas`.
pub const TF_WEAPON_JAR_GAS: c_int = 107;

/// `TF_WEAPON_JAR_MILK`, the weapon id of `CTFJarMilk`.
pub const TF_WEAPON_JAR_MILK: c_int = 70;

/// `TF_WEAPON_KNIFE`, the weapon id of `CTFKnife`.
pub const TF_WEAPON_KNIFE: c_int = 7;

/// `TF_WEAPON_LASER_POINTER`, the weapon id of `CTFLaserPointer`.
pub const TF_WEAPON_LASER_POINTER: c_int = 67;

/// `TF_WEAPON_LIFELINE`, the weapon id of `CTFDecoy`.
pub const TF_WEAPON_LIFELINE: c_int = 66;

/// `TF_WEAPON_LUNCHBOX`, the weapon id of `CTFLunchBox`.
pub const TF_WEAPON_LUNCHBOX: c_int = 59;

/// `TF_WEAPON_MECHANICAL_ARM`, the weapon id of `CTFMechanicalArm`.
pub const TF_WEAPON_MECHANICAL_ARM: c_int = 80;

/// `TF_WEAPON_MEDIGUN`, the weapon id of `CWeaponMedigun`.
pub const TF_WEAPON_MEDIGUN: c_int = 50;

/// `TF_WEAPON_MINIGUN`, the weapon id of `CTFMinigun`.
pub const TF_WEAPON_MINIGUN: c_int = 18;

/// `TF_WEAPON_NAILGUN`, the weapon id of `CTFNailgun`.
pub const TF_WEAPON_NAILGUN: c_int = 44;

/// `TF_WEAPON_NONE`: no weapon, which the base classes of melee weapons and of
/// grenade projectiles report.
pub const TF_WEAPON_NONE: c_int = 0;

/// `TF_WEAPON_PARACHUTE`, the weapon id of `CTFParachute`.
pub const TF_WEAPON_PARACHUTE: c_int = 100;

/// `TF_WEAPON_PARTICLE_CANNON`, the weapon id of `CTFParticleCannon` and
/// `CTFProjectile_EnergyBall`.
pub const TF_WEAPON_PARTICLE_CANNON: c_int = 79;

/// `TF_WEAPON_PASSTIME_GUN`, the weapon id of `CPasstimeGun`.
pub const TF_WEAPON_PASSTIME_GUN: c_int = 102;

/// `TF_WEAPON_PDA`, the weapon id of `CTFWeaponPDA`.
pub const TF_WEAPON_PDA: c_int = 45;

/// `TF_WEAPON_PDA_ENGINEER_BUILD`, the weapon id of
/// `CTFWeaponPDA_Engineer_Build`.
pub const TF_WEAPON_PDA_ENGINEER_BUILD: c_int = 46;

/// `TF_WEAPON_PDA_ENGINEER_DESTROY`, the weapon id of
/// `CTFWeaponPDA_Engineer_Destroy`.
pub const TF_WEAPON_PDA_ENGINEER_DESTROY: c_int = 47;

/// `TF_WEAPON_PDA_SPY`, the weapon id of `CTFWeaponPDA_Spy`.
pub const TF_WEAPON_PDA_SPY: c_int = 48;

/// `TF_WEAPON_PDA_SPY_BUILD`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_PDA_SPY_BUILD: c_int = 94;

/// `TF_WEAPON_PEP_BRAWLER_BLASTER`, the weapon id of `CTFPEPBrawlerBlaster`.
pub const TF_WEAPON_PEP_BRAWLER_BLASTER: c_int = 85;

/// `TF_WEAPON_PIPEBOMBLAUNCHER`, the weapon id of `CTFPipebombLauncher`.
pub const TF_WEAPON_PIPEBOMBLAUNCHER: c_int = 24;

/// `TF_WEAPON_PISTOL`, the weapon id of `CTFPistol`.
pub const TF_WEAPON_PISTOL: c_int = 41;

/// `TF_WEAPON_PISTOL_SCOUT`, the weapon id of `CTFPistol_Scout`.
pub const TF_WEAPON_PISTOL_SCOUT: c_int = 42;

/// `TF_WEAPON_PUMPKIN_BOMB`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_PUMPKIN_BOMB: c_int = 63;

/// `TF_WEAPON_RAYGUN`, the weapon id of `CTFRaygun`.
pub const TF_WEAPON_RAYGUN: c_int = 78;

/// `TF_WEAPON_REVOLVER`, the weapon id of `CTFRevolver`.
pub const TF_WEAPON_REVOLVER: c_int = 43;

/// `TF_WEAPON_ROCKETLAUNCHER`, the weapon id of `CTFBaseRocket`,
/// `CTFRocketLauncher`, `CTFRocketLauncher_AirStrike` and
/// `CTFRocketLauncher_Mortar`.
pub const TF_WEAPON_ROCKETLAUNCHER: c_int = 22;

/// `TF_WEAPON_ROCKETLAUNCHER_DIRECTHIT`, the weapon id of
/// `CTFRocketLauncher_DirectHit`.
pub const TF_WEAPON_ROCKETLAUNCHER_DIRECTHIT: c_int = 65;

/// `TF_WEAPON_ROCKETPACK`, the weapon id of `CTFRocketPack`.
pub const TF_WEAPON_ROCKETPACK: c_int = 105;

/// `TF_WEAPON_SCATTERGUN`, the weapon id of `CTFScatterGun`.
pub const TF_WEAPON_SCATTERGUN: c_int = 16;

/// `TF_WEAPON_SENTRY_BULLET`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_SENTRY_BULLET: c_int = 54;

/// `TF_WEAPON_SENTRY_REVENGE`, the weapon id of `CTFShotgun_Revenge`.
pub const TF_WEAPON_SENTRY_REVENGE: c_int = 69;

/// `TF_WEAPON_SENTRY_ROCKET`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_SENTRY_ROCKET: c_int = 55;

/// `TF_WEAPON_SHOTGUN_BUILDING_RESCUE`, the weapon id of
/// `CTFShotgunBuildingRescue`.
pub const TF_WEAPON_SHOTGUN_BUILDING_RESCUE: c_int = 90;

/// `TF_WEAPON_SHOTGUN_HWG`, the weapon id of `CTFShotgun_HWG`.
pub const TF_WEAPON_SHOTGUN_HWG: c_int = 14;

/// `TF_WEAPON_SHOTGUN_PRIMARY`, the weapon id of `CTFShotgun`.
pub const TF_WEAPON_SHOTGUN_PRIMARY: c_int = 12;

/// `TF_WEAPON_SHOTGUN_PYRO`, the weapon id of `CTFShotgun_Pyro`.
pub const TF_WEAPON_SHOTGUN_PYRO: c_int = 15;

/// `TF_WEAPON_SHOTGUN_SOLDIER`, the weapon id of `CTFShotgun_Soldier`.
pub const TF_WEAPON_SHOTGUN_SOLDIER: c_int = 13;

/// `TF_WEAPON_SHOVEL`, the weapon id of `CTFShovel`.
pub const TF_WEAPON_SHOVEL: c_int = 9;

/// `TF_WEAPON_SLAP`, the weapon id of `CTFSlap`.
pub const TF_WEAPON_SLAP: c_int = 106;

/// `TF_WEAPON_SMG`, the weapon id of `CTFSMG`.
pub const TF_WEAPON_SMG: c_int = 19;

/// `TF_WEAPON_SNIPERRIFLE`, the weapon id of `CTFSniperRifle`.
pub const TF_WEAPON_SNIPERRIFLE: c_int = 17;

/// `TF_WEAPON_SNIPERRIFLE_CLASSIC`, the weapon id of `CTFSniperRifleClassic`.
pub const TF_WEAPON_SNIPERRIFLE_CLASSIC: c_int = 99;

/// `TF_WEAPON_SNIPERRIFLE_DECAP`, the weapon id of `CTFSniperRifleDecap`.
pub const TF_WEAPON_SNIPERRIFLE_DECAP: c_int = 77;

/// `TF_WEAPON_SODA_POPPER`, the weapon id of `CTFSodaPopper`.
pub const TF_WEAPON_SODA_POPPER: c_int = 76;

/// `TF_WEAPON_SPELLBOOK`, the weapon id of `CTFSpellBook`.
pub const TF_WEAPON_SPELLBOOK: c_int = 97;

/// `TF_WEAPON_SPELLBOOK_PROJECTILE`, the weapon id of `CTFProjectile_SpellBats`
/// and `CTFProjectile_SpellFireball`.
pub const TF_WEAPON_SPELLBOOK_PROJECTILE: c_int = 98;

/// `TF_WEAPON_STICKBOMB`, the weapon id of `CTFStickBomb`.
pub const TF_WEAPON_STICKBOMB: c_int = 74;

/// `TF_WEAPON_STICKY_BALL_LAUNCHER`, which no class's `GetWeaponID` returns.
pub const TF_WEAPON_STICKY_BALL_LAUNCHER: c_int = 88;

/// `TF_WEAPON_SWORD`, the weapon id of `CTFDecapitationMeleeWeaponBase`.
pub const TF_WEAPON_SWORD: c_int = 64;

/// `TF_WEAPON_SYRINGEGUN_MEDIC`, the weapon id of `CTFSyringeGun`.
pub const TF_WEAPON_SYRINGEGUN_MEDIC: c_int = 20;

/// `TF_WEAPON_THROWABLE`, the weapon id of `CTFThrowable`.
pub const TF_WEAPON_THROWABLE: c_int = 92;

/// `TF_WEAPON_TRANQ`, the weapon id of `CTFTranq`.
pub const TF_WEAPON_TRANQ: c_int = 21;

/// `TF_WEAPON_WRENCH`, the weapon id of `CTFWrench`.
pub const TF_WEAPON_WRENCH: c_int = 10;
