//! TF2's bosses: the Halloween bosses, skeletons and their spawners,
//! `base_boss`, and the boss health bar.
//!
//! # Spawning
//!
//! [`spawn_halloween_boss`] and [`spawn_skeleton`] spawn a boss or skeleton
//! as the game's own `CHalloweenBaseBoss::SpawnBossAtPos` and
//! `CZombie::SpawnAtPos` do (`game/server/tf/halloween`): they create the
//! entity, put it on its team, including the Halloween team the game's own
//! bosses fight on, and spawn it. Each boss computes its health in its
//! `Spawn`, from the players on RED and BLU and its convars, so a health
//! given to [`spawn_halloween_boss`] is set afterwards.
//!
//! [`SkeletonSpawner`] places a `tf_zombie_spawner`, the map entity that
//! spawns skeletons of any type, with a lifetime, while it is enabled.
//! [`BaseBoss`] spawns and controls a `base_boss`, the NextBot actor that
//! Mann vs. Machine's tank derives from. The spawned entities
//! are NextBot actors, which [`NextBot`](crate::tf2::bots::NextBot) wraps.
//!
//! The bosses and skeletons find their way through the level's navigation
//! mesh, and stand still on a level without one.
//!
//! # Boss health bar
//!
//! [`BossBar`] reads and sets the health bar the HUD shows at the top of the
//! screen, which Monoculus and Merasmus fill while they live. It belongs to
//! no boss: the game shows whatever was last set, and hides it at the start
//! of each round.
//!
//! # Unverified
//!
//! The wrappers follow Valve's Source SDK 2013 and have not been tested on a
//! live server.

#[cfg(test)]
#[path = "../tests/tf2/bosses.rs"]
mod tests;

use crate::datatables::NetPropError;
use crate::entities::Entity;
use crate::entities::health::HealthError;
use crate::inputs::{InputError, InputValue};
use crate::interfaces::ServerTools;
use crate::math::Vector;
use crate::tf2::script_binding::{self as binding, BindingError, VOID};
use crate::tf2::teams::Team;
use crate::{Game, InterfaceError, Server};
use sdk_raw::players::TEAM_UNASSIGNED;
use sdk_raw::tf2::bosses::{self as raw, skeleton};
use sdk_raw::tf2::scoreboard::{TF_TEAM_BLUE, TF_TEAM_RED};
use sdk_raw::tf2::script_binding::boolean;
use sdk_raw::vcall;
use std::ffi::{CStr, CString, c_int};
use std::fmt::Display;

/// The most a boss bar shows: the full bar, as a byte of the monster
/// resource.
const FULL_BAR: c_int = 255;

