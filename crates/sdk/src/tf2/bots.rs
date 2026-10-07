//! TF2's bots (`CTFBot`), which `tf_bot_add` adds, and the queries they share
//! with TF2's other NextBot actors, such as `base_boss` and the Halloween
//! bosses.
//!
//! # Bots
//!
//! A bot is a player whose class is `CTFBot`: it has a player's server class
//! and data maps, and reports [`TF_BOT_TYPE`] from `GetBotType`, which
//! [`TfBot::new`] checks. [`TfBot`] calls the native methods of `CTFBot`'s
//! script class (`game/server/tf/bot/tf_bot.cpp:618-712`) through their typed
//! binding descriptors, without a script VM: attributes, weapon restrictions,
//! behaviour flags, tags, difficulty, missions, squads and buttons. The
//! methods that take or return an entity pass its [`ScriptInstance`], so they
//! need the game's script VM and make the instance if the entity has none.
//! Those that take or return nav areas, action points, a table of tags or the
//! bot's NextBot components are not wrapped.
//!
//! # Adding and kicking
//!
//! [`add_tf_bots`] queues `tf_bot_add` with the arguments of a
//! [`TfBotRequest`], which the server runs from its command buffer, normally
//! on the next frame. [`kick_tf_bots`] and [`TfBot::kick`] queue
//! `tf_bot_kick` and `kickid`, so the bots leave once the commands run.
//!
//! # NextBot actors
//!
//! [`NextBot`] wraps a bot or another entity deriving from
//! `NextBotCombatCharacter`, whose script class declares the same queries
//! (`game/server/NextBot/NextBot.cpp:77-93`): its bot ID, immobility, update
//! flag, and whom it counts as friend or enemy.
//!
//! # Unverified
//!
//! The wrappers follow the script descriptors of Valve's Source SDK 2013 and
//! have not been tested on a live server.

#[cfg(test)]
#[path = "../tests/tf2/bots.rs"]
mod tests;

use crate::entities::Entity;
use crate::tf2::PlayerClass;
use crate::tf2::script_binding::{self as binding, BindingError, FLOAT, VOID, float, string};
use crate::tf2::script_instances::{ScriptInstance, ScriptInstanceError};
use crate::{Game, InterfaceError, Server};

use sdk_raw::tf2::bots::{
	self as raw, attribute, behavior, difficulty, mission, weapon_restriction,
};

use sdk_raw::tf2::script_binding::{BOOL, HANDLE, INT, boolean, handle, int};
use std::ffi::{CStr, CString, c_int};
use std::num::NonZeroU8;

pub use raw::TF_BOT_TYPE;

/// The longest bot name, in bytes: the engine's `MAX_PLAYER_NAME_LENGTH`
/// (`public/const.h`) less the terminator. The engine cuts longer names
/// short.
pub const MAX_NAME_LEN: usize = 31;

/// Why a bot or NextBot actor could not be wrapped or controlled, or a bot
/// command could not be queued.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BotError {
	/// The server is not running Team Fortress 2.
	#[error("bots require Team Fortress 2")]
	UnsupportedGame,

	/// The entity is not one of TF2's bots: it is not a `CTFPlayer`, or its
	/// bot type is not [`TF_BOT_TYPE`].
	#[error("the entity is not one of TF2's bots")]
	NotATfBot,

	/// The entity is neither one of TF2's bots nor a
	/// `NextBotCombatCharacter`.
	#[error("the entity is not a NextBot actor")]
	NotANextBot,

	/// The bot or actor is pending deletion, so it is not called.
	#[error("the bot is marked for deletion")]
	MarkedForDeletion,

	/// The script class descriptors lack the native method, or its signature
	/// differs from the SDK's.
	#[error("the game does not expose the expected native bot method")]
	UnsupportedMethod,

	/// The native method's binding adapter reported failure.
	#[error("the native bot method rejected the call")]
	Rejected,

	/// The engine interface that queues commands is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// An entity's script instance, through which a native method takes or
	/// returns it, could not be found or made.
	#[error(transparent)]
	ScriptInstance(#[from] ScriptInstanceError),
}

impl From<BindingError> for BotError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// [`TfBotRequest::named`] refused a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BotNameError {
	/// The name is empty.
	#[error("the bot name is empty")]
	Empty,

	/// The name is longer than [`MAX_NAME_LEN`] bytes, which the engine would
	/// cut short.
	#[error("the bot name is {len} bytes long, but at most {MAX_NAME_LEN} fit")]
	TooLong {
		/// The name's length in bytes.
		len: usize,
	},

	/// The name contains a control byte, a quote or a semicolon, which would
	/// end the name or the command.
	#[error("the bot name contains the byte {byte:#04x} at index {index}")]
	InvalidByte {
		/// The byte's index in the name.
		index: usize,

		/// The refused byte.
		byte: u8,
	},

	/// `tf_bot_add` would read the name as another argument: a class, a team,
	/// a difficulty, `noquota`, or a number of bots, which is any argument
	/// `atoi` reads as positive, such as `3rd`.
	#[error("`tf_bot_add` would read the bot name as another argument")]
	Reserved,
}

bitflags::bitflags! {
	/// A bot's attribute flags (`CTFBot::AttributeType`). Unnamed bits are
	/// kept, though the game defines none.
	#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
	pub struct BotAttributes: u32 {
		/// In Mann vs. Machine, pushes for the capture point.
		const AGGRESSIVE = attribute::AGGRESSIVE as u32;

		/// A Demoman that charges only in the air.
		const AIR_CHARGE_ONLY = attribute::AIR_CHARGE_ONLY as u32;

		/// Always fires critical hits.
		const ALWAYS_CRIT = attribute::ALWAYS_CRIT as u32;

		/// Fires its weapon constantly.
		const ALWAYS_FIRE_WEAPON = attribute::ALWAYS_FIRE_WEAPON as u32;

		/// Jumps on its own, at intervals [`TfBot::set_auto_jump`] sets.
		const AUTO_JUMP = attribute::AUTO_JUMP as u32;

		/// Moves to the spectators when killed.
		const BECOME_SPECTATOR_ON_DEATH = attribute::BECOME_SPECTATOR_ON_DEATH as u32;

		/// Shielded against blasts.
		const BLAST_IMMUNE = attribute::BLAST_IMMUNE as u32;

		/// Shielded against bullets.
		const BULLET_IMMUNE = attribute::BULLET_IMMUNE as u32;

		/// Does not dodge.
		const DISABLE_DODGE = attribute::DISABLE_DODGE as u32;

		/// Shielded against fire.
		const FIRE_IMMUNE = attribute::FIRE_IMMUNE as u32;

		/// Waits for a barrage weapon, such as a rocket launcher, to reload
		/// fully before firing it.
		const HOLD_FIRE_UNTIL_FULL_RELOAD = attribute::HOLD_FIRE_UNTIL_FULL_RELOAD as u32;

		/// Ignores its enemies.
		const IGNORE_ENEMIES = attribute::IGNORE_ENEMIES as u32;

		/// Does not pick up the flag or the bomb.
		const IGNORE_FLAG = attribute::IGNORE_FLAG as u32;

		/// A non-player support character.
		const IS_NPC = attribute::IS_NPC as u32;

		/// A Mann vs. Machine mini-boss.
		const MINIBOSS = attribute::MINIBOSS as u32;

		/// A Demoman or Soldier that opens a parachute when falling.
		const PARACHUTE = attribute::PARACHUTE as u32;

		/// Prefers the Vaccinator's blast resistance.
		const PREFER_VACCINATOR_BLAST = attribute::PREFER_VACCINATOR_BLAST as u32;

		/// Prefers the Vaccinator's bullet resistance.
		const PREFER_VACCINATOR_BULLETS = attribute::PREFER_VACCINATOR_BULLETS as u32;

		/// Prefers the Vaccinator's fire resistance.
		const PREFER_VACCINATOR_FIRE = attribute::PREFER_VACCINATOR_FIRE as u32;

		/// Defends when it can.
		const PRIORITIZE_DEFENSE = attribute::PRIORITIZE_DEFENSE as u32;

		/// A Medic that deploys a projectile shield.
		const PROJECTILE_SHIELD = attribute::PROJECTILE_SHIELD as u32;

		/// Counted by the bot quota, `tf_bot_quota`, which may kick the bot
		/// when the quota drops. [`TfBotRequest::quota_managed`] asks for it.
		#[doc(alias("QUOTA_MANANGED"))]
		const QUOTA_MANAGED = attribute::QUOTA_MANAGED as u32;

		/// Kicked from the server when killed.
		const REMOVE_ON_DEATH = attribute::REMOVE_ON_DEATH as u32;

		/// Keeps its buildings when it disconnects.
		const RETAIN_BUILDINGS = attribute::RETAIN_BUILDINGS as u32;

		/// Spawns with every weapon fully charged, such as an ÜberCharge.
		const SPAWN_WITH_FULL_CHARGE = attribute::SPAWN_WITH_FULL_CHARGE as u32;

		/// Holds its fire.
		const SUPPRESS_FIRE = attribute::SUPPRESS_FIRE as u32;

		/// Teleports to its hint target instead of walking out of its spawn.
		const TELEPORT_TO_HINT = attribute::TELEPORT_TO_HINT as u32;

		/// Shows its health in the boss health bar.
		const USE_BOSS_HEALTH_BAR = attribute::USE_BOSS_HEALTH_BAR as u32;
	}
}

