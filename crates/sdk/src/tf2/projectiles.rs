//! TF2's projectiles: the rockets, grenades, arrows, jars, syringes and
//! spells weapons and sentries fire, and the flame managers that carry flame
//! throwers' flames.
//!
//! [`Projectile`] reads who fired a projectile and with what, how often it was
//! deflected and by whom, and what its kind networks, such as whether a
//! stickybomb has stuck. It reads and writes whether the projectile is
//! critical and the damage it does. [`ProjectileKind`] names the projectiles'
//! entity classes, and [`ProjectileFamily`] the three C++ classes they derive
//! from, which decide what they network. [`FlameManager`] reads whose flames
//! a flame manager carries.
//!
//! [`sdk_raw::tf2::projectiles`] holds TF2's numbers for projectiles.
//!
//! # Owners
//!
//! Rockets, flares, arrows and syringes are owned by the player who fired
//! them, and sentries' rockets by the sentry. Grenades, which jars, balls and
//! most spells are, have no owner: [`Projectile::thrower`] is the player who
//! threw them. A deflection makes the deflecting player the owner, or the
//! thrower, of what it redirects, and their weapon its launcher, but keeps
//! its original launcher.

#[cfg(test)]
#[path = "../tests/tf2/projectiles.rs"]
mod tests;

use crate::datatables::{NetProp, NetPropError, NetVar};
use crate::entities::Entity;
use crate::{Game, InterfaceError, Server};
use sdk_raw::tf2::projectiles as raw;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};

/// The entity classes of projectiles, as a pattern
/// [`ServerTools::find_by_class_name`](crate::interfaces::ServerTools::find_by_class_name)
/// matches.
const CLASSES: &CStr = c"tf_projectile_*";

/// What an arrow is (`m_iProjectileType`), which arrows of the same class
/// tell apart, as TF2 numbers them (`ProjectileType_t`).
#[doc(alias("m_iProjectileType", "ProjectileType_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArrowKind {
	/// An arrow of the Huntsman or the Fortified Compound.
	#[doc(alias("TF_PROJECTILE_ARROW"))]
	Arrow,

	/// A bolt of the Rescue Ranger, which repairs buildings of its team.
	#[doc(alias("TF_PROJECTILE_BUILDING_REPAIR_BOLT"))]
	BuildingRepairBolt,

	/// An arrow of the Festive Huntsman.
	#[doc(alias("TF_PROJECTILE_FESTIVE_ARROW"))]
	FestiveArrow,

	/// A bolt of the Festive Crusader's Crossbow, which heals teammates.
	#[doc(alias("TF_PROJECTILE_FESTIVE_HEALING_BOLT"))]
	FestiveHealingBolt,

	/// The hook of the Grappling Hook.
	#[doc(alias("TF_PROJECTILE_GRAPPLINGHOOK"))]
	GrapplingHook,

	/// A bolt of the Crusader's Crossbow, which heals teammates.
	#[doc(alias("TF_PROJECTILE_HEALING_BOLT"))]
	HealingBolt,
}

impl ArrowKind {
	/// The kind TF2 numbers `raw`, or `None` for a type that is not an
	/// arrow's.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		Some(match raw {
			raw::TF_PROJECTILE_ARROW => Self::Arrow,
			raw::TF_PROJECTILE_BUILDING_REPAIR_BOLT => Self::BuildingRepairBolt,
			raw::TF_PROJECTILE_FESTIVE_ARROW => Self::FestiveArrow,
			raw::TF_PROJECTILE_FESTIVE_HEALING_BOLT => Self::FestiveHealingBolt,
			raw::TF_PROJECTILE_GRAPPLINGHOOK => Self::GrapplingHook,
			raw::TF_PROJECTILE_HEALING_BOLT => Self::HealingBolt,
			_ => return None,
		})
	}

	/// TF2's number for the kind.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Arrow => raw::TF_PROJECTILE_ARROW,
			Self::BuildingRepairBolt => raw::TF_PROJECTILE_BUILDING_REPAIR_BOLT,
			Self::FestiveArrow => raw::TF_PROJECTILE_FESTIVE_ARROW,
			Self::FestiveHealingBolt => raw::TF_PROJECTILE_FESTIVE_HEALING_BOLT,
			Self::GrapplingHook => raw::TF_PROJECTILE_GRAPPLINGHOOK,
			Self::HealingBolt => raw::TF_PROJECTILE_HEALING_BOLT,
		}
	}
}

