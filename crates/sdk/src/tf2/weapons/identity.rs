//! What a TF2 weapon is beyond its item, and what it is doing.
//!
//! [`Weapon::weapon_id`] reads the id the weapon's C++ class reports
//! (`GetWeaponID`), which every item of a kind of weapon shares, whatever its
//! definition: every rocket launcher but the Direct Hit is a
//! [`WeaponId::ROCKETLAUNCHER`]. [`Weapon::is_sapper`] tells sappers apart
//! from the toolboxes that share their id. The other readers cover what a
//! weapon networks of what it is doing: the ammo it fires, when it began
//! charging, and whether a flame thrower fires critical flames.
//!
//! [`ItemDefinitionIndex`] names the items whose definitions weapons of the
//! same class differ in, such as the fire axes that remove sappers.

#[cfg(test)]
#[path = "../../tests/tf2/weapons/identity.rs"]
mod tests;

use super::{ItemDefinitionIndex, Weapon, WeaponError, check_live};
use crate::datatables::{NetProp, NetVar};
use crate::tf2::ammo::AmmoType;
use sdk_raw::tf2::weapons as raw;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};

/// The id a TF2 weapon's C++ class reports of what it is (`ETFWeaponType`),
/// through `GetWeaponID`, which items of the same class share. Some
/// projectiles' classes report the id of the weapon that fires them too.
///
/// The constants name every id the game's source declares. An id the game
/// adds later is kept as it is.
#[doc(alias("ETFWeaponType", "GetWeaponID"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WeaponId(c_int);

impl WeaponId {
	/// `TF_WEAPON_NONE`: no weapon, which the base classes of melee weapons and
	/// of grenade projectiles report.
	#[doc(alias("TF_WEAPON_NONE"))]
	pub const NONE: Self = Self(raw::TF_WEAPON_NONE);

	/// `TF_WEAPON_BAT`, the weapon id of `CTFBat`.
	#[doc(alias("TF_WEAPON_BAT"))]
	pub const BAT: Self = Self(raw::TF_WEAPON_BAT);

	/// `TF_WEAPON_BAT_WOOD`, the weapon id of `CTFBat_Wood`.
	#[doc(alias("TF_WEAPON_BAT_WOOD"))]
	pub const BAT_WOOD: Self = Self(raw::TF_WEAPON_BAT_WOOD);

	/// `TF_WEAPON_BOTTLE`, the weapon id of `CTFBottle`.
	#[doc(alias("TF_WEAPON_BOTTLE"))]
	pub const BOTTLE: Self = Self(raw::TF_WEAPON_BOTTLE);

	/// `TF_WEAPON_FIREAXE`, the weapon id of `CTFFireAxe`.
	#[doc(alias("TF_WEAPON_FIREAXE"))]
	pub const FIREAXE: Self = Self(raw::TF_WEAPON_FIREAXE);

	/// `TF_WEAPON_CLUB`, the weapon id of `CTFClub`.
	#[doc(alias("TF_WEAPON_CLUB"))]
	pub const CLUB: Self = Self(raw::TF_WEAPON_CLUB);

	/// `TF_WEAPON_CROWBAR`, the weapon id of `CTFCrowbar`.
	#[doc(alias("TF_WEAPON_CROWBAR"))]
	pub const CROWBAR: Self = Self(raw::TF_WEAPON_CROWBAR);

	/// `TF_WEAPON_KNIFE`, the weapon id of `CTFKnife`.
	#[doc(alias("TF_WEAPON_KNIFE"))]
	pub const KNIFE: Self = Self(raw::TF_WEAPON_KNIFE);

	/// `TF_WEAPON_FISTS`, the weapon id of `CTFFists`.
	#[doc(alias("TF_WEAPON_FISTS"))]
	pub const FISTS: Self = Self(raw::TF_WEAPON_FISTS);

	/// `TF_WEAPON_SHOVEL`, the weapon id of `CTFShovel`.
	#[doc(alias("TF_WEAPON_SHOVEL"))]
	pub const SHOVEL: Self = Self(raw::TF_WEAPON_SHOVEL);

	/// `TF_WEAPON_WRENCH`, the weapon id of `CTFWrench`.
	#[doc(alias("TF_WEAPON_WRENCH"))]
	pub const WRENCH: Self = Self(raw::TF_WEAPON_WRENCH);

	/// `TF_WEAPON_BONESAW`, the weapon id of `CTFBonesaw`.
	#[doc(alias("TF_WEAPON_BONESAW"))]
	pub const BONESAW: Self = Self(raw::TF_WEAPON_BONESAW);

	/// `TF_WEAPON_SHOTGUN_PRIMARY`, the weapon id of `CTFShotgun`.
	#[doc(alias("TF_WEAPON_SHOTGUN_PRIMARY"))]
	pub const SHOTGUN_PRIMARY: Self = Self(raw::TF_WEAPON_SHOTGUN_PRIMARY);

	/// `TF_WEAPON_SHOTGUN_SOLDIER`, the weapon id of `CTFShotgun_Soldier`.
	#[doc(alias("TF_WEAPON_SHOTGUN_SOLDIER"))]
	pub const SHOTGUN_SOLDIER: Self = Self(raw::TF_WEAPON_SHOTGUN_SOLDIER);