bitflags::bitflags! {
	/// A bot's behaviour flags (`TFBOT_*`), the spawn flags of the
	/// `bot_generator` entity. Unnamed bits are kept.
	#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
	pub struct BehaviorFlags: u32 {
		/// Ignores enemy Demomen.
		#[doc(alias("TFBOT_IGNORE_ENEMY_DEMOMEN"))]
		const IGNORE_ENEMY_DEMOMEN = behavior::IGNORE_ENEMY_DEMOMEN as u32;

		/// Ignores enemy Engineers.
		#[doc(alias("TFBOT_IGNORE_ENEMY_ENGINEERS"))]
		const IGNORE_ENEMY_ENGINEERS = behavior::IGNORE_ENEMY_ENGINEERS as u32;

		/// Ignores enemy Heavies.
		#[doc(alias("TFBOT_IGNORE_ENEMY_HEAVIES"))]
		const IGNORE_ENEMY_HEAVIES = behavior::IGNORE_ENEMY_HEAVIES as u32;

		/// Ignores enemy Medics.
		#[doc(alias("TFBOT_IGNORE_ENEMY_MEDICS"))]
		const IGNORE_ENEMY_MEDICS = behavior::IGNORE_ENEMY_MEDICS as u32;

		/// Ignores enemy Pyros.
		#[doc(alias("TFBOT_IGNORE_ENEMY_PYROS"))]
		const IGNORE_ENEMY_PYROS = behavior::IGNORE_ENEMY_PYROS as u32;

		/// Ignores enemy Scouts.
		#[doc(alias("TFBOT_IGNORE_ENEMY_SCOUTS"))]
		const IGNORE_ENEMY_SCOUTS = behavior::IGNORE_ENEMY_SCOUTS as u32;

		/// Ignores enemy sentry guns.
		#[doc(alias("TFBOT_IGNORE_ENEMY_SENTRY_GUNS"))]
		const IGNORE_ENEMY_SENTRY_GUNS = behavior::IGNORE_ENEMY_SENTRY_GUNS as u32;

		/// Ignores enemy Snipers.
		#[doc(alias("TFBOT_IGNORE_ENEMY_SNIPERS"))]
		const IGNORE_ENEMY_SNIPERS = behavior::IGNORE_ENEMY_SNIPERS as u32;

		/// Ignores enemy Soldiers.
		#[doc(alias("TFBOT_IGNORE_ENEMY_SOLDIERS"))]
		const IGNORE_ENEMY_SOLDIERS = behavior::IGNORE_ENEMY_SOLDIERS as u32;

		/// Ignores enemy Spies.
		#[doc(alias("TFBOT_IGNORE_ENEMY_SPIES"))]
		const IGNORE_ENEMY_SPIES = behavior::IGNORE_ENEMY_SPIES as u32;

		/// Ignores the map's goals, such as capture points and the flag.
		#[doc(alias("TFBOT_IGNORE_SCENARIO_GOALS"))]
		const IGNORE_SCENARIO_GOALS = behavior::IGNORE_SCENARIO_GOALS as u32;
	}
}

bitflags::bitflags! {
	/// The weapons a bot may not use (`CTFBot::WeaponRestrictionType`). The
	/// empty set, `ANY_WEAPON`, restricts nothing. Unnamed bits are kept.
	#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
	pub struct WeaponRestrictions: u32 {
		/// Uses only its melee weapon.
		const MELEE_ONLY = weapon_restriction::MELEE_ONLY as u32;

		/// Uses only its primary weapon.
		const PRIMARY_ONLY = weapon_restriction::PRIMARY_ONLY as u32;

		/// Uses only its secondary weapon.
		const SECONDARY_ONLY = weapon_restriction::SECONDARY_ONLY as u32;
	}
}

/// The team `tf_bot_add` puts its bots on. Without one, the game picks a team
/// as for a player who joins with auto-assign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BotTeam {
	/// RED.
	Red,

	/// BLU.
	Blue,
}

impl BotTeam {
	/// The argument `tf_bot_add` and `tf_bot_kick` take for the team.
	pub const fn argument(self) -> &'static CStr {
		match self {
			Self::Red => c"red",
			Self::Blue => c"blue",
		}
	}
}

/// How many bots a request adds, and what they are named.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Bots {
	/// This many bots, which the game names.
	Count(NonZeroU8),

	/// One bot with this name, checked by [`check_name`].
	Named(CString),
}

/// A bot's skill (`CTFBot::DifficultyType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Difficulty {
	/// `EASY`: 0.
	#[doc(alias("EASY"))]
	Easy = difficulty::EASY as isize,

	/// `NORMAL`: 1.
	#[doc(alias("NORMAL"))]
	Normal = difficulty::NORMAL as isize,

	/// `HARD`: 2.
	#[doc(alias("HARD"))]
	Hard = difficulty::HARD as isize,

	/// `EXPERT`: 3.
	#[doc(alias("EXPERT"))]
	Expert = difficulty::EXPERT as isize,
}

impl Difficulty {
	/// Every difficulty, from the easiest.
	pub const ALL: [Self; 4] = [Self::Easy, Self::Normal, Self::Hard, Self::Expert];

	/// The difficulty with this number, or `None` for any other number,
	/// including `UNDEFINED` (-1).
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			difficulty::EASY => Some(Self::Easy),
			difficulty::NORMAL => Some(Self::Normal),
			difficulty::HARD => Some(Self::Hard),
			difficulty::EXPERT => Some(Self::Expert),
			_ => None,
		}
	}

	/// The argument `tf_bot_add` takes for the difficulty, which it compares
	/// ignoring case.
	pub const fn argument(self) -> &'static CStr {
		match self {
			Self::Easy => c"easy",
			Self::Normal => c"normal",
			Self::Hard => c"hard",
			Self::Expert => c"expert",
		}
	}

	/// The difficulty's number.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