/// A flame manager (`tf_flame_manager`), which carries the flames a flame
/// thrower fires, scoped to one engine callback. The flame thrower owns it.
#[doc(alias("CTFFlameManager", "tf_flame_manager"))]
#[derive(Debug, Clone, Copy)]
pub struct FlameManager<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
}

impl<'s> FlameManager<'s> {
	/// Wraps a flame manager, or fails with [`ProjectileError::WrongGame`]
	/// unless the server runs TF2, and [`ProjectileError::NotFlameManager`]
	/// unless `entity`'s datamaps include `CTFFlameManager`.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ProjectileError> {
		if server.game() != Game::TeamFortress2 {
			return Err(ProjectileError::WrongGame);
		}

		if !entity.has_data_map_class(c"CTFFlameManager") {
			return Err(ProjectileError::NotFlameManager);
		}

		Ok(Self { server, entity })
	}

	/// The player the flames burn for (`m_hAttacker`): the owner of the flame
	/// thrower, or `None` if they no longer exist.
	#[doc(alias("m_hAttacker", "GetAttacker"))]
	pub fn attacker(self) -> Result<Option<Entity<'s>>, ProjectileError> {
		handle(self.server, self.entity, c"m_hAttacker")
	}

	/// The flame manager's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Whether the flame thrower is firing flames (`m_bIsFiring`).
	#[doc(alias("m_bIsFiring"))]
	pub fn is_firing(self) -> Result<bool, ProjectileError> {
		get(self.server, self.entity, c"m_bIsFiring")
	}

	/// The flame thrower whose flames the manager carries (`m_hWeapon`), or
	/// `None` if it no longer exists.
	#[doc(alias("m_hWeapon"))]
	pub fn weapon(self) -> Result<Option<Entity<'s>>, ProjectileError> {
		handle(self.server, self.entity, c"m_hWeapon")
	}
}

/// What a pipebomb is (`m_iType`), as TF2 numbers the modes of pipebombs
/// (`TF_GL_MODE_*`).
#[doc(alias("m_iType", "TF_GL_MODE"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PipebombKind {
	/// A Loose Cannon's cannonball.
	#[doc(alias("TF_GL_MODE_CANNONBALL"))]
	Cannonball,

	/// A grenade launcher's grenade, which explodes on hitting an enemy before
	/// it touches the world. Jars, balls, cleavers and spells, which derive
	/// from pipebombs, are of this mode too.
	#[doc(alias("TF_GL_MODE_REGULAR"))]
	Grenade,

	/// A Sticky Jumper's stickybomb, which does no damage.
	#[doc(alias("TF_GL_MODE_REMOTE_DETONATE_PRACTICE"))]
	PracticeStickybomb,

	/// A stickybomb launcher's stickybomb, which its Demoman detonates.
	#[doc(alias("TF_GL_MODE_REMOTE_DETONATE"))]
	Stickybomb,
}

impl PipebombKind {
	/// The kind TF2 numbers `raw`, or `None` if it numbers none so.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		Some(match raw {
			raw::TF_GL_MODE_CANNONBALL => Self::Cannonball,
			raw::TF_GL_MODE_REGULAR => Self::Grenade,
			raw::TF_GL_MODE_REMOTE_DETONATE_PRACTICE => Self::PracticeStickybomb,
			raw::TF_GL_MODE_REMOTE_DETONATE => Self::Stickybomb,
			_ => return None,
		})
	}

	/// TF2's number for the kind.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Cannonball => raw::TF_GL_MODE_CANNONBALL,
			Self::Grenade => raw::TF_GL_MODE_REGULAR,
			Self::PracticeStickybomb => raw::TF_GL_MODE_REMOTE_DETONATE_PRACTICE,
			Self::Stickybomb => raw::TF_GL_MODE_REMOTE_DETONATE,
		}
	}
}