	/// `TF_WEAPON_SHOTGUN_HWG`, the weapon id of `CTFShotgun_HWG`.
	#[doc(alias("TF_WEAPON_SHOTGUN_HWG"))]
	pub const SHOTGUN_HWG: Self = Self(raw::TF_WEAPON_SHOTGUN_HWG);

	/// `TF_WEAPON_SHOTGUN_PYRO`, the weapon id of `CTFShotgun_Pyro`.
	#[doc(alias("TF_WEAPON_SHOTGUN_PYRO"))]
	pub const SHOTGUN_PYRO: Self = Self(raw::TF_WEAPON_SHOTGUN_PYRO);

	/// `TF_WEAPON_SCATTERGUN`, the weapon id of `CTFScatterGun`.
	#[doc(alias("TF_WEAPON_SCATTERGUN"))]
	pub const SCATTERGUN: Self = Self(raw::TF_WEAPON_SCATTERGUN);

	/// `TF_WEAPON_SNIPERRIFLE`, the weapon id of `CTFSniperRifle`.
	#[doc(alias("TF_WEAPON_SNIPERRIFLE"))]
	pub const SNIPERRIFLE: Self = Self(raw::TF_WEAPON_SNIPERRIFLE);

	/// `TF_WEAPON_MINIGUN`, the weapon id of `CTFMinigun`.
	#[doc(alias("TF_WEAPON_MINIGUN"))]
	pub const MINIGUN: Self = Self(raw::TF_WEAPON_MINIGUN);

	/// `TF_WEAPON_SMG`, the weapon id of `CTFSMG`.
	#[doc(alias("TF_WEAPON_SMG"))]
	pub const SMG: Self = Self(raw::TF_WEAPON_SMG);

	/// `TF_WEAPON_SYRINGEGUN_MEDIC`, the weapon id of `CTFSyringeGun`.
	#[doc(alias("TF_WEAPON_SYRINGEGUN_MEDIC"))]
	pub const SYRINGEGUN_MEDIC: Self = Self(raw::TF_WEAPON_SYRINGEGUN_MEDIC);

	/// `TF_WEAPON_TRANQ`, the weapon id of `CTFTranq`.
	#[doc(alias("TF_WEAPON_TRANQ"))]
	pub const TRANQ: Self = Self(raw::TF_WEAPON_TRANQ);

	/// `TF_WEAPON_ROCKETLAUNCHER`, the weapon id of `CTFBaseRocket`,
	/// `CTFRocketLauncher`, `CTFRocketLauncher_AirStrike` and
	/// `CTFRocketLauncher_Mortar`.
	#[doc(alias("TF_WEAPON_ROCKETLAUNCHER"))]
	pub const ROCKETLAUNCHER: Self = Self(raw::TF_WEAPON_ROCKETLAUNCHER);

	/// `TF_WEAPON_GRENADELAUNCHER`, the weapon id of `CTFGrenadeLauncher`.
	#[doc(alias("TF_WEAPON_GRENADELAUNCHER"))]
	pub const GRENADELAUNCHER: Self = Self(raw::TF_WEAPON_GRENADELAUNCHER);

	/// `TF_WEAPON_PIPEBOMBLAUNCHER`, the weapon id of `CTFPipebombLauncher`.
	#[doc(alias("TF_WEAPON_PIPEBOMBLAUNCHER"))]
	pub const PIPEBOMBLAUNCHER: Self = Self(raw::TF_WEAPON_PIPEBOMBLAUNCHER);

	/// `TF_WEAPON_FLAMETHROWER`, the weapon id of `CTFFlameThrower`.
	#[doc(alias("TF_WEAPON_FLAMETHROWER"))]
	pub const FLAMETHROWER: Self = Self(raw::TF_WEAPON_FLAMETHROWER);

	/// `TF_WEAPON_GRENADE_NORMAL`, the weapon id of `CTFGrenadeNormal` and
	/// `CTFGrenadeNormalProjectile`.
	#[doc(alias("TF_WEAPON_GRENADE_NORMAL"))]
	pub const GRENADE_NORMAL: Self = Self(raw::TF_WEAPON_GRENADE_NORMAL);

	/// `TF_WEAPON_GRENADE_CONCUSSION`, the weapon id of `CTFGrenadeConcussion`
	/// and `CTFGrenadeConcussionProjectile`.
	#[doc(alias("TF_WEAPON_GRENADE_CONCUSSION"))]
	pub const GRENADE_CONCUSSION: Self = Self(raw::TF_WEAPON_GRENADE_CONCUSSION);

	/// `TF_WEAPON_GRENADE_NAIL`, the weapon id of `CTFGrenadeNail` and
	/// `CTFGrenadeNailProjectile`.
	#[doc(alias("TF_WEAPON_GRENADE_NAIL"))]
	pub const GRENADE_NAIL: Self = Self(raw::TF_WEAPON_GRENADE_NAIL);

	/// `TF_WEAPON_GRENADE_MIRV`, the weapon id of `CTFGrenadeMirv` and
	/// `CTFGrenadeMirvProjectile`.
	#[doc(alias("TF_WEAPON_GRENADE_MIRV"))]
	pub const GRENADE_MIRV: Self = Self(raw::TF_WEAPON_GRENADE_MIRV);