/// What a bot is doing (`CTFBot::MissionType`). Most bots have
/// [`Self::NoMission`]: missions are what Mann vs. Machine and the bot quota
/// give their specialists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mission {
	/// `NO_MISSION`: the bot plays normally.
	#[doc(alias("NO_MISSION"))]
	NoMission = mission::NO_MISSION as isize,

	/// `MISSION_SEEK_AND_DESTROY`: finds and kills enemy players.
	#[doc(alias("MISSION_SEEK_AND_DESTROY"))]
	SeekAndDestroy = mission::SEEK_AND_DESTROY as isize,

	/// `MISSION_DESTROY_SENTRIES`: finds and destroys enemy sentry guns and
	/// other buildings.
	#[doc(alias("MISSION_DESTROY_SENTRIES"))]
	DestroySentries = mission::DESTROY_SENTRIES as isize,

	/// `MISSION_SNIPER`: harasses the enemy as a team of Snipers.
	#[doc(alias("MISSION_SNIPER"))]
	Sniper = mission::SNIPER as isize,

	/// `MISSION_SPY`: harasses the enemy as a team of Spies.
	#[doc(alias("MISSION_SPY"))]
	Spy = mission::SPY as isize,

	/// `MISSION_ENGINEER`: harasses the enemy from an Engineer's nest.
	#[doc(alias("MISSION_ENGINEER"))]
	Engineer = mission::ENGINEER as isize,

	/// `MISSION_REPROGRAMMED`: a Mann vs. Machine robot hacked to turn on its
	/// team.
	#[doc(alias("MISSION_REPROGRAMMED"))]
	Reprogrammed = mission::REPROGRAMMED as isize,
}

impl Mission {
	/// The mission with this number, or `None` for any other number.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			mission::NO_MISSION => Some(Self::NoMission),
			mission::SEEK_AND_DESTROY => Some(Self::SeekAndDestroy),
			mission::DESTROY_SENTRIES => Some(Self::DestroySentries),
			mission::SNIPER => Some(Self::Sniper),
			mission::SPY => Some(Self::Spy),
			mission::ENGINEER => Some(Self::Engineer),
			mission::REPROGRAMMED => Some(Self::Reprogrammed),
			_ => None,
		}
	}

	/// The mission's number.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

/// A bot or another NextBot actor within the current engine callback, for the
/// queries `INextBot` declares.
#[derive(Debug, Clone, Copy)]
pub struct NextBot<'s> {
	entity: Entity<'s>,

	/// The script class declaring the queries for this actor:
	/// [`raw::TF_BOT_CLASS`] for a bot, and
	/// [`raw::NEXT_BOT_COMBAT_CHARACTER_CLASS`] for anything else.
	class: &'static CStr,
}

impl<'s> NextBot<'s> {
	/// Wraps `entity`: one of TF2's bots, or an entity whose data maps include
	/// `NextBotCombatCharacter`'s, such as a `base_boss`, a `tank_boss`, a
	/// skeleton (`tf_zombie`) or a Halloween boss. Fails with
	/// [`BotError::NotANextBot`] for anything else.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, BotError> {
		match TfBot::new(server, entity) {
			Ok(bot) => return Ok(bot.next_bot()),
			Err(BotError::NotATfBot) => {}
			Err(error) => return Err(error),
		}

		if !entity
			.data_maps()
			.any(|map| map.class_name() == Some(raw::NEXT_BOT_COMBAT_CHARACTER_CLASS))
		{
			return Err(BotError::NotANextBot);
		}

		Ok(Self {
			entity,
			class: raw::NEXT_BOT_COMBAT_CHARACTER_CLASS,
		})
	}

	/// Calls a NextBot query without arguments that returns a `bool`.
	///
	/// # Safety
	///
	/// As for [`Self::call`].
	unsafe fn bool(self, name: &CStr) -> Result<bool, BotError> {
		// SAFETY: As the caller promises.
		let result = unsafe { self.call(name, &mut [], BOOL) }?;

		// SAFETY: `call` checked that the adapter returned a boolean variant.
		Ok(unsafe { result.__bindgen_anon_1.m_bool })
	}

	/// The actor's ID among the NextBot manager's actors, which it numbers
	/// from 0 as they are created.
	#[doc(alias("GetBotId"))]
	pub fn bot_id(self) -> Result<c_int, BotError> {
		// SAFETY: The query reads a field of the actor.
		unsafe { self.int(c"GetBotId") }
	}

	/// Calls one of the actor's NextBot methods, unless it is marked for
	/// deletion.
	///
	/// # Safety
	///
	/// As for [`binding::call`]: the method must accept the actor and these
	/// arguments.
	unsafe fn call(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
		result: sys::ScriptDataType_t,
	) -> Result<sys::ScriptVariant_t, BotError> {
		if self.entity.is_marked_for_deletion() {
			return Err(BotError::MarkedForDeletion);
		}

		// SAFETY: As the caller promises.
		Ok(unsafe { binding::call(self.entity, self.class, name, arguments, result) }?)
	}

	/// Clears the actor's immobility: it counts as having moved now.
	#[doc(alias("ClearImmobileStatus"))]
	pub fn clear_immobile_status(self) -> Result<(), BotError> {
		// SAFETY: The method resets a timer and an anchor of the actor.
		unsafe { self.call(c"ClearImmobileStatus", &mut [], VOID) }.map(drop)
	}

	/// The entity the actor is.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Flags the actor for an update, or clears its flag. The NextBot manager
	/// updates flagged actors first when it spreads updates over frames.
	#[doc(alias("FlagForUpdate"))]
	pub fn flag_for_update(self, flag: bool) -> Result<(), BotError> {
		// SAFETY: The method sets a flag of the actor.
		unsafe { self.call(c"FlagForUpdate", &mut [boolean(flag)], VOID) }.map(drop)
	}

	/// Calls a NextBot query without arguments that returns an `f32`.
	///
	/// # Safety
	///
	/// As for [`Self::call`].
	unsafe fn float(self, name: &CStr) -> Result<f32, BotError> {
		// SAFETY: As the caller promises.
		let result = unsafe { self.call(name, &mut [], FLOAT) }?;

		// SAFETY: `call` checked that the adapter returned a float variant.
		Ok(unsafe { result.__bindgen_anon_1.m_float })
	}

	/// How long, in seconds, the actor has been immobile, or 0 if it is not.
	#[doc(alias("GetImmobileDuration"))]
	pub fn immobile_duration(self) -> Result<f32, BotError> {
		// SAFETY: The query reads a timer of the actor.
		unsafe { self.float(c"GetImmobileDuration") }
	}

	/// The speed, in units per second, below which the actor counts as
	/// immobile.
	#[doc(alias("GetImmobileSpeedThreshold"))]
	pub fn immobile_speed_threshold(self) -> Result<f32, BotError> {
		// SAFETY: The query returns a constant of the actor's class.
		unsafe { self.float(c"GetImmobileSpeedThreshold") }
	}

	/// Calls a NextBot query without arguments that returns an `int`.
	///
	/// # Safety
	///
	/// As for [`Self::call`].
	unsafe fn int(self, name: &CStr) -> Result<c_int, BotError> {
		// SAFETY: As the caller promises.
		let result = unsafe { self.call(name, &mut [], INT) }?;

		// SAFETY: `call` checked that the adapter returned an integer variant.
		Ok(unsafe { result.__bindgen_anon_1.m_int })
	}

	/// Whether `other` is the actor's enemy: on another team, as
	/// `INextBot::IsEnemy` decides unless the actor's class overrides it.
	///
	/// The other entity's [`ScriptInstance`] is made if it has none, which gives
	/// it a script scope too.
	#[doc(alias("IsEnemy"))]
	pub fn is_enemy(self, server: Server<'s>, other: Entity<'s>) -> Result<bool, BotError> {
		let other = ScriptInstance::of(server, other)?;

		// SAFETY: `ScriptIsEnemy` resolves the instance to its entity and compares
		// teams.
		let result = unsafe { self.call(c"IsEnemy", &mut [handle(other.as_raw())], BOOL) }?;

		// SAFETY: `call` checked that the adapter returned a boolean variant.
		Ok(unsafe { result.__bindgen_anon_1.m_bool })
	}

	/// Whether the actor is flagged for an update.
	#[doc(alias("IsFlaggedForUpdate"))]
	pub fn is_flagged_for_update(self) -> Result<bool, BotError> {
		// SAFETY: The query reads a flag of the actor.
		unsafe { self.bool(c"IsFlaggedForUpdate") }
	}

	/// Whether `other` is the actor's friend: on its team, as
	/// `INextBot::IsFriend` decides unless the actor's class overrides it.
	///
	/// The other entity's [`ScriptInstance`] is made if it has none, which gives
	/// it a script scope too.
	#[doc(alias("IsFriend"))]
	pub fn is_friend(self, server: Server<'s>, other: Entity<'s>) -> Result<bool, BotError> {
		let other = ScriptInstance::of(server, other)?;

		// SAFETY: `ScriptIsFriend` resolves the instance to its entity and
		// compares teams.
		let result = unsafe { self.call(c"IsFriend", &mut [handle(other.as_raw())], BOOL) }?;

		// SAFETY: `call` checked that the adapter returned a boolean variant.
		Ok(unsafe { result.__bindgen_anon_1.m_bool })
	}

	/// Whether the actor has not moved for a while.
	#[doc(alias("IsImmobile"))]
	pub fn is_immobile(self) -> Result<bool, BotError> {
		// SAFETY: The query reads a timer of the actor.
		unsafe { self.bool(c"IsImmobile") }
	}

	/// The tick on which the actor last updated.
	#[doc(alias("GetTickLastUpdate"))]
	pub fn tick_last_update(self) -> Result<c_int, BotError> {
		// SAFETY: The query reads a field of the actor.
		unsafe { self.int(c"GetTickLastUpdate") }
	}
}