/// Why a boss could not be spawned, wrapped or controlled.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BossError {
	/// The server is not running Team Fortress 2.
	#[error("bosses require Team Fortress 2")]
	UnsupportedGame,

	/// The entity is not of the class the wrapper takes.
	#[error("the entity is not a {class:?}")]
	WrongClass {
		/// The data map class the wrapper takes.
		class: &'static CStr,
	},

	/// The entity is marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// A position, speed, height, lifetime or fraction given is NaN or
	/// infinite.
	#[error("a value given is NaN or infinite")]
	NonFinite,

	/// A health given is 0 or less, which game code divides by.
	#[error("health {0} is not positive")]
	NonPositiveHealth(c_int),

	/// A `base_boss` model given is not precached. Precaching it while the
	/// level runs could overflow the engine's model table, so it must be
	/// precached already, as the level's own models are.
	#[error("the model is not precached")]
	ModelNotPrecached,

	/// The game created no entity of the class.
	#[error("the game could not create a {class:?}")]
	NotCreated {
		/// The class name.
		class: &'static CStr,
	},

	/// The new entity refused a key value. It was removed.
	#[error("the entity refused the key value {key:?}")]
	KeyValueRejected {
		/// The key.
		key: &'static CStr,
	},

	/// The entity marked itself for deletion as it spawned.
	#[error("the entity removed itself as it spawned")]
	SpawnFailed,

	/// The script class descriptors lack a native method, or its signature
	/// differs from the SDK's.
	#[error("the game does not expose the expected native method")]
	UnsupportedMethod,

	/// A native method's binding adapter reported failure.
	#[error("the native method rejected the call")]
	Rejected,

	/// The new boss's health could not be set.
	#[error(transparent)]
	Health(#[from] HealthError),

	/// An input was not sent, or the entity rejected it.
	#[error(transparent)]
	Input(#[from] InputError),

	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// A networked variable could not be read or written.
	#[error(transparent)]
	NetProp(#[from] NetPropError),
}

impl From<BindingError> for BossError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// A `base_boss` (`CTFBaseBoss`) within the current engine callback, or one
/// of the classes deriving from it, such as Mann vs. Machine's tank
/// (`tank_boss`).
///
/// A `base_boss` is a NextBot actor with a model, health and speed, and no
/// behaviour of its own: it moves only as its locomotion is told to, which
/// the tank's own behaviour does along its path. While enabled, it pushes
/// living players out of its way, unless told not to. Killed, it fires
/// `OnKilled` and drops money packs. Send it the health inputs through
/// [`ServerTools::send_health_input`](crate::interfaces::ServerTools::send_health_input).
#[doc(alias("base_boss", "CTFBaseBoss"))]
#[derive(Debug, Clone, Copy)]
pub struct BaseBoss<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
}

impl<'s> BaseBoss<'s> {
	/// Wraps `entity`. Fails with [`BossError::UnsupportedGame`] outside TF2,
	/// and with [`BossError::WrongClass`] unless its data maps include
	/// `CTFBaseBoss`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, BossError> {
		check_class(server, entity, raw::BASE_BOSS_CLASS)?;

		Ok(Self { server, entity })
	}

	/// Spawns a `base_boss` as `spawn` describes.
	///
	/// Fails, before creating it, with [`BossError::NonFinite`] for a position
	/// or speed that is not finite, [`BossError::NonPositiveHealth`] for a
	/// health of 0 or less, and [`BossError::ModelNotPrecached`] for a model
	/// that is not precached.
	pub fn spawn(server: Server<'s>, spawn: &BaseBossSpawn<'_>) -> Result<Self, BossError> {
		check_game(server)?;

		if !spawn.origin.is_finite() || !spawn.speed.is_none_or(f32::is_finite) {
			return Err(BossError::NonFinite);
		}

		if spawn.health <= 0 {
			return Err(BossError::NonPositiveHealth(spawn.health));
		}

		if server.model_info()?.model_index(spawn.model).is_none() {
			return Err(BossError::ModelNotPrecached);
		}

		let tools = server.server_tools()?;

		// SAFETY: `CTFBaseBoss`'s constructor only sets its members and makes
		// its NextBot components (`game/server/tf/player_vs_environment`).
		let entity = unsafe { create(tools, raw::BASE_BOSS) }?;
		let health = number_value(spawn.health);
		let team = number_value(spawn.team.to_raw());

		let mut keys = vec![
			(c"origin", vector_value(spawn.origin)),
			(c"model", spawn.model.to_owned()),
			(c"health", health),
			(c"teamnumber", team),
			(c"start_disabled", flag_value(spawn.start_disabled)),
		];

		if let Some(speed) = spawn.speed {
			keys.push((c"speed", number_value(speed)));
		}

		for (key, value) in &keys {
			set_key(tools, entity, key, value)?;
		}

		// SAFETY: Its `Spawn` precaches its model, which the check above found
		// precached, sets its model and health, adds it to the game rules'
		// bosses, and starts it thinking.
		unsafe { spawn_entity(tools, entity) }?;

		Ok(Self { server, entity })
	}

	/// Stops the boss: it neither updates nor pushes players (`Disable`).
	#[doc(alias("Disable"))]
	pub fn disable(self) -> Result<(), BossError> {
		self.input(c"Disable", InputValue::Void)
	}

	/// Lets the boss update and push players again (`Enable`).
	#[doc(alias("Enable"))]
	pub fn enable(self) -> Result<(), BossError> {
		self.input(c"Enable", InputValue::Void)
	}

	/// The boss's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Sends the boss an input, with itself as the activator and caller.
	fn input(self, name: &CStr, value: InputValue<'_>) -> Result<(), BossError> {
		let tools = self.server.server_tools()?;

		Ok(tools.accept_input(self.entity, name, value, self.entity, self.entity)?)
	}

	/// Sets the highest ledge the boss climbs, in units (`SetMaxJumpHeight`).
	#[doc(alias("SetMaxJumpHeight"))]
	pub fn set_max_jump_height(self, height: f32) -> Result<(), BossError> {
		self.input(c"SetMaxJumpHeight", InputValue::Float(height))
	}

	/// Sets whether the boss pushes players out of its way as it moves,
	/// which it does unless told otherwise (`SetResolvePlayerCollisions`).
	#[doc(alias("SetResolvePlayerCollisions"))]
	pub fn set_resolve_player_collisions(self, resolve: bool) -> Result<(), BossError> {
		check_live(self.entity)?;

		// SAFETY: The method only stores the flag
		// (`game/server/tf/player_vs_environment/tf_base_boss.h`).
		unsafe {
			binding::call(
				self.entity,
				raw::BASE_BOSS_CLASS,
				c"SetResolvePlayerCollisions",
				&mut [boolean(resolve)],
				VOID,
			)
		}?;

		Ok(())
	}

	/// Sets the speed the boss's locomotion runs at, in units per second
	/// (`SetSpeed`).
	#[doc(alias("SetSpeed"))]
	pub fn set_speed(self, speed: f32) -> Result<(), BossError> {
		self.input(c"SetSpeed", InputValue::Float(speed))
	}

	/// Sets the highest step the boss walks up, in units (`SetStepHeight`).
	#[doc(alias("SetStepHeight"))]
	pub fn set_step_height(self, height: f32) -> Result<(), BossError> {
		self.input(c"SetStepHeight", InputValue::Float(height))
	}
}