	/// `TF_WEAPON_GRENADE_MIRV_DEMOMAN`, the weapon id of
	/// `CTFGrenadeMirv_Demoman`.
	#[doc(alias("TF_WEAPON_GRENADE_MIRV_DEMOMAN"))]
	pub const GRENADE_MIRV_DEMOMAN: Self = Self(raw::TF_WEAPON_GRENADE_MIRV_DEMOMAN);

	/// `TF_WEAPON_GRENADE_NAPALM`, the weapon id of `CTFGrenadeNapalm` and
	/// `CTFGrenadeNapalmProjectile`.
	#[doc(alias("TF_WEAPON_GRENADE_NAPALM"))]
	pub const GRENADE_NAPALM: Self = Self(raw::TF_WEAPON_GRENADE_NAPALM);

	/// `TF_WEAPON_GRENADE_GAS`, the weapon id of `CTFGrenadeGas` and
	/// `CTFGrenadeGasProjectile`.
	#[doc(alias("TF_WEAPON_GRENADE_GAS"))]
	pub const GRENADE_GAS: Self = Self(raw::TF_WEAPON_GRENADE_GAS);

	/// `TF_WEAPON_GRENADE_EMP`, the weapon id of `CTFGrenadeEmp` and
	/// `CTFGrenadeEmpProjectile`.
	#[doc(alias("TF_WEAPON_GRENADE_EMP"))]
	pub const GRENADE_EMP: Self = Self(raw::TF_WEAPON_GRENADE_EMP);

	/// `TF_WEAPON_GRENADE_CALTROP`, the weapon id of `CTFGrenadeCaltrop` and
	/// `CTFGrenadeCaltropProjectile`.
	#[doc(alias("TF_WEAPON_GRENADE_CALTROP"))]
	pub const GRENADE_CALTROP: Self = Self(raw::TF_WEAPON_GRENADE_CALTROP);

	/// `TF_WEAPON_GRENADE_PIPEBOMB`, which no class's `GetWeaponID` returns.
	#[doc(alias("TF_WEAPON_GRENADE_PIPEBOMB"))]
	pub const GRENADE_PIPEBOMB: Self = Self(raw::TF_WEAPON_GRENADE_PIPEBOMB);

	/// `TF_WEAPON_GRENADE_SMOKE_BOMB`, the weapon id of `CTFGrenadeSmokeBomb`.
	#[doc(alias("TF_WEAPON_GRENADE_SMOKE_BOMB"))]
	pub const GRENADE_SMOKE_BOMB: Self = Self(raw::TF_WEAPON_GRENADE_SMOKE_BOMB);

	/// `TF_WEAPON_GRENADE_HEAL`, the weapon id of `CTFGrenadeHeal` and
	/// `CTFGrenadeHealProjectile`.
	#[doc(alias("TF_WEAPON_GRENADE_HEAL"))]
	pub const GRENADE_HEAL: Self = Self(raw::TF_WEAPON_GRENADE_HEAL);

	/// `TF_WEAPON_GRENADE_STUNBALL`, the weapon id of `CTFStunBall`.
	#[doc(alias("TF_WEAPON_GRENADE_STUNBALL"))]
	pub const GRENADE_STUNBALL: Self = Self(raw::TF_WEAPON_GRENADE_STUNBALL);

	/// `TF_WEAPON_GRENADE_JAR`, the weapon id of `CTFProjectile_Jar`.
	#[doc(alias("TF_WEAPON_GRENADE_JAR"))]
	pub const GRENADE_JAR: Self = Self(raw::TF_WEAPON_GRENADE_JAR);

	/// `TF_WEAPON_GRENADE_JAR_MILK`, the weapon id of `CTFProjectile_JarMilk`.
	#[doc(alias("TF_WEAPON_GRENADE_JAR_MILK"))]
	pub const GRENADE_JAR_MILK: Self = Self(raw::TF_WEAPON_GRENADE_JAR_MILK);

	/// `TF_WEAPON_PISTOL`, the weapon id of `CTFPistol`.
	#[doc(alias("TF_WEAPON_PISTOL"))]
	pub const PISTOL: Self = Self(raw::TF_WEAPON_PISTOL);

	/// `TF_WEAPON_PISTOL_SCOUT`, the weapon id of `CTFPistol_Scout`.
	#[doc(alias("TF_WEAPON_PISTOL_SCOUT"))]
	pub const PISTOL_SCOUT: Self = Self(raw::TF_WEAPON_PISTOL_SCOUT);

	/// `TF_WEAPON_REVOLVER`, the weapon id of `CTFRevolver`.
	#[doc(alias("TF_WEAPON_REVOLVER"))]
	pub const REVOLVER: Self = Self(raw::TF_WEAPON_REVOLVER);

	/// `TF_WEAPON_NAILGUN`, the weapon id of `CTFNailgun`.
	#[doc(alias("TF_WEAPON_NAILGUN"))]
	pub const NAILGUN: Self = Self(raw::TF_WEAPON_NAILGUN);