/// A TF2 projectile, scoped to one engine callback.
///
/// What a projectile networks depends on its [`ProjectileFamily`] and kind.
/// The readers and writers of variables a projectile lacks fail with
/// [`ProjectileError::NetProp`].
#[doc(alias("CBaseProjectile"))]
#[derive(Debug, Clone, Copy)]
pub struct Projectile<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
	family: ProjectileFamily,
}

impl<'s> Projectile<'s> {
	/// Wraps a projectile, or fails with [`ProjectileError::WrongGame`] unless
	/// the server runs TF2, and [`ProjectileError::NotProjectile`] unless
	/// `entity`'s datamaps include the class of a [`ProjectileFamily`].
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ProjectileError> {
		if server.game() != Game::TeamFortress2 {
			return Err(ProjectileError::WrongGame);
		}

		let family = ProjectileFamily::ALL
			.into_iter()
			.find(|family| entity.has_data_map_class(family.data_map_class()))
			.ok_or(ProjectileError::NotProjectile)?;

		Ok(Self {
			server,
			entity,
			family,
		})
	}

	/// Every projectile of a `tf_projectile_*` class on the server, but not
	/// those already marked for deletion.
	pub fn all(server: Server<'s>) -> Result<impl Iterator<Item = Self>, ProjectileError> {
		if server.game() != Game::TeamFortress2 {
			return Err(ProjectileError::WrongGame);
		}

		let tools = server.server_tools()?;

		Ok(
			std::iter::successors(tools.find_by_class_name(None, CLASSES), move |&entity| {
				tools.find_by_class_name(Some(entity), CLASSES)
			})
			.filter(|entity| !entity.is_marked_for_deletion())
			.filter_map(move |entity| Self::new(server, entity).ok()),
		)
	}

	/// What an arrow is (`m_iProjectileType`), or `None` for a type that is
	/// not an arrow's. Only arrows network it: fails with
	/// [`ProjectileError::NetProp`] for the others.
	#[doc(alias("m_iProjectileType", "GetProjectileType"))]
	pub fn arrow_kind(self) -> Result<Option<ArrowKind>, ProjectileError> {
		Ok(ArrowKind::from_raw(self.get(c"m_iProjectileType")?))
	}

	/// The damage the projectile does where it hits (`GetDamage`), before
	/// falloff, critical hits and the target's resistances.
	///
	/// Most projectiles report what their weapon set, and Crusader's Crossbow
	/// bolts scale it with how long they flew. Some report a fixed damage:
	/// jars none, Flying Guillotine cleavers 50, energy rings 20 or 60, and the
	/// Grappling Hook 1.
	#[doc(alias("GetDamage", "m_flDamage"))]
	pub fn damage(self) -> f32 {
		let projectile = self.entity.as_ptr();

		// SAFETY: The projectile is a live entity during `'s`, on the main thread.
		// `GetDamage` reads a member, or computes the damage from the
		// projectile's state and attributes.
		unsafe { vcall!(projectile as sys::CBaseEntity__bindgen_vtable => CBaseEntity_GetDamage()) }
	}