/// One of TF2's bots within the current engine callback.
///
/// Methods fail with [`BotError::MarkedForDeletion`] once the bot is pending
/// deletion, [`BotError::UnsupportedMethod`] when the game lacks the expected
/// native method, and [`BotError::Rejected`] when the method's binding
/// reports failure.
#[derive(Debug, Clone, Copy)]
pub struct TfBot<'s> {
	entity: Entity<'s>,
}

impl<'s> TfBot<'s> {
	/// Wraps `entity`. Fails with [`BotError::UnsupportedGame`] outside TF2,
	/// [`BotError::MarkedForDeletion`] for an entity pending deletion, and
	/// [`BotError::NotATfBot`] unless it is a `CTFPlayer` whose bot type is
	/// [`TF_BOT_TYPE`].
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, BotError> {
		if server.game() != Game::TeamFortress2 {
			return Err(BotError::UnsupportedGame);
		}

		if entity.is_marked_for_deletion() {
			return Err(BotError::MarkedForDeletion);
		}

		// SAFETY: `GetBotType` reads a constant of the player's class, through
		// `CTFPlayer`'s wrapper (`game/server/tf/tf_player.h:864`).
		let result =
			unsafe { binding::call(entity, raw::TF_PLAYER_CLASS, c"GetBotType", &mut [], INT) };

		let bot_type = match result {
			// SAFETY: `call` checked that the adapter returned an integer variant.
			Ok(result) => unsafe { result.__bindgen_anon_1.m_int },

			// The entity's script classes do not include `CTFPlayer`.
			Err(BindingError::Unavailable) => return Err(BotError::NotATfBot),

			Err(error) => return Err(error.into()),
		};

		if bot_type != TF_BOT_TYPE {
			return Err(BotError::NotATfBot);
		}

		Ok(Self { entity })
	}

	/// Adds attribute flags.
	#[doc(alias("AddBotAttribute", "SetAttribute"))]
	pub fn add_attributes(self, attributes: BotAttributes) -> Result<(), BotError> {
		// SAFETY: The method sets bits of a field of the bot.
		unsafe { self.void(c"AddBotAttribute", &mut [int(attributes.bits() as c_int)]) }
	}

	/// Adds behaviour flags.
	#[doc(alias("SetBehaviorFlag"))]
	pub fn add_behavior_flags(self, flags: BehaviorFlags) -> Result<(), BotError> {
		// SAFETY: The method sets bits of a field of the bot.
		unsafe { self.void(c"SetBehaviorFlag", &mut [int(flags.bits() as c_int)]) }
	}

	/// Adds a tag, which Mann vs. Machine's populators and map logic match
	/// bots by, unless the bot has it already. Tags are compared ignoring
	/// ASCII case.
	#[doc(alias("AddBotTag", "AddTag"))]
	pub fn add_tag(self, tag: &CStr) -> Result<(), BotError> {
		// SAFETY: The method copies the tag into the bot's list.
		unsafe { self.void(c"AddBotTag", &mut [string(tag)]) }
	}

	/// Restricts the bot to a weapon, adding to its restrictions.
	#[doc(alias("AddWeaponRestriction", "SetWeaponRestriction"))]
	pub fn add_weapon_restrictions(self, restrictions: WeaponRestrictions) -> Result<(), BotError> {
		// SAFETY: The method sets bits of a field of the bot.
		unsafe {
			self.void(
				c"AddWeaponRestriction",
				&mut [int(restrictions.bits() as c_int)],
			)
		}
	}

	/// The bot's attribute flags, tested one named flag at a time.
	pub fn attributes(self) -> Result<BotAttributes, BotError> {
		let mut attributes = BotAttributes::empty();

		for flag in BotAttributes::all().iter() {
			if self.has_any_attribute(flag)? {
				attributes |= flag;
			}
		}

		Ok(attributes)
	}

	/// The bot's behaviour flags, tested one named flag at a time.
	pub fn behavior_flags(self) -> Result<BehaviorFlags, BotError> {
		let mut flags = BehaviorFlags::empty();

		for flag in BehaviorFlags::all().iter() {
			if self.has_any_behavior_flag(flag)? {
				flags |= flag;
			}
		}

		Ok(flags)
	}

	/// Calls a native method returning a `bool`.
	///
	/// # Safety
	///
	/// As for [`Self::call`].
	unsafe fn bool(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
	) -> Result<bool, BotError> {
		// SAFETY: As the caller promises.
		let result = unsafe { self.call(name, arguments, BOOL) }?;

		// SAFETY: `call` checked that the adapter returned a boolean variant.
		Ok(unsafe { result.__bindgen_anon_1.m_bool })
	}

	/// Calls one of `CTFBot`'s native methods, unless the bot is marked for
	/// deletion.
	///
	/// # Safety
	///
	/// As for [`binding::call`]: the method must accept the bot and these
	/// arguments.
	unsafe fn call(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
		result: sys::ScriptDataType_t,
	) -> Result<sys::ScriptVariant_t, BotError> {
		// SAFETY: As the caller promises.
		unsafe { self.next_bot().call(name, arguments, result) }
	}

	/// Clears the entity the bot's attention is restricted to, if any.
	#[doc(alias("ClearAttentionFocus"))]
	pub fn clear_attention_focus(self) -> Result<(), BotError> {
		// SAFETY: The method clears a handle of the bot.
		unsafe { self.void(c"ClearAttentionFocus", &mut []) }
	}

	/// Clears every attribute flag.
	#[doc(alias("ClearAllBotAttributes", "ClearAllAttributes"))]
	pub fn clear_attributes(self) -> Result<(), BotError> {
		// SAFETY: The method clears a field of the bot.
		unsafe { self.void(c"ClearAllBotAttributes", &mut []) }
	}

	/// Removes every tag.
	#[doc(alias("ClearAllBotTags", "ClearTags"))]
	pub fn clear_tags(self) -> Result<(), BotError> {
		// SAFETY: The method empties the bot's list of tags.
		unsafe { self.void(c"ClearAllBotTags", &mut []) }
	}

	/// Lifts every weapon restriction.
	#[doc(alias("ClearAllWeaponRestrictions", "ClearWeaponRestrictions"))]
	pub fn clear_weapon_restrictions(self) -> Result<(), BotError> {
		// SAFETY: The method clears a field of the bot.
		unsafe { self.void(c"ClearAllWeaponRestrictions", &mut []) }
	}