/// What [`BaseBoss::spawn`] spawns: a `base_boss`'s key values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BaseBossSpawn<'a> {
	/// Where it spawns.
	pub origin: Vector,

	/// Its model, such as `models/bots/boss_bot/boss_tank.mdl`, which must
	/// be precached already (`model`).
	pub model: &'a CStr,

	/// Its health and maximum health (`health`).
	pub health: c_int,

	/// The speed its locomotion runs at, in units per second, or `None` for
	/// `tf_base_boss_speed` (`speed`).
	pub speed: Option<f32>,

	/// Its team (`teamnumber`).
	pub team: Team,

	/// Whether it starts disabled, until [`BaseBoss::enable`]
	/// (`start_disabled`).
	pub start_disabled: bool,
}

impl<'a> BaseBossSpawn<'a> {
	/// A `base_boss` with `model` and `health` at `origin`, on no team, at the
	/// game's default speed, and enabled.
	pub const fn new(origin: Vector, model: &'a CStr, health: c_int) -> Self {
		Self {
			origin,
			model,
			health,
			speed: None,
			team: Team::Unassigned,
			start_disabled: false,
		}
	}
}

/// The boss health bar (`monster_resource`, `CMonsterResource`): the bar
/// the HUD shows at the top of the screen while it is more than empty, with
/// a stun meter below it.
///
/// It holds its fractions as bytes, so the fractions read back are
/// multiples of 1/255, rounded down. Monoculus and Merasmus set it each
/// update while they live, and the game rules hide it at the start of each
/// round.
#[doc(alias("monster_resource", "CMonsterResource"))]
#[derive(Debug, Clone, Copy)]
pub struct BossBar<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
}

impl<'s> BossBar<'s> {
	/// Wraps `entity`. Fails with [`BossError::UnsupportedGame`] outside TF2,
	/// and with [`BossError::WrongClass`] unless its data maps include
	/// `CMonsterResource`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, BossError> {
		check_class(server, entity, raw::MONSTER_RESOURCE_CLASS)?;