	/// The player who last deflected a grenade (`m_hDeflectOwner`), or `None`.
	/// Stickybombs, which deflections push rather than redirect, forget it a few
	/// seconds later. Rockets, flares and arrows instead change owners as they
	/// are deflected. Only grenades network it: fails with
	/// [`ProjectileError::NetProp`] for the others.
	#[doc(alias("m_hDeflectOwner", "GetDeflectOwner"))]
	pub fn deflected_by(self) -> Result<Option<Entity<'s>>, ProjectileError> {
		self.handle(c"m_hDeflectOwner")
	}

	/// How often the projectile was deflected (`m_iDeflected`). Stickybombs reset
	/// it a few seconds after a deflection. Rockets, flares, arrows and grenades
	/// network it: fails with [`ProjectileError::NetProp`] for syringes and
	/// energy rings.
	#[doc(alias("m_iDeflected", "GetDeflected"))]
	pub fn deflections(self) -> Result<c_int, ProjectileError> {
		self.get(c"m_iDeflected")
	}

	/// The projectile's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// The C++ class the projectile's class derives from.
	pub const fn family(self) -> ProjectileFamily {
		self.family
	}

	/// Reads one of the projectile's networked variables.
	fn get<T: NetVar>(self, name: &CStr) -> Result<T, ProjectileError> {
		get(self.server, self.entity, name)
	}

	/// Resolves one of the projectile's networked entity handles.
	fn handle(self, name: &CStr) -> Result<Option<Entity<'s>>, ProjectileError> {
		handle(self.server, self.entity, name)
	}

	/// Whether a pipebomb has touched the world (`m_bTouched`): a grenade or
	/// cannonball that has no longer explodes on hitting an enemy, and a
	/// stickybomb that has stuck. Only pipebombs, and the jars, balls and
	/// spells that derive from them, network it: fails with
	/// [`ProjectileError::NetProp`] for the others.
	#[doc(alias("m_bTouched"))]
	pub fn has_touched(self) -> Result<bool, ProjectileError> {
		self.get(c"m_bTouched")
	}

	/// Whether an arrow is on fire (`m_bArrowAlight`), as from passing through
	/// fire, and ignites what it hits. Only arrows network it: fails with
	/// [`ProjectileError::NetProp`] for the others.
	#[doc(alias("m_bArrowAlight"))]
	pub fn is_alight(self) -> Result<bool, ProjectileError> {
		self.get(c"m_bArrowAlight")
	}

	/// Whether the projectile does critical damage (`m_bCritical`). Rockets,
	/// flares, arrows, fireballs and grenades network it: fails with
	/// [`ProjectileError::NetProp`] for Cow Mangler 5000 shots, syringes and
	/// energy rings.
	#[doc(alias("m_bCritical", "IsCritical"))]
	pub fn is_critical(self) -> Result<bool, ProjectileError> {
		self.get(c"m_bCritical")
	}

	/// The projectile's kind, from its entity class, or `None` for a class
	/// this crate does not know.
	pub fn kind(self) -> Option<ProjectileKind> {
		ProjectileKind::from_class_name(self.entity.class_name())
	}

	/// The weapon that last fired or deflected the projectile (`m_hLauncher`),
	/// or `None` for a sentry's rocket, or if it no longer exists. Rockets,
	/// flares, arrows, pipebombs and the jars, balls and spells that derive
	/// from them, syringes and energy rings network it: fails with
	/// [`ProjectileError::NetProp`] for the others.
	#[doc(alias("m_hLauncher", "GetLauncher"))]
	pub fn launcher(self) -> Result<Option<Entity<'s>>, ProjectileError> {
		self.handle(c"m_hLauncher")
	}

	/// Resolves one of the projectile's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, ProjectileError> {
		net_prop(self.server, self.entity, name)
	}

	/// The weapon that first fired the projectile (`m_hOriginalLauncher`),
	/// which deflections keep, or `None` if it no longer exists.
	#[doc(alias("m_hOriginalLauncher", "GetOriginalLauncher"))]
	pub fn original_launcher(self) -> Result<Option<Entity<'s>>, ProjectileError> {
		self.handle(c"m_hOriginalLauncher")
	}

	/// The entity that owns the projectile (`m_hOwnerEntity`), as the module
	/// documentation describes, or `None`.
	#[doc(alias("m_hOwnerEntity", "GetOwnerEntity"))]
	pub fn owner(self) -> Result<Option<Entity<'s>>, ProjectileError> {
		self.handle(c"m_hOwnerEntity")
	}

	/// What a pipebomb is (`m_iType`), or `None` for a mode TF2 does not
	/// number. Only pipebombs, and the jars, balls and spells that derive from
	/// them, network it: fails with [`ProjectileError::NetProp`] for the
	/// others.
	#[doc(alias("m_iType"))]
	pub fn pipebomb_kind(self) -> Result<Option<PipebombKind>, ProjectileError> {
		Ok(PipebombKind::from_raw(self.get(c"m_iType")?))
	}

	/// Makes the projectile do critical damage, or not (`m_bCritical`), as a
	/// critical shot or a critical boosted deflection does. Fails as
	/// [`Self::is_critical`] does.
	#[doc(alias("SetCritical"))]
	pub fn set_critical(self, critical: bool) -> Result<(), ProjectileError> {
		let prop = self.net_prop(c"m_bCritical")?;
		let engine = self.server.valve_engine()?;

		// SAFETY: The game sets the variable to either value itself.
		unsafe { prop.set(engine, self.entity, critical) }?;
		Ok(())
	}

	/// Sets the damage the projectile does (`SetDamage`), as its weapon does
	/// as it fires it. Fails with [`ProjectileError::InvalidDamage`], without
	/// setting it, unless `damage` is finite and not negative.
	///
	/// The projectiles that report a fixed [`damage`](Self::damage) ignore it.
	#[doc(alias("SetDamage", "m_flDamage"))]
	pub fn set_damage(self, damage: f32) -> Result<(), ProjectileError> {
		if !damage.is_finite() || damage < 0.0 {
			return Err(ProjectileError::InvalidDamage(damage));
		}

		let projectile = self.entity.as_ptr();

		// SAFETY: As for `damage`. `SetDamage` assigns a member, as the weapon
		// firing the projectile does, which the projectile reads as it hits.
		unsafe {
			vcall!(projectile as sys::CBaseEntity__bindgen_vtable => CBaseEntity_SetDamage(damage))
		};

		Ok(())
	}

	/// The player who threw a grenade (`m_hThrower`), or who last deflected
	/// it, or `None` if they no longer exist. Only grenades network it: fails
	/// with [`ProjectileError::NetProp`] for the others, whose
	/// [`owner`](Self::owner) fired them.
	#[doc(alias("m_hThrower", "GetThrower"))]
	pub fn thrower(self) -> Result<Option<Entity<'s>>, ProjectileError> {
		self.handle(c"m_hThrower")
	}
}