	/// Notices `threat` after `delay` seconds, unless the bot already will
	/// sooner. Panics unless the delay is finite and nonnegative.
	///
	/// The threat's [`ScriptInstance`] is made if it has none, which gives it a
	/// script scope too.
	#[doc(alias("DelayedThreatNotice"))]
	pub fn delayed_threat_notice(
		self,
		server: Server<'s>,
		threat: Entity<'s>,
		delay: f32,
	) -> Result<(), BotError> {
		assert!(delay.is_finite() && delay >= 0.0, "invalid notice delay");

		let threat = ScriptInstance::of(server, threat)?;

		// SAFETY: `ScriptDelayedThreatNotice` resolves the instance to its entity
		// and queues a notice of it, kept by handle, in the bot.
		unsafe {
			self.void(
				c"DelayedThreatNotice",
				&mut [handle(threat.as_raw()), float(delay)],
			)
		}
	}

	/// The bot's skill, or `None` for a value the SDK does not know.
	#[doc(alias("GetDifficulty"))]
	pub fn difficulty(self) -> Result<Option<Difficulty>, BotError> {
		// SAFETY: The method reads a field of the bot.
		Ok(Difficulty::from_raw(unsafe {
			self.int(c"GetDifficulty", &mut [])
		}?))
	}

	/// Disbands the bot's squad, if it is in one, for every member.
	#[doc(alias("DisbandCurrentSquad"))]
	pub fn disband_squad(self) -> Result<(), BotError> {
		// SAFETY: The method has each member leave the squad, which frees
		// itself once empty; squads are not entities.
		unsafe { self.void(c"DisbandCurrentSquad", &mut []) }
	}

	/// The entity the bot is.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Calls a native method returning an `f32`.
	///
	/// # Safety
	///
	/// As for [`Self::call`].
	unsafe fn float(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
	) -> Result<f32, BotError> {
		// SAFETY: As the caller promises.
		let result = unsafe { self.call(name, arguments, FLOAT) }?;

		// SAFETY: `call` checked that the adapter returned a float variant.
		Ok(unsafe { result.__bindgen_anon_1.m_float })
	}

	/// Generates the item with this name from the item schema and gives it to
	/// the bot, as a bot of Mann vs. Machine's populators is given its items.
	/// A weapon replaces the bot's weapon of the same class, which the game
	/// removes through its deferred deletion. The game prints a message and
	/// gives nothing for a name the schema lacks.
	#[doc(alias("GenerateAndWearItem"))]
	pub fn generate_and_wear_item(self, name: &CStr) -> Result<(), BotError> {
		// SAFETY: `BotGenerateAndWearItem` (`game/server/tf/tf_bot_temp.cpp:1386`)
		// spawns the generated item and gives it to the bot, and removes a
		// weapon it replaces with `UTIL_Remove`, which defers the deletion.
		unsafe { self.void(c"GenerateAndWearItem", &mut [string(name)]) }
	}

	/// Whether the bot has any of `attributes`, as the game's `HasAttribute`
	/// tests.
	#[doc(alias("HasBotAttribute", "HasAttribute"))]
	pub fn has_any_attribute(self, attributes: BotAttributes) -> Result<bool, BotError> {
		// SAFETY: The method reads a field of the bot.
		unsafe { self.bool(c"HasBotAttribute", &mut [int(attributes.bits() as c_int)]) }
	}

	/// Whether the bot has any of `flags`, as the game's `IsBehaviorFlagSet`
	/// tests.
	#[doc(alias("IsBehaviorFlagSet"))]
	pub fn has_any_behavior_flag(self, flags: BehaviorFlags) -> Result<bool, BotError> {
		// SAFETY: The method reads a field of the bot.
		unsafe { self.bool(c"IsBehaviorFlagSet", &mut [int(flags.bits() as c_int)]) }
	}

	/// Whether the bot has any of `restrictions`, as the game's
	/// `HasWeaponRestriction` tests.
	#[doc(alias("HasWeaponRestriction"))]
	pub fn has_any_weapon_restriction(
		self,
		restrictions: WeaponRestrictions,
	) -> Result<bool, BotError> {
		// SAFETY: The method reads a field of the bot.
		unsafe {
			self.bool(
				c"HasWeaponRestriction",
				&mut [int(restrictions.bits() as c_int)],
			)
		}
	}

	/// Whether the bot has the tag, compared ignoring ASCII case.
	#[doc(alias("HasBotTag", "HasTag"))]
	pub fn has_tag(self, tag: &CStr) -> Result<bool, BotError> {
		// SAFETY: The method compares the tag with the bot's list.
		unsafe { self.bool(c"HasBotTag", &mut [string(tag)]) }
	}

	/// Calls a native method returning an `int`.
	///
	/// # Safety
	///
	/// As for [`Self::call`].
	unsafe fn int(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
	) -> Result<c_int, BotError> {
		// SAFETY: As the caller promises.
		let result = unsafe { self.call(name, arguments, INT) }?;

		// SAFETY: `call` checked that the adapter returned an integer variant.
		Ok(unsafe { result.__bindgen_anon_1.m_int })
	}

	/// Whether the bot's primary and secondary ammunition, and an Engineer's
	/// metal, are full.
	#[doc(alias("IsAmmoFull"))]
	pub fn is_ammo_full(self) -> Result<bool, BotError> {
		// SAFETY: The method reads the bot's ammunition.
		unsafe { self.bool(c"IsAmmoFull", &mut []) }
	}

	/// Whether the bot's active weapon is low on ammunition. A melee weapon
	/// never is, except an Engineer's wrench without metal.
	#[doc(alias("IsAmmoLow"))]
	pub fn is_ammo_low(self) -> Result<bool, BotError> {
		// SAFETY: The method reads the bot's ammunition and weapon.
		unsafe { self.bool(c"IsAmmoLow", &mut []) }
	}

	/// Whether the bot's attention is restricted to an entity.
	#[doc(alias("IsAttentionFocused"))]
	pub fn is_attention_focused(self) -> Result<bool, BotError> {
		// SAFETY: The method reads a handle of the bot.
		unsafe { self.bool(c"IsAttentionFocused", &mut []) }
	}

	/// Whether the bot's attention is restricted to `target`, compared by
	/// entity index.
	///
	/// The target's [`ScriptInstance`] is made if it has none, which gives it a
	/// script scope too.
	#[doc(alias("IsAttentionFocusedOn"))]
	pub fn is_attention_focused_on(
		self,
		server: Server<'s>,
		target: Entity<'s>,
	) -> Result<bool, BotError> {
		let target = ScriptInstance::of(server, target)?;

		// SAFETY: `ScriptIsAttentionFocusedOn` resolves the instance to its entity
		// and compares it with a handle of the bot.
		unsafe { self.bool(c"IsAttentionFocusedOn", &mut [handle(target.as_raw())]) }
	}

	/// Whether the bot is in a squad.
	#[doc(alias("IsInASquad"))]
	pub fn is_in_squad(self) -> Result<bool, BotError> {
		// SAFETY: The method reads a pointer of the bot.
		unsafe { self.bool(c"IsInASquad", &mut []) }
	}

	/// Whether the bot has a mission other than [`Mission::NoMission`].
	#[doc(alias("IsOnAnyMission"))]
	pub fn is_on_any_mission(self) -> Result<bool, BotError> {
		// SAFETY: The method reads a field of the bot.
		unsafe { self.bool(c"IsOnAnyMission", &mut []) }
	}

	/// Whether the bot may not use `weapon` under its weapon restrictions. A
	/// weapon outside the restricted slots is not restricted, and anything that
	/// is not one of TF2's weapons is.
	///
	/// The weapon's [`ScriptInstance`] is made if it has none, which gives it a
	/// script scope too.
	#[doc(alias("IsWeaponRestricted"))]
	pub fn is_weapon_restricted(
		self,
		server: Server<'s>,
		weapon: Entity<'s>,
	) -> Result<bool, BotError> {
		let weapon = ScriptInstance::of(server, weapon)?;

		// SAFETY: `ScriptIsWeaponRestricted` (`tf_bot.cpp:4056`) resolves the
		// instance to its entity, casts it to a TF2 weapon with `dynamic_cast`,
		// and reads its item definition's loadout slot.
		unsafe { self.bool(c"IsWeaponRestricted", &mut [handle(weapon.as_raw())]) }
	}