		Ok(Self { server, entity })
	}

	/// Finds the monster resource, which TF2's game rules create with each
	/// level, or `None` if there is none.
	pub fn find(server: Server<'s>) -> Result<Option<Self>, BossError> {
		check_game(server)?;

		let tools = server.server_tools()?;
		let mut found = tools.find_by_class_name(None, raw::MONSTER_RESOURCE);

		while let Some(entity) = found {
			if let Ok(bar) = Self::new(server, entity) {
				return Ok(Some(bar));
			}

			found = tools.find_by_class_name(Some(entity), raw::MONSTER_RESOURCE);
		}

		Ok(None)
	}

	/// The monster resource's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// How full the bar is, from 0, which hides it, to 1
	/// (`m_iBossHealthPercentageByte`).
	#[doc(alias("m_iBossHealthPercentageByte", "GetBossHealthPercentage"))]
	pub fn health(self) -> Result<f32, BossError> {
		self.fraction(c"m_iBossHealthPercentageByte")
	}

	/// Empties the bar and the stun meter, which hides them, as the game's
	/// `HideBossHealthMeter` does.
	#[doc(alias("HideBossHealthMeter"))]
	pub fn hide(self) -> Result<(), BossError> {
		self.set_fraction(c"m_iBossHealthPercentageByte", 0.0)?;
		self.set_fraction(c"m_iBossStunPercentageByte", 0.0)
	}

	/// Whether the bar shows in the HUD's inactive colour, as while Merasmus
	/// hides among props, rather than its boss colour (`m_iBossState`).
	#[doc(alias("m_iBossState"))]
	pub fn is_inactive(self) -> Result<bool, BossError> {
		Ok(self.prop(c"m_iBossState")? != 0)
	}

	/// Whether the HUD shows the bar: whether it is more than empty.
	pub fn is_shown(self) -> Result<bool, BossError> {
		Ok(self.prop(c"m_iBossHealthPercentageByte")? > 0)
	}

	/// Sets the bar to `fraction` of its length, which shows it unless it
	/// rounds down to 0, as the game's `SetBossHealthPercentage` does.
	/// Fractions beyond 0 to 1 are clamped.
	///
	/// Fails with [`BossError::NonFinite`] for NaN or an infinite fraction.
	#[doc(alias("SetBossHealthPercentage", "m_iBossHealthPercentageByte"))]
	pub fn set_health(self, fraction: f32) -> Result<(), BossError> {
		self.set_fraction(c"m_iBossHealthPercentageByte", fraction)
	}

	/// Sets whether the bar shows in the HUD's inactive colour, as
	/// [`is_inactive`](Self::is_inactive) describes.
	#[doc(alias("SetBossState", "m_iBossState"))]
	pub fn set_inactive(self, inactive: bool) -> Result<(), BossError> {
		self.set_prop(c"m_iBossState", c_int::from(inactive))
	}

	/// Sets the stun meter to `fraction` of its length, as the game's
	/// `SetBossStunPercentage` does: clients show it while the bar shows,
	/// unless they set `cl_boss_show_stun` to 0. Fractions beyond 0 to 1 are
	/// clamped.
	///
	/// Fails with [`BossError::NonFinite`] for NaN or an infinite fraction.
	#[doc(alias("SetBossStunPercentage", "m_iBossStunPercentageByte"))]
	pub fn set_stun(self, fraction: f32) -> Result<(), BossError> {
		self.set_fraction(c"m_iBossStunPercentageByte", fraction)
	}

	/// How full the stun meter is, from 0, which hides it, to 1
	/// (`m_iBossStunPercentageByte`).
	#[doc(alias("m_iBossStunPercentageByte", "GetBossStunPercentage"))]
	pub fn stun(self) -> Result<f32, BossError> {
		self.fraction(c"m_iBossStunPercentageByte")
	}

	/// Reads a byte variable as a fraction of [`FULL_BAR`].
	fn fraction(self, name: &CStr) -> Result<f32, BossError> {
		Ok(self.prop(name)?.clamp(0, FULL_BAR) as f32 / FULL_BAR as f32)
	}

	/// Reads the networked `int` `name`.
	fn prop(self, name: &CStr) -> Result<c_int, BossError> {
		check_live(self.entity)?;

		let prop = self
			.server
			.server_game_dll()?
			.entity_net_prop(self.entity, name)?;

		Ok(prop.get::<c_int>(self.entity)?)
	}

	/// Writes `fraction` of [`FULL_BAR`] to a byte variable, rounding down as
	/// the game does.
	fn set_fraction(self, name: &CStr, fraction: f32) -> Result<(), BossError> {
		if !fraction.is_finite() {
			return Err(BossError::NonFinite);
		}

		self.set_prop(name, (fraction.clamp(0.0, 1.0) * FULL_BAR as f32) as c_int)
	}

	/// Writes the networked `int` `name`.
	fn set_prop(self, name: &CStr, value: c_int) -> Result<(), BossError> {
		check_live(self.entity)?;

		let prop = self
			.server
			.server_game_dll()?
			.entity_net_prop(self.entity, name)?;

		// SAFETY: The callers write a byte of 0 to 255, or a state of 0 or 1,
		// as the game's own setters do; clients only draw them.
		Ok(unsafe { prop.set(self.server.valve_engine()?, self.entity, value) }?)
	}
}