	/// `TF_WEAPON_PDA`, the weapon id of `CTFWeaponPDA`.
	#[doc(alias("TF_WEAPON_PDA"))]
	pub const PDA: Self = Self(raw::TF_WEAPON_PDA);

	/// `TF_WEAPON_PDA_ENGINEER_BUILD`, the weapon id of
	/// `CTFWeaponPDA_Engineer_Build`.
	#[doc(alias("TF_WEAPON_PDA_ENGINEER_BUILD"))]
	pub const PDA_ENGINEER_BUILD: Self = Self(raw::TF_WEAPON_PDA_ENGINEER_BUILD);

	/// `TF_WEAPON_PDA_ENGINEER_DESTROY`, the weapon id of
	/// `CTFWeaponPDA_Engineer_Destroy`.
	#[doc(alias("TF_WEAPON_PDA_ENGINEER_DESTROY"))]
	pub const PDA_ENGINEER_DESTROY: Self = Self(raw::TF_WEAPON_PDA_ENGINEER_DESTROY);

	/// `TF_WEAPON_PDA_SPY`, the weapon id of `CTFWeaponPDA_Spy`.
	#[doc(alias("TF_WEAPON_PDA_SPY"))]
	pub const PDA_SPY: Self = Self(raw::TF_WEAPON_PDA_SPY);

	/// `TF_WEAPON_BUILDER`, the weapon id of `CTFWeaponBuilder`.
	#[doc(alias("TF_WEAPON_BUILDER"))]
	pub const BUILDER: Self = Self(raw::TF_WEAPON_BUILDER);

	/// `TF_WEAPON_MEDIGUN`, the weapon id of `CWeaponMedigun`.
	#[doc(alias("TF_WEAPON_MEDIGUN"))]
	pub const MEDIGUN: Self = Self(raw::TF_WEAPON_MEDIGUN);

	/// `TF_WEAPON_GRENADE_MIRVBOMB`, the weapon id of `CTFGrenadeMirvBomb`.
	#[doc(alias("TF_WEAPON_GRENADE_MIRVBOMB"))]
	pub const GRENADE_MIRVBOMB: Self = Self(raw::TF_WEAPON_GRENADE_MIRVBOMB);

	/// `TF_WEAPON_FLAMETHROWER_ROCKET`, which no class of the game server
	/// reports.
	#[doc(alias("TF_WEAPON_FLAMETHROWER_ROCKET"))]
	pub const FLAMETHROWER_ROCKET: Self = Self(raw::TF_WEAPON_FLAMETHROWER_ROCKET);

	/// `TF_WEAPON_GRENADE_DEMOMAN`, which no class's `GetWeaponID` returns.
	#[doc(alias("TF_WEAPON_GRENADE_DEMOMAN"))]
	pub const GRENADE_DEMOMAN: Self = Self(raw::TF_WEAPON_GRENADE_DEMOMAN);

	/// `TF_WEAPON_SENTRY_BULLET`, which no class's `GetWeaponID` returns.
	#[doc(alias("TF_WEAPON_SENTRY_BULLET"))]
	pub const SENTRY_BULLET: Self = Self(raw::TF_WEAPON_SENTRY_BULLET);

	/// `TF_WEAPON_SENTRY_ROCKET`, which no class's `GetWeaponID` returns.
	#[doc(alias("TF_WEAPON_SENTRY_ROCKET"))]
	pub const SENTRY_ROCKET: Self = Self(raw::TF_WEAPON_SENTRY_ROCKET);

	/// `TF_WEAPON_DISPENSER`, which no class's `GetWeaponID` returns.
	#[doc(alias("TF_WEAPON_DISPENSER"))]
	pub const DISPENSER: Self = Self(raw::TF_WEAPON_DISPENSER);

	/// `TF_WEAPON_INVIS`, the weapon id of `CTFWeaponInvis`.
	#[doc(alias("TF_WEAPON_INVIS"))]
	pub const INVIS: Self = Self(raw::TF_WEAPON_INVIS);

	/// `TF_WEAPON_FLAREGUN`, the weapon id of `CTFFlareGun` and
	/// `CTFProjectile_Flare`.
	#[doc(alias("TF_WEAPON_FLAREGUN"))]
	pub const FLAREGUN: Self = Self(raw::TF_WEAPON_FLAREGUN);

	/// `TF_WEAPON_LUNCHBOX`, the weapon id of `CTFLunchBox`.
	#[doc(alias("TF_WEAPON_LUNCHBOX"))]
	pub const LUNCHBOX: Self = Self(raw::TF_WEAPON_LUNCHBOX);

	/// `TF_WEAPON_JAR`, the weapon id of `CTFJar`.
	#[doc(alias("TF_WEAPON_JAR"))]
	pub const JAR: Self = Self(raw::TF_WEAPON_JAR);

	/// `TF_WEAPON_COMPOUND_BOW`, the weapon id of `CTFCompoundBow`.
	#[doc(alias("TF_WEAPON_COMPOUND_BOW"))]
	pub const COMPOUND_BOW: Self = Self(raw::TF_WEAPON_COMPOUND_BOW);

	/// `TF_WEAPON_BUFF_ITEM`, the weapon id of `CTFBuffItem`.
	#[doc(alias("TF_WEAPON_BUFF_ITEM"))]
	pub const BUFF_ITEM: Self = Self(raw::TF_WEAPON_BUFF_ITEM);