	/// Queues `kickid` with the bot's user ID, which disconnects it when the
	/// server next runs its command buffer, normally on the next frame. Fails
	/// with [`BotError::NotATfBot`] if no client owns the bot's edict anymore.
	pub fn kick(self, server: Server<'_>) -> Result<(), BotError> {
		let engine = server.valve_engine()?;

		let user_id = self
			.entity
			.edict()
			.and_then(|edict| engine.user_id_of_edict(edict))
			.ok_or(BotError::NotATfBot)?;

		let command = format!("kickid {}\n", user_id.to_raw());

		engine.server_command(&CString::new(command).expect("a number has no NUL"));
		Ok(())
	}

	/// Has the bot leave its squad, if it is in one.
	#[doc(alias("LeaveSquad"))]
	pub fn leave_squad(self) -> Result<(), BotError> {
		// SAFETY: The method removes the bot from its squad, which frees itself
		// once empty; squads are not entities.
		unsafe { self.void(c"LeaveSquad", &mut []) }
	}

	/// The range, in units, the bot sees to instead of its usual vision range,
	/// or `None` if it has no override.
	#[doc(alias("GetMaxVisionRangeOverride"))]
	pub fn max_vision_range_override(self) -> Result<Option<f32>, BotError> {
		// SAFETY: The method reads a field of the bot.
		let range = unsafe { self.float(c"GetMaxVisionRangeOverride", &mut []) }?;

		// The bot's vision uses the override only while it is positive
		// (`game/server/tf/bot/tf_bot_vision.cpp:461`).
		Ok((range > 0.0).then_some(range))
	}

	/// The bot's mission, or `None` for a value the SDK does not know.
	#[doc(alias("GetMission"))]
	pub fn mission(self) -> Result<Option<Mission>, BotError> {
		// SAFETY: The method reads a field of the bot.
		Ok(Mission::from_raw(unsafe {
			self.int(c"GetMission", &mut [])
		}?))
	}

	/// The entity the bot's mission is aimed at, such as the sentry gun of a
	/// sentry buster, or `None`.
	///
	/// The bot's own [`ScriptInstance`] is made if it has none, to check that
	/// the game has the script VM it needs to return the target. The target's
	/// instance is made if it has none, which gives it a script scope too.
	/// Finding the target's entity costs a pass over the entity list.
	#[doc(alias("GetMissionTarget"))]
	pub fn mission_target(self, server: Server<'s>) -> Result<Option<Entity<'s>>, BotError> {
		// SAFETY: `ScriptGetMissionTarget` returns the target's instance from
		// `ToHScript`, or null.
		unsafe { self.returned_entity(server, c"GetMissionTarget") }
	}

	/// The nearest enemy building within 500 units that the bot knows of and
	/// that has no sapper yet, as a Spy bot looks for one, or `None`.
	///
	/// As for [`Self::mission_target`], the bot's and the building's
	/// [`ScriptInstance`] are made if they have none, and finding the
	/// building's entity costs a pass over the entity list.
	#[doc(alias("GetNearestKnownSappableTarget"))]
	pub fn nearest_known_sappable_target(
		self,
		server: Server<'s>,
	) -> Result<Option<Entity<'s>>, BotError> {
		// SAFETY: `ScriptGetNearestKnownSappableTarget` (`tf_bot.cpp:4485`) reads
		// the entities the bot's vision knows, and returns the nearest building's
		// instance from `ToHScript`, or null.
		unsafe { self.returned_entity(server, c"GetNearestKnownSappableTarget") }
	}