/// The team [`spawn_halloween_boss`] and [`spawn_skeleton`] put a boss or
/// skeleton on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BossTeam {
	/// RED, as the Halloween spells summon them for RED's casters.
	#[doc(alias("TF_TEAM_RED"))]
	Red,

	/// BLU, as for RED.
	#[doc(alias("TF_TEAM_BLUE"))]
	Blue,

	/// The Halloween team, neither RED nor BLU, which the game spawns its own
	/// Halloween bosses and skeletons on (`TF_TEAM_HALLOWEEN`).
	#[doc(alias("TF_TEAM_HALLOWEEN"))]
	Halloween,
}

impl BossTeam {
	/// The team numbered `raw`, or `None` for any other number.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			TF_TEAM_RED => Some(Self::Red),
			TF_TEAM_BLUE => Some(Self::Blue),
			raw::TF_TEAM_HALLOWEEN => Some(Self::Halloween),
			_ => None,
		}
	}

	/// The team's number, as `m_iTeamNum` holds it.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Red => TF_TEAM_RED,
			Self::Blue => TF_TEAM_BLUE,
			Self::Halloween => raw::TF_TEAM_HALLOWEEN,
		}
	}
}

/// One of TF2's Halloween bosses, which [`spawn_halloween_boss`] spawns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HalloweenBoss {
	/// The Horseless Headless Horsemann (`headless_hatman`), who hunts one
	/// player at a time with his axe.
	#[doc(alias("headless_hatman", "CHeadlessHatman", "HHH"))]
	Horsemann,

	/// Monoculus (`eyeball_boss`), who fires rockets from the air and
	/// teleports between the `info_target`s named `spawn_boss_alt`, or stays
	/// where it is on a level without them.
	#[doc(alias("eyeball_boss", "CEyeballBoss"))]
	Monoculus,

	/// Merasmus (`merasmus`), who throws bombs and hides among props on the
	/// level's navigation mesh, and leaves after `tf_merasmus_lifetime`
	/// seconds.
	#[doc(alias("merasmus", "CMerasmus"))]
	Merasmus,
}

impl HalloweenBoss {
	/// Every Halloween boss.
	pub const ALL: [Self; 3] = [Self::Horsemann, Self::Monoculus, Self::Merasmus];

	/// The boss's class name.
	pub const fn class_name(self) -> &'static CStr {
		match self {
			Self::Horsemann => raw::HEADLESS_HATMAN,
			Self::Monoculus => raw::EYEBALL_BOSS,
			Self::Merasmus => raw::MERASMUS,
		}
	}
}

/// A `tf_zombie_spawner` (`CZombieSpawner`) within the current engine
/// callback: the map entity that spawns skeletons at its position while it
/// is enabled, one every 1.5 to 3 seconds.
///
/// With infinite skeletons, it keeps up to its maximum alive at a time;
/// otherwise it spawns up to its maximum in all, and spawns them again once
/// disabled and enabled. Its skeletons take its team if it is RED or BLU,
/// and the Halloween team otherwise.
#[doc(alias("tf_zombie_spawner", "CZombieSpawner"))]
#[derive(Debug, Clone, Copy)]
pub struct SkeletonSpawner<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
}

impl<'s> SkeletonSpawner<'s> {
	/// Wraps `entity`. Fails with [`BossError::UnsupportedGame`] outside TF2,
	/// and with [`BossError::WrongClass`] unless its data maps include
	/// `CZombieSpawner`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, BossError> {
		check_class(server, entity, raw::TF_ZOMBIE_SPAWNER_CLASS)?;