	/// `TF_WEAPON_PUMPKIN_BOMB`, which no class's `GetWeaponID` returns.
	#[doc(alias("TF_WEAPON_PUMPKIN_BOMB"))]
	pub const PUMPKIN_BOMB: Self = Self(raw::TF_WEAPON_PUMPKIN_BOMB);

	/// `TF_WEAPON_SWORD`, the weapon id of `CTFDecapitationMeleeWeaponBase`.
	#[doc(alias("TF_WEAPON_SWORD"))]
	pub const SWORD: Self = Self(raw::TF_WEAPON_SWORD);

	/// `TF_WEAPON_ROCKETLAUNCHER_DIRECTHIT`, the weapon id of
	/// `CTFRocketLauncher_DirectHit`.
	#[doc(alias("TF_WEAPON_ROCKETLAUNCHER_DIRECTHIT"))]
	pub const ROCKETLAUNCHER_DIRECTHIT: Self = Self(raw::TF_WEAPON_ROCKETLAUNCHER_DIRECTHIT);

	/// `TF_WEAPON_LIFELINE`, the weapon id of `CTFDecoy`.
	#[doc(alias("TF_WEAPON_LIFELINE"))]
	pub const LIFELINE: Self = Self(raw::TF_WEAPON_LIFELINE);

	/// `TF_WEAPON_LASER_POINTER`, the weapon id of `CTFLaserPointer`.
	#[doc(alias("TF_WEAPON_LASER_POINTER"))]
	pub const LASER_POINTER: Self = Self(raw::TF_WEAPON_LASER_POINTER);

	/// `TF_WEAPON_DISPENSER_GUN`, which no class's `GetWeaponID` returns.
	#[doc(alias("TF_WEAPON_DISPENSER_GUN"))]
	pub const DISPENSER_GUN: Self = Self(raw::TF_WEAPON_DISPENSER_GUN);

	/// `TF_WEAPON_SENTRY_REVENGE`, the weapon id of `CTFShotgun_Revenge`.
	#[doc(alias("TF_WEAPON_SENTRY_REVENGE"))]
	pub const SENTRY_REVENGE: Self = Self(raw::TF_WEAPON_SENTRY_REVENGE);

	/// `TF_WEAPON_JAR_MILK`, the weapon id of `CTFJarMilk`.
	#[doc(alias("TF_WEAPON_JAR_MILK"))]
	pub const JAR_MILK: Self = Self(raw::TF_WEAPON_JAR_MILK);

	/// `TF_WEAPON_HANDGUN_SCOUT_PRIMARY`, the weapon id of
	/// `CTFPistol_ScoutPrimary`.
	#[doc(alias("TF_WEAPON_HANDGUN_SCOUT_PRIMARY"))]
	pub const HANDGUN_SCOUT_PRIMARY: Self = Self(raw::TF_WEAPON_HANDGUN_SCOUT_PRIMARY);

	/// `TF_WEAPON_BAT_FISH`, the weapon id of `CTFBat_Fish`.
	#[doc(alias("TF_WEAPON_BAT_FISH"))]
	pub const BAT_FISH: Self = Self(raw::TF_WEAPON_BAT_FISH);

	/// `TF_WEAPON_CROSSBOW`, the weapon id of `CTFCrossbow`.
	#[doc(alias("TF_WEAPON_CROSSBOW"))]
	pub const CROSSBOW: Self = Self(raw::TF_WEAPON_CROSSBOW);

	/// `TF_WEAPON_STICKBOMB`, the weapon id of `CTFStickBomb`.
	#[doc(alias("TF_WEAPON_STICKBOMB"))]
	pub const STICKBOMB: Self = Self(raw::TF_WEAPON_STICKBOMB);

	/// `TF_WEAPON_HANDGUN_SCOUT_SECONDARY`, the weapon id of
	/// `CTFPistol_ScoutSecondary`.
	#[doc(alias("TF_WEAPON_HANDGUN_SCOUT_SECONDARY"))]
	pub const HANDGUN_SCOUT_SECONDARY: Self = Self(raw::TF_WEAPON_HANDGUN_SCOUT_SECONDARY);

	/// `TF_WEAPON_SODA_POPPER`, the weapon id of `CTFSodaPopper`.
	#[doc(alias("TF_WEAPON_SODA_POPPER"))]
	pub const SODA_POPPER: Self = Self(raw::TF_WEAPON_SODA_POPPER);

	/// `TF_WEAPON_SNIPERRIFLE_DECAP`, the weapon id of `CTFSniperRifleDecap`.
	#[doc(alias("TF_WEAPON_SNIPERRIFLE_DECAP"))]
	pub const SNIPERRIFLE_DECAP: Self = Self(raw::TF_WEAPON_SNIPERRIFLE_DECAP);

	/// `TF_WEAPON_RAYGUN`, the weapon id of `CTFRaygun`.
	#[doc(alias("TF_WEAPON_RAYGUN"))]
	pub const RAYGUN: Self = Self(raw::TF_WEAPON_RAYGUN);