/// Why a projectile operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ProjectileError {
	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The damage is not finite, or negative.
	#[error("{0} is not a damage a projectile can do")]
	InvalidDamage(f32),

	/// A networked variable could not be read or written, as one the
	/// projectile does not have.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// The entity is not a TF2 flame manager.
	#[error("the entity is not a TF2 flame manager")]
	NotFlameManager,

	/// The entity is not a TF2 projectile.
	#[error("the entity is not a TF2 projectile")]
	NotProjectile,

	/// The server does not run Team Fortress 2.
	#[error("projectile operations require Team Fortress 2")]
	WrongGame,
}

/// The C++ classes TF2's projectiles derive from, which decide what they
/// network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProjectileFamily {
	/// A grenade (`CTFWeaponBaseGrenadeProj`): pipebombs, stickybombs,
	/// cannonballs, jars, cleavers, balls, throwables and most spells. Grenades
	/// network their thrower and damage.
	#[doc(alias("CTFWeaponBaseGrenadeProj", "CBaseGrenade"))]
	Grenade,

	/// A projectile of `CTFBaseProjectile`, the class of syringes, which TF2
	/// fires as nails, and of energy rings. They network only their launchers.
	#[doc(alias("CTFBaseProjectile"))]
	Nail,

	/// A rocket (`CTFBaseRocket`): rockets, sentries' rockets, flares, arrows,
	/// Cow Mangler 5000 shots, fireballs and orbs.
	#[doc(alias("CTFBaseRocket"))]
	Rocket,
}

impl ProjectileFamily {
	/// Every family.
	pub const ALL: [Self; 3] = [Self::Grenade, Self::Nail, Self::Rocket];

	/// The name of the family's class in its projectiles' datamaps.
	pub const fn data_map_class(self) -> &'static CStr {
		match self {
			Self::Grenade => c"CTFWeaponBaseGrenadeProj",
			Self::Nail => c"CTFBaseProjectile",
			Self::Rocket => c"CTFBaseRocket",
		}
	}
}