	/// The bot's queries as a NextBot actor.
	pub const fn next_bot(self) -> NextBot<'s> {
		NextBot {
			entity: self.entity,
			class: raw::TF_BOT_CLASS,
		}
	}

	/// Presses a button through a native method taking its duration.
	fn press(self, name: &CStr, duration: Option<f32>) -> Result<(), BotError> {
		let duration = match duration {
			Some(duration) => {
				assert!(
					duration.is_finite() && duration >= 0.0,
					"invalid button duration"
				);
				duration
			}

			// The header's default, which presses it until the next update.
			None => -1.0,
		};

		// SAFETY: The methods set an input bit and start a timer of the bot.
		unsafe { self.void(name, &mut [float(duration)]) }
	}

	/// Presses the bot's alternate fire button, which stays pressed for
	/// `duration` seconds, or for the next update with `None`.
	#[doc(alias("PressAltFireButton"))]
	pub fn press_alt_fire_button(self, duration: Option<f32>) -> Result<(), BotError> {
		self.press(c"PressAltFireButton", duration)
	}

	/// Presses the bot's fire button, which stays pressed for `duration`
	/// seconds, or for the next update with `None`. A stunned bot does not
	/// fire.
	#[doc(alias("PressFireButton"))]
	pub fn press_fire_button(self, duration: Option<f32>) -> Result<(), BotError> {
		self.press(c"PressFireButton", duration)
	}

	/// Presses the bot's special fire button, which stays pressed for
	/// `duration` seconds, or for the next update with `None`.
	#[doc(alias("PressSpecialFireButton"))]
	pub fn press_special_fire_button(self, duration: Option<f32>) -> Result<(), BotError> {
		self.press(c"PressSpecialFireButton", duration)
	}

	/// The bot's previous mission, or `None` for a value the SDK does not
	/// know.
	#[doc(alias("GetPrevMission"))]
	pub fn previous_mission(self) -> Result<Option<Mission>, BotError> {
		// SAFETY: The method reads a field of the bot.
		Ok(Mission::from_raw(unsafe {
			self.int(c"GetPrevMission", &mut [])
		}?))
	}

	/// Whether the bot builds its buildings instantly.
	#[doc(alias("ShouldQuickBuild"))]
	pub fn quick_build(self) -> Result<bool, BotError> {
		// SAFETY: The method reads a field of the bot.
		unsafe { self.bool(c"ShouldQuickBuild", &mut []) }
	}

	/// Removes attribute flags.
	#[doc(alias("RemoveBotAttribute", "ClearAttribute"))]
	pub fn remove_attributes(self, attributes: BotAttributes) -> Result<(), BotError> {
		// SAFETY: The method clears bits of a field of the bot.
		unsafe {
			self.void(
				c"RemoveBotAttribute",
				&mut [int(attributes.bits() as c_int)],
			)
		}
	}

	/// Removes behaviour flags. [`BehaviorFlags::all`] removes every one.
	#[doc(alias("ClearBehaviorFlag"))]
	pub fn remove_behavior_flags(self, flags: BehaviorFlags) -> Result<(), BotError> {
		// SAFETY: The method clears bits of a field of the bot.
		unsafe { self.void(c"ClearBehaviorFlag", &mut [int(flags.bits() as c_int)]) }
	}

	/// Removes a tag, compared ignoring ASCII case.
	#[doc(alias("RemoveBotTag", "RemoveTag"))]
	pub fn remove_tag(self, tag: &CStr) -> Result<(), BotError> {
		// SAFETY: The method removes a matching tag from the bot's list.
		unsafe { self.void(c"RemoveBotTag", &mut [string(tag)]) }
	}

	/// Lifts weapon restrictions.
	#[doc(alias("RemoveWeaponRestriction"))]
	pub fn remove_weapon_restrictions(
		self,
		restrictions: WeaponRestrictions,
	) -> Result<(), BotError> {
		// SAFETY: The method clears bits of a field of the bot.
		unsafe {
			self.void(
				c"RemoveWeaponRestriction",
				&mut [int(restrictions.bits() as c_int)],
			)
		}
	}

	/// Calls a native method without arguments that returns an entity's script
	/// instance, and finds that entity.
	///
	/// # Safety
	///
	/// As for [`Self::call`], and the method must return an instance from
	/// `ToHScript`, or null.
	unsafe fn returned_entity(
		self,
		server: Server<'s>,
		name: &CStr,
	) -> Result<Option<Entity<'s>>, BotError> {
		// `ToHScript` makes the entity's instance without checking for a VM.
		ScriptInstance::of(server, self.entity)?;

		// SAFETY: As the caller promises. `GetScriptInstance` registers the
		// instance with the VM checked above.
		let result = unsafe { self.call(name, &mut [], HANDLE) }?;

		// SAFETY: `call` checked that the adapter returned a handle variant, whose
		// instance stays registered until its entity is removed, which the
		// callback defers past `'s`.
		let Some(instance) =
			(unsafe { ScriptInstance::from_raw(result.__bindgen_anon_1.m_hScript) })
		else {
			return Ok(None);
		};

		Ok(instance.entity(server)?)
	}

	/// Restricts the bot's attention to `target`, to the exclusion of
	/// everything else, until [`Self::clear_attention_focus`].
	///
	/// The target's [`ScriptInstance`] is made if it has none, which gives it a
	/// script scope too.
	#[doc(alias("SetAttentionFocus"))]
	pub fn set_attention_focus(
		self,
		server: Server<'s>,
		target: Entity<'s>,
	) -> Result<(), BotError> {
		let target = ScriptInstance::of(server, target)?;

		// SAFETY: `ScriptSetAttentionFocus` resolves the instance to its entity
		// and keeps it by handle in the bot.
		unsafe { self.void(c"SetAttentionFocus", &mut [handle(target.as_raw())]) }
	}

	/// Sets the shortest and longest interval, in seconds, between the jumps
	/// of a bot with [`BotAttributes::AUTO_JUMP`]. Panics unless both are
	/// finite, nonnegative, and `min` is at most `max`.
	#[doc(alias("SetAutoJump"))]
	pub fn set_auto_jump(self, min: f32, max: f32) -> Result<(), BotError> {
		assert!(
			min.is_finite() && max.is_finite() && 0.0 <= min && min <= max,
			"invalid auto jump interval"
		);

		// SAFETY: The method sets two fields of the bot.
		unsafe { self.void(c"SetAutoJump", &mut [float(min), float(max)]) }
	}

	/// Sets the bot's skill.
	#[doc(alias("SetDifficulty"))]
	pub fn set_difficulty(self, difficulty: Difficulty) -> Result<(), BotError> {
		// SAFETY: The method sets two fields of the bot to a valid skill.
		unsafe { self.void(c"SetDifficulty", &mut [int(difficulty.to_raw())]) }
	}

	/// Sets the range, in units, the bot sees to instead of its usual vision
	/// range, or removes the override with `None`. Panics unless a range is
	/// finite and positive.
	#[doc(alias("SetMaxVisionRangeOverride"))]
	pub fn set_max_vision_range_override(self, range: Option<f32>) -> Result<(), BotError> {
		let range = match range {
			Some(range) => {
				assert!(range.is_finite() && range > 0.0, "invalid vision range");
				range
			}

			// What the bot is constructed with (`tf_bot.cpp:1294`).
			None => -1.0,
		};

		// SAFETY: The method sets a field of the bot.
		unsafe { self.void(c"SetMaxVisionRangeOverride", &mut [float(range)]) }
	}

	/// Gives the bot a mission, keeping the old one as its previous mission.
	/// With `reset_behavior`, the bot's behaviour restarts to pursue it, as
	/// the game does when it changes a bot's mission.
	#[doc(alias("SetMission"))]
	pub fn set_mission(self, mission: Mission, reset_behavior: bool) -> Result<(), BotError> {
		// SAFETY: `CTFBot::SetMission` (`tf_bot.cpp:1363`) sets two fields,
		// resets the bot's intention interface on request, and starts an idle
		// sound for a mission, with a valid mission.
		unsafe {
			self.void(
				c"SetMission",
				&mut [int(mission.to_raw()), boolean(reset_behavior)],
			)
		}
	}

	/// Aims the bot's mission at `target`, or at nothing with `None`.
	///
	/// The target's [`ScriptInstance`] is made if it has none, which gives it a
	/// script scope too.
	#[doc(alias("SetMissionTarget"))]
	pub fn set_mission_target(
		self,
		server: Server<'s>,
		target: Option<Entity<'s>>,
	) -> Result<(), BotError> {
		let target = match target {
			Some(target) => ScriptInstance::of(server, target)?.as_raw(),
			None => std::ptr::null_mut(),
		};

		// SAFETY: `ScriptSetMissionTarget` resolves the instance, or null, to its
		// entity and keeps it by handle in the bot.
		unsafe { self.void(c"SetMissionTarget", &mut [handle(target)]) }
	}

	/// Sets the bot's previous mission.
	#[doc(alias("SetPrevMission"))]
	pub fn set_previous_mission(self, mission: Mission) -> Result<(), BotError> {
		// SAFETY: The method sets a field of the bot to a valid mission.
		unsafe { self.void(c"SetPrevMission", &mut [int(mission.to_raw())]) }
	}

	/// Sets whether the bot builds its buildings instantly.
	#[doc(alias("SetShouldQuickBuild"))]
	pub fn set_quick_build(self, quick_build: bool) -> Result<(), BotError> {
		// SAFETY: The method sets a field of the bot.
		unsafe { self.void(c"SetShouldQuickBuild", &mut [boolean(quick_build)]) }
	}

	/// Scales the bot's model, which also scales its size, or restores its
	/// usual scale with `None`. A Mann vs. Machine mini-boss uses the override
	/// in place of `tf_mvm_miniboss_scale` when it spawns. Panics unless a
	/// scale is finite and positive.
	#[doc(alias("SetScaleOverride"))]
	pub fn set_scale_override(self, scale: Option<f32>) -> Result<(), BotError> {
		let scale = match scale {
			Some(scale) => {
				assert!(scale.is_finite() && scale > 0.0, "invalid model scale");
				scale
			}

			// The bot then scales its model to 1 (`tf_bot.h:865-870`).
			None => -1.0,
		};

		// SAFETY: The method sets a field of the bot and its model scale.
		unsafe { self.void(c"SetScaleOverride", &mut [float(scale)]) }
	}

	/// Sets how far, from 0 in position to 1 completely out of it, the bot is
	/// from its place in its squad's formation. Panics unless the error is
	/// finite.
	#[doc(alias("SetSquadFormationError"))]
	pub fn set_squad_formation_error(self, error: f32) -> Result<(), BotError> {
		assert!(error.is_finite(), "invalid squad formation error");

		// SAFETY: The method sets a field of the bot.
		unsafe { self.void(c"SetSquadFormationError", &mut [float(error)]) }
	}

	/// How far, from 0 in position to 1 completely out of it, the bot is from
	/// its place in its squad's formation.
	#[doc(alias("GetSquadFormationError"))]
	pub fn squad_formation_error(self) -> Result<f32, BotError> {
		// SAFETY: The method reads a field of the bot.
		unsafe { self.float(c"GetSquadFormationError", &mut []) }
	}

	/// Calls a native method returning nothing.
	///
	/// # Safety
	///
	/// As for [`Self::call`].
	unsafe fn void(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
	) -> Result<(), BotError> {
		// SAFETY: As the caller promises.
		unsafe { self.call(name, arguments, VOID) }.map(drop)
	}

	/// The bot's weapon restrictions, tested one named flag at a time.
	pub fn weapon_restrictions(self) -> Result<WeaponRestrictions, BotError> {
		let mut restrictions = WeaponRestrictions::empty();

		for flag in WeaponRestrictions::all().iter() {
			if self.has_any_weapon_restriction(flag)? {
				restrictions |= flag;
			}
		}

		Ok(restrictions)
	}
}