	/// `TF_WEAPON_PARTICLE_CANNON`, the weapon id of `CTFParticleCannon` and
	/// `CTFProjectile_EnergyBall`.
	#[doc(alias("TF_WEAPON_PARTICLE_CANNON"))]
	pub const PARTICLE_CANNON: Self = Self(raw::TF_WEAPON_PARTICLE_CANNON);

	/// `TF_WEAPON_MECHANICAL_ARM`, the weapon id of `CTFMechanicalArm`.
	#[doc(alias("TF_WEAPON_MECHANICAL_ARM"))]
	pub const MECHANICAL_ARM: Self = Self(raw::TF_WEAPON_MECHANICAL_ARM);

	/// `TF_WEAPON_DRG_POMSON`, the weapon id of `CTFDRGPomson`.
	#[doc(alias("TF_WEAPON_DRG_POMSON"))]
	pub const DRG_POMSON: Self = Self(raw::TF_WEAPON_DRG_POMSON);

	/// `TF_WEAPON_BAT_GIFTWRAP`, the weapon id of `CTFBat_Giftwrap`.
	#[doc(alias("TF_WEAPON_BAT_GIFTWRAP"))]
	pub const BAT_GIFTWRAP: Self = Self(raw::TF_WEAPON_BAT_GIFTWRAP);

	/// `TF_WEAPON_GRENADE_ORNAMENT_BALL`, the weapon id of `CTFBall_Ornament`.
	#[doc(alias("TF_WEAPON_GRENADE_ORNAMENT_BALL"))]
	pub const GRENADE_ORNAMENT_BALL: Self = Self(raw::TF_WEAPON_GRENADE_ORNAMENT_BALL);

	/// `TF_WEAPON_FLAREGUN_REVENGE`, the weapon id of `CTFFlareGun_Revenge`.
	#[doc(alias("TF_WEAPON_FLAREGUN_REVENGE"))]
	pub const FLAREGUN_REVENGE: Self = Self(raw::TF_WEAPON_FLAREGUN_REVENGE);

	/// `TF_WEAPON_PEP_BRAWLER_BLASTER`, the weapon id of
	/// `CTFPEPBrawlerBlaster`.
	#[doc(alias("TF_WEAPON_PEP_BRAWLER_BLASTER"))]
	pub const PEP_BRAWLER_BLASTER: Self = Self(raw::TF_WEAPON_PEP_BRAWLER_BLASTER);

	/// `TF_WEAPON_CLEAVER`, the weapon id of `CTFCleaver`.
	#[doc(alias("TF_WEAPON_CLEAVER"))]
	pub const CLEAVER: Self = Self(raw::TF_WEAPON_CLEAVER);

	/// `TF_WEAPON_GRENADE_CLEAVER`, the weapon id of `CTFProjectile_Cleaver`.
	#[doc(alias("TF_WEAPON_GRENADE_CLEAVER"))]
	pub const GRENADE_CLEAVER: Self = Self(raw::TF_WEAPON_GRENADE_CLEAVER);

	/// `TF_WEAPON_STICKY_BALL_LAUNCHER`, which no class of the game server
	/// reports.
	#[doc(alias("TF_WEAPON_STICKY_BALL_LAUNCHER"))]
	pub const STICKY_BALL_LAUNCHER: Self = Self(raw::TF_WEAPON_STICKY_BALL_LAUNCHER);

	/// `TF_WEAPON_GRENADE_STICKY_BALL`, which no class of the game server
	/// reports.
	#[doc(alias("TF_WEAPON_GRENADE_STICKY_BALL"))]
	pub const GRENADE_STICKY_BALL: Self = Self(raw::TF_WEAPON_GRENADE_STICKY_BALL);

	/// `TF_WEAPON_SHOTGUN_BUILDING_RESCUE`, the weapon id of
	/// `CTFShotgunBuildingRescue`.
	#[doc(alias("TF_WEAPON_SHOTGUN_BUILDING_RESCUE"))]
	pub const SHOTGUN_BUILDING_RESCUE: Self = Self(raw::TF_WEAPON_SHOTGUN_BUILDING_RESCUE);

	/// `TF_WEAPON_CANNON`, the weapon id of `CTFCannon`.
	#[doc(alias("TF_WEAPON_CANNON"))]
	pub const CANNON: Self = Self(raw::TF_WEAPON_CANNON);

	/// `TF_WEAPON_THROWABLE`, the weapon id of `CTFThrowable`.
	#[doc(alias("TF_WEAPON_THROWABLE"))]
	pub const THROWABLE: Self = Self(raw::TF_WEAPON_THROWABLE);

	/// `TF_WEAPON_GRENADE_THROWABLE`, the weapon id of
	/// `CTFProjectile_Throwable`.
	#[doc(alias("TF_WEAPON_GRENADE_THROWABLE"))]
	pub const GRENADE_THROWABLE: Self = Self(raw::TF_WEAPON_GRENADE_THROWABLE);

	/// `TF_WEAPON_PDA_SPY_BUILD`, which no class's `GetWeaponID` returns.
	#[doc(alias("TF_WEAPON_PDA_SPY_BUILD"))]
	pub const PDA_SPY_BUILD: Self = Self(raw::TF_WEAPON_PDA_SPY_BUILD);