/// The kinds of TF2's projectiles, by their entity classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProjectileKind {
	/// An arrow of the Huntsman or the Fortified Compound, or a bolt of the
	/// Rescue Ranger, which [`Projectile::arrow_kind`] tells apart.
	#[doc(alias("tf_projectile_arrow", "CTFProjectile_Arrow"))]
	Arrow,

	/// A fireball of the Dragon's Fury.
	#[doc(alias("tf_projectile_balloffire", "CTFProjectile_BallOfFire"))]
	BallOfFire,

	/// A cleaver of the Flying Guillotine.
	#[doc(alias("tf_projectile_cleaver", "CTFProjectile_Cleaver"))]
	Cleaver,

	/// A shot of the Cow Mangler 5000.
	#[doc(alias("tf_projectile_energy_ball", "CTFProjectile_EnergyBall"))]
	EnergyBall,

	/// A shot of the Righteous Bison or the Pomson 6000.
	#[doc(alias("tf_projectile_energy_ring", "CTFProjectile_EnergyRing"))]
	EnergyRing,

	/// A flare of a flare gun.
	#[doc(alias("tf_projectile_flare", "CTFProjectile_Flare"))]
	Flare,

	/// The hook of the Grappling Hook.
	#[doc(alias("tf_projectile_grapplinghook", "CTFProjectile_GrapplingHook"))]
	GrapplingHook,

	/// A bolt of the Crusader's Crossbow.
	#[doc(alias("tf_projectile_healing_bolt", "CTFProjectile_HealingBolt"))]
	HealingBolt,

	/// A jar of Jarate.
	#[doc(alias("tf_projectile_jar", "CTFProjectile_Jar"))]
	Jar,

	/// A jar of the Gas Passer.
	#[doc(alias("tf_projectile_jar_gas", "CTFProjectile_JarGas"))]
	JarGas,

	/// A jar of Mad Milk.
	#[doc(alias("tf_projectile_jar_milk", "CTFProjectile_JarMilk"))]
	JarMilk,

	/// An orb of the Short Circuit.
	#[doc(alias("tf_projectile_mechanicalarmorb", "CTFProjectile_MechanicalArmOrb"))]
	MechanicalArmOrb,

	/// An ornament of the Wrap Assassin.
	#[doc(alias("tf_projectile_ball_ornament", "CTFBall_Ornament"))]
	Ornament,

	/// A grenade of a grenade launcher, or a cannonball of the Loose Cannon.
	#[doc(alias("tf_projectile_pipe", "CTFGrenadePipebombProjectile"))]
	Pipe,

	/// A stickybomb of a stickybomb launcher.
	#[doc(alias("tf_projectile_pipe_remote", "CTFGrenadePipebombProjectile"))]
	PipeRemote,

	/// A rocket of a rocket launcher.
	#[doc(alias("tf_projectile_rocket", "CTFProjectile_Rocket"))]
	Rocket,

	/// A rocket of a level 3 sentry.
	#[doc(alias("tf_projectile_sentryrocket", "CTFProjectile_SentryRocket"))]
	SentryRocket,

	/// The bats spell.
	#[doc(alias("tf_projectile_spellbats", "CTFProjectile_SpellBats"))]
	SpellBats,

	/// The fireball spell.
	#[doc(alias("tf_projectile_spellfireball", "CTFProjectile_SpellFireball"))]
	SpellFireball,

	/// The bats spell of bumper cars.
	#[doc(alias("tf_projectile_spellkartbats", "CTFProjectile_SpellKartBats"))]
	SpellKartBats,

	/// The orb spell of bumper cars.
	#[doc(alias("tf_projectile_spellkartorb", "CTFProjectile_SpellKartOrb"))]
	SpellKartOrb,

	/// The lightning orb spell.
	#[doc(alias("tf_projectile_lightningorb", "CTFProjectile_SpellLightningOrb"))]
	SpellLightningOrb,

	/// The meteor shower spell.
	#[doc(alias("tf_projectile_spellmeteorshower", "CTFProjectile_SpellMeteorShower"))]
	SpellMeteorShower,

	/// The pumpkin MIRV spell.
	#[doc(alias("tf_projectile_spellmirv", "CTFProjectile_SpellMirv"))]
	SpellMirv,

	/// A pumpkin bomb of the pumpkin MIRV spell.
	#[doc(alias("tf_projectile_spellpumpkin", "CTFProjectile_SpellPumpkin"))]
	SpellPumpkin,

	/// The spell that summons MONOCULUS.
	#[doc(alias("tf_projectile_spellspawnboss", "CTFProjectile_SpellSpawnBoss"))]
	SpellSpawnBoss,

	/// The spell that summons a horde of skeletons.
	#[doc(alias("tf_projectile_spellspawnhorde", "CTFProjectile_SpellSpawnHorde"))]
	SpellSpawnHorde,

	/// The spell that summons a skeleton.
	#[doc(alias("tf_projectile_spellspawnzombie", "CTFProjectile_SpellSpawnZombie"))]
	SpellSpawnZombie,

	/// The teleport spell.
	#[doc(alias(
		"tf_projectile_spelltransposeteleport",
		"CTFProjectile_SpellTransposeTeleport"
	))]
	SpellTransposeTeleport,

	/// A ball of the Sandman.
	#[doc(alias("tf_projectile_stun_ball", "CTFStunBall"))]
	StunBall,

	/// A syringe of a syringe gun.
	#[doc(alias("tf_projectile_syringe", "CTFProjectile_Syringe"))]
	Syringe,

	/// An unused throwable.
	#[doc(alias("tf_projectile_throwable", "CTFProjectile_Throwable"))]
	Throwable,

	/// An unused throwable bread monster.
	#[doc(alias(
		"tf_projectile_throwable_breadmonster",
		"CTFProjectile_ThrowableBreadMonster"
	))]
	ThrowableBreadMonster,

	/// An unused throwable brick.
	#[doc(alias("tf_projectile_throwable_brick", "CTFProjectile_ThrowableBrick"))]
	ThrowableBrick,

	/// An unused throwable that repels.
	#[doc(alias("tf_projectile_throwable_repel", "CTFProjectile_ThrowableRepel"))]
	ThrowableRepel,
}