/// What `tf_bot_add` is asked to add: how many bots, or one bot with a name,
/// on which team, of which class and with which skill.
///
/// [`Self::command`] writes the command line, and [`add_tf_bots`] queues it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TfBotRequest {
	bots: Bots,
	team: Option<BotTeam>,
	class: Option<PlayerClass>,
	difficulty: Option<Difficulty>,
	quota_managed: bool,
}

impl TfBotRequest {
	/// Asks for `count` bots, which the game names, on a team and of classes
	/// it picks, with the skill `tf_bot_difficulty` sets, and not counted by
	/// the bot quota.
	pub const fn new(count: NonZeroU8) -> Self {
		Self {
			bots: Bots::Count(count),
			team: None,
			class: None,
			difficulty: None,
			quota_managed: false,
		}
	}

	/// Asks for one bot named `name`, otherwise as [`Self::new`] does. Fails
	/// with a [`BotNameError`] for a name `tf_bot_add` would not take as one:
	/// see the error's variants.
	pub fn named(name: &CStr) -> Result<Self, BotNameError> {
		check_name(name)?;

		Ok(Self {
			bots: Bots::Named(name.to_owned()),
			..Self::new(NonZeroU8::MIN)
		})
	}

	/// The class of the bots, or `None` for classes the game picks.
	pub const fn class(&self) -> Option<PlayerClass> {
		self.class
	}

	/// The command line that adds the bots, such as
	/// `tf_bot_add 2 blue heavyweapons expert noquota`, ending with a newline.
	/// A name is quoted, as one argument.
	pub fn command(&self) -> CString {
		let mut command = b"tf_bot_add".to_vec();

		match &self.bots {
			Bots::Count(count) => command.extend_from_slice(format!(" {count}").as_bytes()),

			// A name holds no quote and no control byte, so it is one argument
			// once quoted.
			Bots::Named(name) => {
				command.extend_from_slice(b" \"");
				command.extend_from_slice(name.to_bytes());
				command.push(b'"');
			}
		}

		let arguments = [
			self.team.map(BotTeam::argument),
			self.class.map(class_argument),
			self.difficulty.map(Difficulty::argument),
			(!self.quota_managed).then_some(c"noquota"),
		];

		for argument in arguments.into_iter().flatten() {
			command.push(b' ');
			command.extend_from_slice(argument.to_bytes());
		}

		command.push(b'\n');
		CString::new(command).expect("a request holds no NUL")
	}

	/// The number of bots asked for: 1 for a named bot.
	pub const fn count(&self) -> NonZeroU8 {
		match self.bots {
			Bots::Count(count) => count,
			Bots::Named(_) => NonZeroU8::MIN,
		}
	}

	/// The skill of the bots, or `None` for the skill `tf_bot_difficulty`
	/// sets.
	pub const fn difficulty(&self) -> Option<Difficulty> {
		self.difficulty
	}

	/// The bot's name, or `None` for bots the game names.
	pub fn name(&self) -> Option<&CStr> {
		match &self.bots {
			Bots::Count(_) => None,
			Bots::Named(name) => Some(name),
		}
	}

	/// Whether the bot quota counts the bots, which gives them
	/// [`BotAttributes::QUOTA_MANAGED`].
	pub const fn quota_managed(&self) -> bool {
		self.quota_managed
	}

	/// The team of the bots, or `None` for teams the game picks.
	pub const fn team(&self) -> Option<BotTeam> {
		self.team
	}

	/// Asks for bots of this class, or of classes the game picks with `None`.
	/// `tf_bot_force_class` overrides it.
	pub const fn with_class(mut self, class: Option<PlayerClass>) -> Self {
		self.class = class;
		self
	}

	/// Asks for bots with this skill, or the skill `tf_bot_difficulty` sets
	/// with `None`. Bots on a training level are always easy.
	pub const fn with_difficulty(mut self, difficulty: Option<Difficulty>) -> Self {
		self.difficulty = difficulty;
		self
	}

	/// Asks for bots the bot quota counts, or does not, as by default. The
	/// quota manager may kick bots it counts, as `tf_bot_quota` changes.
	pub const fn with_quota_managed(mut self, quota_managed: bool) -> Self {
		self.quota_managed = quota_managed;
		self
	}

	/// Asks for bots on this team, or on teams the game picks with `None`.
	pub const fn with_team(mut self, team: Option<BotTeam>) -> Self {
		self.team = team;
		self
	}
}

/// Queues `tf_bot_add` with the request's [`command`](TfBotRequest::command),
/// which the server runs from its command buffer, normally on the next frame.
/// The game adds the bots then, if the server has room for them.
#[doc(alias("tf_bot_add"))]
pub fn add_tf_bots(server: Server<'_>, request: &TfBotRequest) -> Result<(), BotError> {
	if server.game() != Game::TeamFortress2 {
		return Err(BotError::UnsupportedGame);
	}

	server.valve_engine()?.server_command(&request.command());
	Ok(())
}

/// Checks a name for [`TfBotRequest::named`].
fn check_name(name: &CStr) -> Result<(), BotNameError> {
	let bytes = name.to_bytes();

	if bytes.is_empty() {
		return Err(BotNameError::Empty);
	}

	if bytes.len() > MAX_NAME_LEN {
		return Err(BotNameError::TooLong { len: bytes.len() });
	}

	if let Some((index, &byte)) = bytes
		.iter()
		.enumerate()
		.find(|&(_, &byte)| byte < 0x20 || byte == 0x7f || byte == b'"' || byte == b';')
	{
		return Err(BotNameError::InvalidByte { index, byte });
	}

	// The arguments `tf_bot_add` tests before taking one as a name
	// (`game/server/tf/bot/tf_bot.cpp:338-369`).
	let reserved = PlayerClass::ALL
		.into_iter()
		.map(class_argument)
		.chain([c"red", c"blue", c"noquota"])
		.chain(Difficulty::ALL.map(Difficulty::argument))
		.any(|argument| argument.to_bytes().eq_ignore_ascii_case(bytes));

	if reserved || is_positive_for_atoi(bytes) {
		return Err(BotNameError::Reserved);
	}

	Ok(())
}

/// The argument `tf_bot_add` takes for a class: the class's name in
/// `TFPlayerClassData_t`, which it compares ignoring case.
const fn class_argument(class: PlayerClass) -> &'static CStr {
	match class {
		PlayerClass::Scout => c"scout",
		PlayerClass::Sniper => c"sniper",
		PlayerClass::Soldier => c"soldier",
		PlayerClass::Demoman => c"demoman",
		PlayerClass::Medic => c"medic",
		PlayerClass::Heavy => c"heavyweapons",
		PlayerClass::Pyro => c"pyro",
		PlayerClass::Spy => c"spy",
		PlayerClass::Engineer => c"engineer",
	}
}

/// Whether C's `atoi` reads `bytes` as a positive number: after optional
/// whitespace and an optional `+`, digits that are not all zeros. Overflow,
/// for which `atoi` is undefined, counts as positive, so such names are
/// refused too.
fn is_positive_for_atoi(bytes: &[u8]) -> bool {
	let mut rest = bytes
		.iter()
		.skip_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r'))
		.peekable();

	match rest.peek() {
		Some(b'+') => {
			rest.next();
		}

		// A negative number is never positive.
		Some(b'-') => return false,

		_ => {}
	}

	rest.take_while(|byte| byte.is_ascii_digit())
		.any(|&digit| digit != b'0')
}

/// Queues `tf_bot_kick` for every bot, or for those on one team, which kicks
/// them when the server runs its command buffer, normally on the next frame.
#[doc(alias("tf_bot_kick"))]
pub fn kick_tf_bots(server: Server<'_>, team: Option<BotTeam>) -> Result<(), BotError> {
	if server.game() != Game::TeamFortress2 {
		return Err(BotError::UnsupportedGame);
	}

	let command = match team {
		None => c"tf_bot_kick all\n",
		Some(BotTeam::Red) => c"tf_bot_kick red\n",
		Some(BotTeam::Blue) => c"tf_bot_kick blue\n",
	};

	server.valve_engine()?.server_command(command);
	Ok(())
}