		Ok(Self { server, entity })
	}

	/// Spawns a disabled spawner at `origin`, as `spawn` describes. Enable
	/// it to spawn skeletons.
	///
	/// Fails, before creating it, with [`BossError::NonFinite`] for a position
	/// or lifetime that is not finite.
	pub fn spawn(
		server: Server<'s>,
		origin: Vector,
		spawn: &SkeletonSpawn,
	) -> Result<Self, BossError> {
		check_game(server)?;

		if !origin.is_finite() || !spawn.lifetime.is_none_or(f32::is_finite) {
			return Err(BossError::NonFinite);
		}

		let tools = server.server_tools()?;

		// SAFETY: `CZombieSpawner`'s constructor only sets its members
		// (`game/server/tf/halloween/zombie/zombie_spawner.cpp`).
		let entity = unsafe { create(tools, raw::TF_ZOMBIE_SPAWNER) }?;

		// The spawner gives its skeletons the Halloween team for any team
		// other than RED and BLU.
		let team = match spawn.team {
			BossTeam::Red => TF_TEAM_RED,
			BossTeam::Blue => TF_TEAM_BLUE,
			BossTeam::Halloween => TEAM_UNASSIGNED,
		};

		let keys = [
			(c"origin", vector_value(origin)),
			(c"teamnumber", number_value(team)),
			(c"zombie_type", number_value(spawn.kind.to_raw())),
			(c"max_zombies", number_value(spawn.count)),
			(c"infinite_zombies", flag_value(spawn.infinite)),
			(
				c"zombie_lifetime",
				number_value(spawn.lifetime.unwrap_or(0.0)),
			),
		];

		for (key, value) in &keys {
			set_key(tools, entity, key, value)?;
		}

		// SAFETY: Its `Spawn` only starts it thinking. It spawns skeletons from
		// its thinks, while enabled.
		unsafe { spawn_entity(tools, entity) }?;

		Ok(Self { server, entity })
	}

	/// Stops the spawner, and forgets the skeletons it spawned, which live on
	/// (`Disable`).
	#[doc(alias("Disable"))]
	pub fn disable(self) -> Result<(), BossError> {
		self.input(c"Disable", InputValue::Void)
	}

	/// Starts the spawner (`Enable`).
	#[doc(alias("Enable"))]
	pub fn enable(self) -> Result<(), BossError> {
		self.input(c"Enable", InputValue::Void)
	}

	/// The spawner's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Sends the spawner an input, with itself as the activator and caller.
	fn input(self, name: &CStr, value: InputValue<'_>) -> Result<(), BossError> {
		let tools = self.server.server_tools()?;

		Ok(tools.accept_input(self.entity, name, value, self.entity, self.entity)?)
	}

	/// Sets how many skeletons the spawner keeps alive, or spawns in all
	/// (`SetMaxActiveZombies`).
	#[doc(alias("SetMaxActiveZombies", "max_zombies"))]
	pub fn set_count(self, count: c_int) -> Result<(), BossError> {
		self.input(c"SetMaxActiveZombies", InputValue::Int(count))
	}
}

/// What a [`SkeletonSpawner`] spawns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkeletonSpawn {
	/// The skeletons' type (`zombie_type`).
	pub kind: SkeletonType,

	/// The skeletons' team.
	pub team: BossTeam,

	/// How many skeletons it keeps alive, or spawns in all, as
	/// [`infinite`](Self::infinite) decides (`max_zombies`).
	pub count: c_int,

	/// Whether it keeps spawning skeletons as they die, rather than stopping
	/// after [`count`](Self::count) (`infinite_zombies`).
	pub infinite: bool,

	/// How long each skeleton lives, in seconds, or `None` for no limit
	/// (`zombie_lifetime`).
	pub lifetime: Option<f32>,
}

impl SkeletonSpawn {
	/// One skeleton of `kind` on the Halloween team, with no lifetime.
	pub const fn new(kind: SkeletonType) -> Self {
		Self {
			kind,
			team: BossTeam::Halloween,
			count: 1,
			infinite: false,
			lifetime: None,
		}
	}
}

/// A skeleton's type (`CZombie::SkeletonType_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkeletonType {
	/// An ordinary skeleton, with 50 health (`SKELETON_NORMAL`).
	#[doc(alias("SKELETON_NORMAL"))]
	Normal,

	/// A skeleton king, twice the size, with 1,000 health and a crown, which
	/// the game never kills to keep skeletons under `tf_max_active_zombie`
	/// (`SKELETON_KING`).
	#[doc(alias("SKELETON_KING"))]
	King,

	/// A small skeleton (`SKELETON_MINI`).
	#[doc(alias("SKELETON_MINI"))]
	Mini,
}