	/// `TF_WEAPON_GRENADE_WATERBALLOON`, which no class of the game server
	/// reports.
	#[doc(alias("TF_WEAPON_GRENADE_WATERBALLOON"))]
	pub const GRENADE_WATERBALLOON: Self = Self(raw::TF_WEAPON_GRENADE_WATERBALLOON);

	/// `TF_WEAPON_HARVESTER_SAW`, which no class's `GetWeaponID` returns.
	#[doc(alias("TF_WEAPON_HARVESTER_SAW"))]
	pub const HARVESTER_SAW: Self = Self(raw::TF_WEAPON_HARVESTER_SAW);

	/// `TF_WEAPON_SPELLBOOK`, the weapon id of `CTFSpellBook`.
	#[doc(alias("TF_WEAPON_SPELLBOOK"))]
	pub const SPELLBOOK: Self = Self(raw::TF_WEAPON_SPELLBOOK);

	/// `TF_WEAPON_SPELLBOOK_PROJECTILE`, the weapon id of
	/// `CTFProjectile_SpellBats` and `CTFProjectile_SpellFireball`.
	#[doc(alias("TF_WEAPON_SPELLBOOK_PROJECTILE"))]
	pub const SPELLBOOK_PROJECTILE: Self = Self(raw::TF_WEAPON_SPELLBOOK_PROJECTILE);

	/// `TF_WEAPON_SNIPERRIFLE_CLASSIC`, the weapon id of
	/// `CTFSniperRifleClassic`.
	#[doc(alias("TF_WEAPON_SNIPERRIFLE_CLASSIC"))]
	pub const SNIPERRIFLE_CLASSIC: Self = Self(raw::TF_WEAPON_SNIPERRIFLE_CLASSIC);

	/// `TF_WEAPON_PARACHUTE`, the weapon id of `CTFParachute`.
	#[doc(alias("TF_WEAPON_PARACHUTE"))]
	pub const PARACHUTE: Self = Self(raw::TF_WEAPON_PARACHUTE);

	/// `TF_WEAPON_GRAPPLINGHOOK`, the weapon id of `CTFGrapplingHook`.
	#[doc(alias("TF_WEAPON_GRAPPLINGHOOK"))]
	pub const GRAPPLINGHOOK: Self = Self(raw::TF_WEAPON_GRAPPLINGHOOK);

	/// `TF_WEAPON_PASSTIME_GUN`, the weapon id of `CPasstimeGun`.
	#[doc(alias("TF_WEAPON_PASSTIME_GUN"))]
	pub const PASSTIME_GUN: Self = Self(raw::TF_WEAPON_PASSTIME_GUN);

	/// `TF_WEAPON_CHARGED_SMG`, the weapon id of `CTFChargedSMG`.
	#[doc(alias("TF_WEAPON_CHARGED_SMG"))]
	pub const CHARGED_SMG: Self = Self(raw::TF_WEAPON_CHARGED_SMG);

	/// `TF_WEAPON_BREAKABLE_SIGN`, the weapon id of `CTFBreakableSign`.
	#[doc(alias("TF_WEAPON_BREAKABLE_SIGN"))]
	pub const BREAKABLE_SIGN: Self = Self(raw::TF_WEAPON_BREAKABLE_SIGN);

	/// `TF_WEAPON_ROCKETPACK`, the weapon id of `CTFRocketPack`.
	#[doc(alias("TF_WEAPON_ROCKETPACK"))]
	pub const ROCKETPACK: Self = Self(raw::TF_WEAPON_ROCKETPACK);

	/// `TF_WEAPON_SLAP`, the weapon id of `CTFSlap`.
	#[doc(alias("TF_WEAPON_SLAP"))]
	pub const SLAP: Self = Self(raw::TF_WEAPON_SLAP);

	/// `TF_WEAPON_JAR_GAS`, the weapon id of `CTFJarGas`.
	#[doc(alias("TF_WEAPON_JAR_GAS"))]
	pub const JAR_GAS: Self = Self(raw::TF_WEAPON_JAR_GAS);

	/// `TF_WEAPON_GRENADE_JAR_GAS`, the weapon id of `CTFProjectile_JarGas`.
	#[doc(alias("TF_WEAPON_GRENADE_JAR_GAS"))]
	pub const GRENADE_JAR_GAS: Self = Self(raw::TF_WEAPON_GRENADE_JAR_GAS);

	/// `TF_WEAPON_FLAME_BALL`, the weapon id of `CTFWeaponFlameBall`.
	#[doc(alias("TF_WEAPON_FLAME_BALL"))]
	pub const FLAME_BALL: Self = Self(raw::TF_WEAPON_FLAME_BALL);

	/// The id TF2 numbers `raw`.
	pub const fn from_raw(raw: c_int) -> Self {
		Self(raw)
	}

	/// The id's number.
	pub const fn to_raw(self) -> c_int {
		self.0
	}
}

impl ItemDefinitionIndex {
	/// The Concheror, the Soldier's banner whose rage heals its team.
	pub const CONCHEROR: Self = Self(354);

	/// The Cow Mangler 5000, the Soldier's energy rocket launcher, whose
	/// charged shot dissolves what it kills.
	pub const COW_MANGLER_5000: Self = Self(441);