impl ProjectileKind {
	/// Every kind, in the order of their class names.
	pub const ALL: [Self; 35] = [
		Self::Arrow,
		Self::Ornament,
		Self::BallOfFire,
		Self::Cleaver,
		Self::EnergyBall,
		Self::EnergyRing,
		Self::Flare,
		Self::GrapplingHook,
		Self::HealingBolt,
		Self::Jar,
		Self::JarGas,
		Self::JarMilk,
		Self::SpellLightningOrb,
		Self::MechanicalArmOrb,
		Self::Pipe,
		Self::PipeRemote,
		Self::Rocket,
		Self::SentryRocket,
		Self::SpellBats,
		Self::SpellFireball,
		Self::SpellKartBats,
		Self::SpellKartOrb,
		Self::SpellMeteorShower,
		Self::SpellMirv,
		Self::SpellPumpkin,
		Self::SpellSpawnBoss,
		Self::SpellSpawnHorde,
		Self::SpellSpawnZombie,
		Self::SpellTransposeTeleport,
		Self::StunBall,
		Self::Syringe,
		Self::Throwable,
		Self::ThrowableBreadMonster,
		Self::ThrowableBrick,
		Self::ThrowableRepel,
	];

	/// The kind whose entity class is named `class_name`, or `None` for a
	/// class this crate does not know.
	pub fn from_class_name(class_name: &CStr) -> Option<Self> {
		Self::ALL
			.into_iter()
			.find(|kind| kind.class_name() == class_name)
	}