impl SkeletonType {
	/// The type numbered `raw`, or `None` for any other number.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			skeleton::SKELETON_NORMAL => Some(Self::Normal),
			skeleton::SKELETON_KING => Some(Self::King),
			skeleton::SKELETON_MINI => Some(Self::Mini),
			_ => None,
		}
	}

	/// The type's number.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Normal => skeleton::SKELETON_NORMAL,
			Self::King => skeleton::SKELETON_KING,
			Self::Mini => skeleton::SKELETON_MINI,
		}
	}
}

/// Spawns a Halloween boss at `origin` on `team`, as the game's
/// `CHalloweenBaseBoss::SpawnBossAtPos` does, which spawns them 10 units
/// above the ground it picks.
///
/// The boss computes its health as it spawns, from the players on RED and
/// BLU and its `tf_halloween_bot_health_*`, `tf_eyeball_boss_health_*` or
/// `tf_merasmus_health_*` convars. `Some(health)` then sets its health and
/// maximum health to that.
///
/// A boss on RED or BLU counts as one a spell summoned: Monoculus then shows
/// no health bar. On a level the game did not precache the boss for, as it
/// does on the Halloween levels each boss belongs to, the boss precaches its
/// models and sounds as it spawns, which needs room in the engine's model
/// table, as
/// [`MODEL_PRECACHE`](crate::interfaces::network_string_tables::MODEL_PRECACHE)
/// describes.
///
/// Fails, before creating it, with [`BossError::NonFinite`] for a position
/// that is not finite, and [`BossError::NonPositiveHealth`] for a health of
/// 0 or less.
pub fn spawn_halloween_boss<'s>(
	server: Server<'s>,
	boss: HalloweenBoss,
	origin: Vector,
	team: BossTeam,
	health: Option<c_int>,
) -> Result<Entity<'s>, BossError> {
	check_game(server)?;

	if !origin.is_finite() {
		return Err(BossError::NonFinite);
	}

	if let Some(health) = health
		&& health <= 0
	{
		return Err(BossError::NonPositiveHealth(health));
	}

	let tools = server.server_tools()?;

	// SAFETY: The bosses' constructors only set their members, make their
	// NextBot components, and, for Merasmus, listen for game events
	// (`game/server/tf/halloween`).
	let entity = unsafe { create(tools, boss.class_name()) }?;

	set_key(tools, entity, c"origin", &vector_value(origin))?;

	// SAFETY: The game puts its own bosses on any of these teams before they
	// spawn, through the same method.
	unsafe { change_team(entity, team) };

	// SAFETY: Their `Spawn`s precache what they use, set their models and
	// health, create the Horsemann's axe as a prop, find the entities they
	// work with by name, add them to the game rules' bosses, and fire
	// `recalculate_truce` or `merasmus_summoned`, whose listeners `Server::new`
	// condition 4 covers.
	unsafe { spawn_entity(tools, entity) }?;

	if let Some(health) = health {
		entity.set_max_health(server, health)?;
		entity.set_health(server, health)?;
	}

	Ok(entity)
}

/// Spawns an ordinary skeleton at `origin` on `team`, as the game's
/// `CZombie::SpawnAtPos` does, with no lifetime. A [`SkeletonSpawner`]
/// spawns the other types, and skeletons with a lifetime.
///
/// Once more skeletons live than `tf_max_active_zombie` allows, the oldest,
/// other than skeleton kings, die on their next update. On a level without
/// skeletons of its own, the skeleton precaches its models as it spawns, as
/// [`spawn_halloween_boss`] describes for the bosses.
///
/// Fails, before creating it, with [`BossError::NonFinite`] for a position
/// that is not finite.
#[doc(alias("tf_zombie", "CZombie"))]
pub fn spawn_skeleton<'s>(
	server: Server<'s>,
	origin: Vector,
	team: BossTeam,
) -> Result<Entity<'s>, BossError> {
	check_game(server)?;

	if !origin.is_finite() {
		return Err(BossError::NonFinite);
	}

	let tools = server.server_tools()?;

	// SAFETY: `CZombie`'s constructor only sets its members and makes its
	// NextBot components (`game/server/tf/halloween/zombie/zombie.cpp`).
	let entity = unsafe { create(tools, raw::TF_ZOMBIE) }?;

	set_key(tools, entity, c"origin", &vector_value(origin))?;

	// SAFETY: As for the bosses: the game puts skeletons on any of these teams
	// before they spawn.
	unsafe { change_team(entity, team) };

	// SAFETY: Its `Spawn` precaches what it uses, sets its model, health and
	// skin, and marks the oldest skeletons beyond `tf_max_active_zombie` to
	// kill themselves on their next update.
	unsafe { spawn_entity(tools, entity) }?;

	Ok(entity)
}