	/// The Dragon's Fury, the Pyro's flame thrower that fires fireballs.
	pub const DRAGONS_FURY: Self = Self(1178);

	/// The Fists of Steel, the Heavy's defensive fists.
	pub const FISTS_OF_STEEL: Self = Self(331);

	/// The Homewrecker, the Pyro's fire axe that removes sappers.
	pub const HOMEWRECKER: Self = Self(153);

	/// The Maul, the Homewrecker's reskin.
	pub const MAUL: Self = Self(466);

	/// The Neon Annihilator, the Pyro's sign that removes sappers.
	pub const NEON_ANNIHILATOR: Self = Self(813);

	/// The Neon Annihilator's second definition, which only Genuine ones have.
	pub const NEON_ANNIHILATOR_GENUINE: Self = Self(834);

	/// The Phlogistinator, the Pyro's flame thrower whose rage makes its
	/// flames critical.
	pub const PHLOGISTINATOR: Self = Self(594);
}

impl<'s> Weapon<'s> {
	/// The reserve ammo the weapon fires (`m_iPrimaryAmmoType`), or `None` for
	/// a weapon without, such as a melee weapon.
	#[doc(alias("m_iPrimaryAmmoType", "GetPrimaryAmmoType"))]
	pub fn ammo_type(self) -> Result<Option<AmmoType>, WeaponError> {
		Ok(AmmoType::from_raw(self.get(c"m_iPrimaryAmmoType")?))
	}

	/// When the weapon began charging (`m_flChargeBeginTime`), in server time,
	/// or zero while it is not charging. Only the weapons that charge network
	/// it: stickybomb launchers and bows, flame throwers, flare guns, the Cow
	/// Mangler 5000, throwables and sappers. Fails with
	/// [`WeaponError::NetProp`] for the others.
	#[doc(alias("m_flChargeBeginTime", "GetChargeBeginTime"))]
	pub fn charge_begin_time(self) -> Result<f32, WeaponError> {
		self.get(c"m_flChargeBeginTime")
	}

	/// Reads one of the weapon's networked variables.
	fn get<T: NetVar>(self, name: &CStr) -> Result<T, WeaponError> {
		check_live(self.entity)?;

		Ok(self.net_prop(name)?.get(self.entity)?)
	}

	/// Whether a flame thrower fires critical flames (`m_bCritFire`), as with
	/// critical boosts and the Phlogistinator's rage. Fails with
	/// [`WeaponError::NetProp`] for weapons other than flame throwers.
	#[doc(alias("m_bCritFire"))]
	pub fn is_firing_crits(self) -> Result<bool, WeaponError> {
		self.get(c"m_bCritFire")
	}

	/// Whether the weapon is a spy's sapper (`CTFWeaponSapper`), which reports
	/// the id of the toolboxes engineers carry blueprints with,
	/// [`WeaponId::BUILDER`].
	#[doc(alias("CTFWeaponSapper", "tf_weapon_sapper"))]
	pub fn is_sapper(self) -> bool {
		self.entity
			.server_class()
			.is_some_and(|class| class.name() == c"CTFWeaponSapper")
	}

	/// Resolves one of the weapon's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, WeaponError> {
		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.entity, name)?)
	}

	/// The initial afterburn duration and duration added by each hit, in seconds.
	/// Reads the weapon's virtual getters, including flame thrower overrides.
	/// These values precede the caller's burn-time attribute multiplier.
	#[doc(alias("GetInitialAfterburnDuration", "GetAfterburnRateOnHit"))]
	pub fn afterburn_duration(self) -> Result<(f32, f32), WeaponError> {
		check_live(self.entity)?;
		let weapon = self
			.entity
			.as_ptr()
			.cast::<sys::CTFWeaponBase>()
			.cast_const();
		// SAFETY: `new` verified CTFWeaponBase; these generated const getters
		// return scalars and retain no callback-scoped pointers.
		Ok(unsafe {
			(
				vcall!(weapon as sys::CTFWeaponBase__bindgen_vtable => CTFWeaponBase_GetInitialAfterburnDuration()),
				vcall!(weapon as sys::CTFWeaponBase__bindgen_vtable => CTFWeaponBase_GetAfterburnRateOnHit()),
			)
		})
	}

	/// The id the weapon's C++ class reports (`GetWeaponID`), such as
	/// [`WeaponId::FLAMETHROWER`] for every flame thrower but the Dragon's
	/// Fury.
	#[doc(alias("GetWeaponID"))]
	pub fn weapon_id(self) -> Result<WeaponId, WeaponError> {
		check_live(self.entity)?;

		let weapon = self
			.entity
			.as_ptr()
			.cast::<sys::CTFWeaponBase>()
			.cast_const();

		// SAFETY: `new` found `CTFWeaponBase` in the weapon's datamaps, so it is
		// one, whose entity base `sdk_raw::tf2::weapons` asserts is at offset
		// zero. The generated GetWeaponID entry only returns its class's id.
		Ok(WeaponId(unsafe {
			vcall!(weapon as sys::CTFWeaponBase__bindgen_vtable => CTFWeaponBase_GetWeaponID())
		}))
	}
}