	/// The kind's entity class name, such as `tf_projectile_rocket`.
	pub const fn class_name(self) -> &'static CStr {
		match self {
			Self::Arrow => c"tf_projectile_arrow",
			Self::BallOfFire => c"tf_projectile_balloffire",
			Self::Cleaver => c"tf_projectile_cleaver",
			Self::EnergyBall => c"tf_projectile_energy_ball",
			Self::EnergyRing => c"tf_projectile_energy_ring",
			Self::Flare => c"tf_projectile_flare",
			Self::GrapplingHook => c"tf_projectile_grapplinghook",
			Self::HealingBolt => c"tf_projectile_healing_bolt",
			Self::Jar => c"tf_projectile_jar",
			Self::JarGas => c"tf_projectile_jar_gas",
			Self::JarMilk => c"tf_projectile_jar_milk",
			Self::MechanicalArmOrb => c"tf_projectile_mechanicalarmorb",
			Self::Ornament => c"tf_projectile_ball_ornament",
			Self::Pipe => c"tf_projectile_pipe",
			Self::PipeRemote => c"tf_projectile_pipe_remote",
			Self::Rocket => c"tf_projectile_rocket",
			Self::SentryRocket => c"tf_projectile_sentryrocket",
			Self::SpellBats => c"tf_projectile_spellbats",
			Self::SpellFireball => c"tf_projectile_spellfireball",
			Self::SpellKartBats => c"tf_projectile_spellkartbats",
			Self::SpellKartOrb => c"tf_projectile_spellkartorb",
			Self::SpellLightningOrb => c"tf_projectile_lightningorb",
			Self::SpellMeteorShower => c"tf_projectile_spellmeteorshower",
			Self::SpellMirv => c"tf_projectile_spellmirv",
			Self::SpellPumpkin => c"tf_projectile_spellpumpkin",
			Self::SpellSpawnBoss => c"tf_projectile_spellspawnboss",
			Self::SpellSpawnHorde => c"tf_projectile_spellspawnhorde",
			Self::SpellSpawnZombie => c"tf_projectile_spellspawnzombie",
			Self::SpellTransposeTeleport => c"tf_projectile_spelltransposeteleport",
			Self::StunBall => c"tf_projectile_stun_ball",
			Self::Syringe => c"tf_projectile_syringe",
			Self::Throwable => c"tf_projectile_throwable",
			Self::ThrowableBreadMonster => c"tf_projectile_throwable_breadmonster",
			Self::ThrowableBrick => c"tf_projectile_throwable_brick",
			Self::ThrowableRepel => c"tf_projectile_throwable_repel",
		}
	}

	/// The family of the kind's class.
	pub const fn family(self) -> ProjectileFamily {
		match self {
			Self::Arrow
			| Self::BallOfFire
			| Self::EnergyBall
			| Self::Flare
			| Self::GrapplingHook
			| Self::HealingBolt
			| Self::MechanicalArmOrb
			| Self::Rocket
			| Self::SentryRocket
			| Self::SpellFireball
			| Self::SpellKartOrb
			| Self::SpellLightningOrb => ProjectileFamily::Rocket,

			Self::EnergyRing | Self::Syringe => ProjectileFamily::Nail,

			Self::Cleaver
			| Self::Jar
			| Self::JarGas
			| Self::JarMilk
			| Self::Ornament
			| Self::Pipe
			| Self::PipeRemote
			| Self::SpellBats
			| Self::SpellKartBats
			| Self::SpellMeteorShower
			| Self::SpellMirv
			| Self::SpellPumpkin
			| Self::SpellSpawnBoss
			| Self::SpellSpawnHorde
			| Self::SpellSpawnZombie
			| Self::SpellTransposeTeleport
			| Self::StunBall
			| Self::Throwable
			| Self::ThrowableBreadMonster
			| Self::ThrowableBrick
			| Self::ThrowableRepel => ProjectileFamily::Grenade,
		}
	}
}

/// Reads one of an entity's networked variables.
fn get<T: NetVar>(
	server: Server<'_>,
	entity: Entity<'_>,
	name: &CStr,
) -> Result<T, ProjectileError> {
	Ok(net_prop(server, entity, name)?.get(entity)?)
}

/// Resolves one of an entity's networked entity handles.
fn handle<'s>(
	server: Server<'s>,
	entity: Entity<'s>,
	name: &CStr,
) -> Result<Option<Entity<'s>>, ProjectileError> {
	let handle = net_prop(server, entity, name)?.get_handle(entity)?;

	Ok(server.server_tools()?.entity_by_handle(handle))
}

/// Resolves one of an entity's networked variables.
fn net_prop<'s>(
	server: Server<'s>,
	entity: Entity<'s>,
	name: &CStr,
) -> Result<NetProp<'s>, ProjectileError> {
	Ok(server.server_game_dll()?.entity_net_prop(entity, name)?)
}