/// Fails with [`BossError::UnsupportedGame`] outside TF2, and with
/// [`BossError::WrongClass`] unless `entity`'s data maps include `class`'s.
fn check_class(
	server: Server<'_>,
	entity: Entity<'_>,
	class: &'static CStr,
) -> Result<(), BossError> {
	check_game(server)?;

	if entity.has_data_map_class(class) {
		Ok(())
	} else {
		Err(BossError::WrongClass { class })
	}
}

/// Fails with [`BossError::UnsupportedGame`] outside TF2.
fn check_game(server: Server<'_>) -> Result<(), BossError> {
	if server.game() == Game::TeamFortress2 {
		Ok(())
	} else {
		Err(BossError::UnsupportedGame)
	}
}

/// Fails with [`BossError::MarkedForDeletion`] for an entity marked for
/// deletion.
fn check_live(entity: Entity<'_>) -> Result<(), BossError> {
	if entity.is_marked_for_deletion() {
		Err(BossError::MarkedForDeletion)
	} else {
		Ok(())
	}
}

/// Puts an entity that has not spawned on `team`, through its
/// `CBaseEntity::ChangeTeam`.
///
/// # Safety
///
/// The entity's class must accept the team, as the game's own spawning of it
/// shows.
unsafe fn change_team(entity: Entity<'_>, team: BossTeam) {
	let raw = entity.as_ptr();

	// SAFETY: The entity is live during the callback, on the main thread, and
	// its class belongs to TF2's game DLL, whose vtables have `ChangeTeam`
	// where the generated one does (`sdk_raw::tf2::teams`). The classes here
	// only store the team, and leave a nav area they are not in yet.
	unsafe {
		vcall!(raw as sys::CBaseEntity__bindgen_vtable => CBaseEntity_ChangeTeam(team.to_raw()))
	};
}

/// Creates an entity of `class`, without spawning it.
///
/// # Safety
///
/// As for [`ServerTools::create_entity_by_name`].
unsafe fn create<'s>(
	tools: ServerTools<'s>,
	class: &'static CStr,
) -> Result<Entity<'s>, BossError> {
	// SAFETY: As the caller promises.
	unsafe { tools.create_entity_by_name(class) }.ok_or(BossError::NotCreated { class })
}

/// `1` or `0`, as boolean key values are given.
fn flag_value(flag: bool) -> CString {
	CString::from(if flag { c"1" } else { c"0" })
}

/// Sets a key value of an entity that has not spawned, or removes it and
/// fails with [`BossError::KeyValueRejected`] if it is refused.
fn set_key(
	tools: ServerTools<'_>,
	entity: Entity<'_>,
	key: &'static CStr,
	value: &CStr,
) -> Result<(), BossError> {
	if tools.set_key_value(entity, key, value) {
		return Ok(());
	}

	// The entity is neither the world nor a player, so removal is allowed.
	let _ = tools.remove(entity);

	Err(BossError::KeyValueRejected { key })
}

/// Spawns an entity created with [`create`], and fails with
/// [`BossError::SpawnFailed`] if it marked itself for deletion as it
/// spawned.
///
/// # Safety
///
/// As for [`ServerTools::dispatch_spawn`].
unsafe fn spawn_entity(tools: ServerTools<'_>, entity: Entity<'_>) -> Result<(), BossError> {
	// SAFETY: As the caller promises.
	unsafe { tools.dispatch_spawn(entity) };

	if entity.is_marked_for_deletion() {
		Err(BossError::SpawnFailed)
	} else {
		Ok(())
	}
}

/// A number as a key value, in decimal.
fn number_value(number: impl Display) -> CString {
	CString::new(number.to_string()).expect("numbers contain no NUL")
}

/// A vector as a key value: its three components, in decimal.
fn vector_value(vector: Vector) -> CString {
	let value = format!("{} {} {}", vector.x, vector.y, vector.z);

	CString::new(value).expect("numbers contain no NUL")
}
