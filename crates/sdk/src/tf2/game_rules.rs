//! TF2's game rules (`CTFGameRules`), which run its rounds: the game rules
//! object and its networked variables, such as the round's state, and the
//! vtable of its class, to hook.
//!
//! The game creates one game rules object as each level loads, and deletes it
//! as the level shuts down, so get [`GameRules`] again in each callback.
//! Clients receive its variables through the `tf_gamerules` entity, of the
//! server class `CTFGameRulesProxy`, whose send table nests the game rules'
//! own tables, `DT_TeamplayRoundBasedRules` and `DT_TFGameRules`, behind
//! proxies that give the game rules object instead of the entity. A
//! [`NetProp`] of the entity's class therefore finds those variables
//! [relocated](NetPropError::Relocated). [`GameRules`] calls the proxies as
//! the engine does when it sends the entity, and reads and writes the
//! variables in the object they give.

#[cfg(test)]
#[path = "../tests/tf2/game_rules.rs"]
mod tests;

use crate::NotThreadSafe;

use crate::datatables::{
	NetProp, NetPropError, NetVar, PropKind, SendProp, SendTable, ServerClass, StandardSendProxies,
};

use crate::edicts::Edict;
use crate::entities::{Entity, EntityHandle};
use crate::inputs::{InputError, InputValue};
use crate::interfaces::ServerTools;
use crate::interfaces::ValveEngine;
use crate::tf2::ammo::AmmoType;
use crate::tf2::player::TfPlayer;
use crate::tf2::scoreboard::ScoringTeam;
use crate::{Game, InterfaceError, Server};
use sdk_raw::datatables::call_table_proxy;

use sdk_raw::tf2::game_rules::{
	GR_STATE_BETWEEN_RNDS, GR_STATE_BONUS, GR_STATE_GAME_OVER, GR_STATE_INIT, GR_STATE_PREGAME,
	GR_STATE_PREROUND, GR_STATE_RESTART, GR_STATE_RND_RUNNING, GR_STATE_STALEMATE,
	GR_STATE_STARTGAME, GR_STATE_TEAM_WIN, can_have_ammo, find_game_rules_vtable,
};

use sdk_raw::tf2::game_rules::{TEAM_ROLE_ATTACKERS, TEAM_ROLE_DEFENDERS, TEAM_ROLE_NONE};
use sdk_raw::util;
use std::ffi::{CStr, c_int, c_void};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The server class of the entity that networks the game rules.
const PROXY_CLASS: &CStr = c"CTFGameRulesProxy";

/// The class name of the entity that networks the game rules.
const PROXY_ENTITY: &CStr = c"tf_gamerules";

/// The send table of [`PROXY_CLASS`].
const PROXY_TABLE: &CStr = c"DT_TFGameRulesProxy";

/// The property of [`PROXY_TABLE`]'s base table nesting
/// `CTeamplayRoundBasedRules`' table, and that table.
const ROUND_RULES_DATA: (&CStr, &CStr) = (
	c"teamplayroundbased_gamerules_data",
	c"DT_TeamplayRoundBasedRules",
);

/// The property of [`PROXY_TABLE`] nesting `CTFGameRules`' table, and that
/// table.
const RULES_DATA: (&CStr, &CStr) = (c"tf_gamerules_data", c"DT_TFGameRules");

/// TF2's game rules object, as the proxies of its networked tables give it.
///
/// It stays valid for the callback scope `'s`, since the game only deletes it
/// as a level shuts down. Do not get it from callbacks the game runs while it
/// creates or deletes the game rules, such as entity callbacks during a
/// level's shutdown.
#[doc(alias("CTFGameRules", "TFGameRules"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameRules<'s> {
	/// The game rules as `CTFGameRules`, which [`Self::rules_table`]
	/// describes.
	rules: NonNull<c_void>,

	/// The game rules as `CTeamplayRoundBasedRules`, which
	/// [`Self::round_rules_table`] describes.
	round_rules: NonNull<c_void>,

	/// The `tf_gamerules` entity, whose proxies gave the game rules.
	proxy: Entity<'s>,

	/// The edict of [`Self::proxy`].
	proxy_edict: Edict<'s>,

	rules_table: SendTable<'s>,
	round_rules_table: SendTable<'s>,
	proxies: StandardSendProxies<'s>,
	_not_thread_safe: NotThreadSafe,
}

/// The inputs of the `tf_gamerules` entity, through which maps control the
/// game rules.
///
/// Each sends its input through `tools` as a map would, from the entity to
/// itself, and fails as [`ServerTools::accept_input`] does.
impl GameRules<'_> {
	/// Activates the team's King of the Hill clock, and pauses the other
	/// team's, as capturing the hill does (`SetRedKothClockActive`).
	#[doc(alias("SetRedKothClockActive", "SetBlueKothClockActive"))]
	pub fn activate_koth_clock(
		self,
		tools: ServerTools<'_>,
		team: ScoringTeam,
	) -> Result<(), InputError> {
		let input = match team {
			ScoringTeam::Red => c"SetRedKothClockActive",
			ScoringTeam::Blue => c"SetBlueKothClockActive",
		};

		self.input(tools, input, InputValue::Void)
	}

	/// Adds seconds to the team's respawn wave time, from `mp_respawnwavetime`
	/// if the level set none, down to no less than 0
	/// (`AddRedTeamRespawnWaveTime`).
	#[doc(alias("AddRedTeamRespawnWaveTime", "AddBlueTeamRespawnWaveTime"))]
	pub fn add_respawn_wave_time(
		self,
		tools: ServerTools<'_>,
		team: ScoringTeam,
		seconds: f32,
	) -> Result<(), InputError> {
		let input = match team {
			ScoringTeam::Red => c"AddRedTeamRespawnWaveTime",
			ScoringTeam::Blue => c"AddBlueTeamRespawnWaveTime",
		};

		self.input(tools, input, InputValue::Float(seconds))
	}

	/// Adds points to the team's score, as capturing the flag does
	/// (`AddRedTeamScore`). Negative points take some away.
	#[doc(alias("AddRedTeamScore", "AddBlueTeamScore"))]
	pub fn add_team_score(
		self,
		tools: ServerTools<'_>,
		team: ScoringTeam,
		points: c_int,
	) -> Result<(), InputError> {
		let input = match team {
			ScoringTeam::Red => c"AddRedTeamScore",
			ScoringTeam::Blue => c"AddBlueTeamScore",
		};

		self.input(tools, input, InputValue::Int(points))
	}

	/// Sends the input named `input` with `value` to the `tf_gamerules`
	/// entity, from itself.
	fn input(
		self,
		tools: ServerTools<'_>,
		input: &CStr,
		value: InputValue<'_>,
	) -> Result<(), InputError> {
		tools.accept_input(self.proxy, input, value, self.proxy, self.proxy)
	}

	/// Sets the team's respawn wave time, in seconds, no less than 0
	/// (`SetRedTeamRespawnWaveTime`), which the time each player waits to
	/// respawn scales from.
	#[doc(alias("SetRedTeamRespawnWaveTime", "SetBlueTeamRespawnWaveTime"))]
	pub fn set_respawn_wave_time(
		self,
		tools: ServerTools<'_>,
		team: ScoringTeam,
		seconds: f32,
	) -> Result<(), InputError> {
		let input = match team {
			ScoringTeam::Red => c"SetRedTeamRespawnWaveTime",
			ScoringTeam::Blue => c"SetBlueTeamRespawnWaveTime",
		};

		self.input(tools, input, InputValue::Float(seconds))
	}

	/// Sets the goal the team's players see as the round starts, a
	/// localization token such as `#koth_setup_goal` or plain text, of fewer
	/// than 256 bytes, or clears it with an empty string
	/// (`SetRedTeamGoalString`).
	#[doc(alias("SetRedTeamGoalString", "SetBlueTeamGoalString"))]
	pub fn set_team_goal(
		self,
		tools: ServerTools<'_>,
		team: ScoringTeam,
		goal: &CStr,
	) -> Result<(), InputError> {
		let input = match team {
			ScoringTeam::Red => c"SetRedTeamGoalString",
			ScoringTeam::Blue => c"SetBlueTeamGoalString",
		};

		self.input(tools, input, InputValue::String(goal))
	}

	/// Sets whether the team attacks or defends the objectives
	/// (`SetRedTeamRole`), which players' voice responses follow, and which
	/// automatic team assignment breaks ties with, in the attackers' favor.
	#[doc(alias("SetRedTeamRole", "SetBlueTeamRole"))]
	pub fn set_team_role(
		self,
		tools: ServerTools<'_>,
		team: ScoringTeam,
		role: TeamRole,
	) -> Result<(), InputError> {
		let input = match team {
			ScoringTeam::Red => c"SetRedTeamRole",
			ScoringTeam::Blue => c"SetBlueTeamRole",
		};

		self.input(tools, input, InputValue::Int(role.to_raw()))
	}
}

/// The round's progress, from the variables of `CTeamplayRoundBasedRules`.
impl<'s> GameRules<'s> {
	/// Whether the level has several payload carts per team
	/// (`m_bMultipleTrains`), as Payload Race levels with a
	/// `tf_logic_multiple_escort` do.
	#[doc(alias("m_bMultipleTrains", "HasMultipleTrains"))]
	pub fn has_multiple_trains(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bMultipleTrains")
	}

	/// Whether the round is in overtime (`m_bInOvertime`), as while a team
	/// still contests the last objective as time runs out.
	#[doc(alias("m_bInOvertime", "InOvertime"))]
	pub fn is_in_overtime(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bInOvertime")
	}

	/// Whether the game plays stopwatch rounds (`m_bStopWatch`), as tournament
	/// mode does on Attack/Defend levels.
	#[doc(alias("m_bStopWatch", "IsInStopWatch"))]
	pub fn is_stopwatch(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bStopWatch")
	}

	/// How many rounds were played on the level (`m_nRoundsPlayed`).
	#[doc(alias("m_nRoundsPlayed", "GetRoundsPlayed"))]
	pub fn rounds_played(self) -> Result<c_int, GameRulesError> {
		self.read(c"m_nRoundsPlayed")
	}

	/// Whether the teams switched sides for this round
	/// (`m_bSwitchedTeamsThisRound`).
	#[doc(alias("m_bSwitchedTeamsThisRound", "SwitchedTeamsThisRound"))]
	pub fn switched_teams_this_round(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bSwitchedTeamsThisRound")
	}

	/// The team that won the last round won (`m_iWinningTeam`), or `None` when
	/// none has, as before the level's first win, and after a stalemate.
	#[doc(alias("m_iWinningTeam", "GetWinningTeam"))]
	pub fn winning_team(self) -> Result<Option<ScoringTeam>, GameRulesError> {
		Ok(ScoringTeam::from_raw(self.read(c"m_iWinningTeam")?))
	}
}

impl<'s> GameRules<'s> {
	/// Finds the game rules through the `tf_gamerules` entity.
	///
	/// Fails if the server does not run TF2, if the game rules' send tables
	/// are not nested as TF2 nests them, if no `tf_gamerules` entity exists,
	/// as before a level's entities are created, or if its proxies give no
	/// game rules.
	pub fn get(server: Server<'s>) -> Result<Self, GameRulesError> {
		if server.game() != Game::TeamFortress2 {
			return Err(GameRulesError::WrongGame);
		}

		let dll = server.server_game_dll()?;
		let proxies = dll
			.standard_send_proxies()
			.ok_or(NetPropError::NoStandardProxies)?;

		let class = dll
			.server_class(PROXY_CLASS)
			.ok_or(GameRulesError::UnexpectedTables)?;

		let table = class
			.table()
			.filter(|table| table.name() == PROXY_TABLE)
			.ok_or(GameRulesError::UnexpectedTables)?;

		// The base class's table, `DT_TeamplayRoundBasedRulesProxy`, at the
		// start of the entity.
		let base = nested(table, c"baseclass")
			.filter(|&base| base.offset() == 0 && proxies.is_direct(base))
			.and_then(SendProp::data_table)
			.ok_or(GameRulesError::UnexpectedTables)?;

		let (rules_prop, rules_table) = relocating(table, RULES_DATA, proxies)?;
		let (round_rules_prop, round_rules_table) = relocating(base, ROUND_RULES_DATA, proxies)?;
		let entity = proxy_entity(server, class)?;
		let edict = entity.edict().ok_or(GameRulesError::NoProxyEntity)?;
		let object = entity.as_ptr().cast::<c_void>().cast_const();

		let call = |prop: SendProp<'_>| {
			// SAFETY: The property nests one of the game rules' tables at offset 0
			// of the live proxy entity, whose class's table holds it directly or
			// through its base class's table at offset 0, so the entity is both
			// the structure holding the property and the nested structure, and its
			// edict index is its object ID. This runs on the main thread
			// (`Server::new`).
			let rules = unsafe { call_table_proxy(prop.as_ptr(), object, object, edict.index()) };

			rules
				.and_then(NonNull::new)
				.ok_or(GameRulesError::NoGameRules)
		};

		Ok(Self {
			rules: call(rules_prop)?,
			round_rules: call(round_rules_prop)?,
			proxy: entity,
			proxy_edict: edict,
			rules_table,
			round_rules_table,
			proxies,
			_not_thread_safe: PhantomData,
		})
	}

	/// The game rules object, a `CTFGameRules`, which starts with its primary
	/// vtable.
	pub const fn as_ptr(self) -> *mut c_void {
		self.rules.as_ptr()
	}

	/// The game rules object, as [`Self::as_ptr`] gives it.
	pub(crate) const fn as_non_null(self) -> NonNull<c_void> {
		self.rules
	}

	/// Whether the round is in its setup time, before attackers may leave
	/// their spawn.
	#[doc(alias("m_bInSetup"))]
	pub fn is_in_setup(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bInSetup")
	}

	/// Whether the game is waiting for players before its first round, as
	/// `mp_waitingforplayers_time` makes it.
	#[doc(alias("m_bInWaitingForPlayers"))]
	pub fn is_waiting_for_players(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bInWaitingForPlayers")
	}

	/// Records that the game rules' networked variables changed, so the engine
	/// compares all of them, and sends clients those that differ.
	///
	/// The game does this for every change of its own, as
	/// `CGameRules::NetworkStateChanged`, through the `tf_gamerules` entity, which
	/// the engine sends the game rules with.
	#[doc(alias("NetworkStateChanged", "NotifyNetworkStateChanged"))]
	pub fn network_state_changed(self, engine: ValveEngine<'_>) {
		self.proxy_edict.full_state_changed(engine);
	}

	/// The `tf_gamerules` entity, of the server class `CTFGameRulesProxy`, which
	/// networks the game rules, and takes the inputs maps send them, such as
	/// `SetRedTeamRespawnWaveTime`.
	#[doc(alias("CTFGameRulesProxy", "tf_gamerules"))]
	pub const fn proxy(self) -> Entity<'s> {
		self.proxy
	}

	/// Reads a networked variable of the game rules by name, as stored, such
	/// as `m_nRoundsPlayed`, from `DT_TeamplayRoundBasedRules`, or else from
	/// `DT_TFGameRules`, searching each table and the tables nested within it
	/// depth first.
	///
	/// Fails if neither table has the variable, if it cannot be addressed, or
	/// if it is not a single value stored
	/// [compatibly](crate::datatables::Storage::is_compatible) with `T`.
	pub fn read<T: NetVar>(self, name: &CStr) -> Result<T, GameRulesError> {
		let (prop, object) = self.variable(name)?;

		// SAFETY: The object is the one the proxy of the table's property gave,
		// so the table describes it, and the game only deletes it as the level
		// shuts down, after `'s`. The game initializes the variables it
		// networks, which the engine reads to send them.
		Ok(unsafe { prop.get_at(object) }?)
	}

	/// Reads an entity handle variable of the game rules by name, as stored,
	/// such as `m_hRedKothTimer`, found as [`Self::read`] finds variables, or
	/// `None` for an invalid handle.
	///
	/// Fails as [`Self::read`] does, or with [`NetPropError::NotAHandle`] if the
	/// variable is not declared as `SendPropEHandle` declares handles.
	#[doc(alias("SendPropEHandle", "CHandle", "EHANDLE"))]
	pub fn read_handle(self, name: &CStr) -> Result<Option<EntityHandle>, GameRulesError> {
		let (prop, object) = self.variable(name)?;

		// SAFETY: As for `read`.
		let handle = unsafe { prop.get_handle_at(object) }?;

		Ok(handle.is_valid().then_some(handle))
	}

	/// The state of the round (`m_iRoundState`).
	///
	/// Fails with [`GameRulesError::UnknownRoundState`] for a value
	/// [`RoundState`] does not know.
	#[doc(alias("m_iRoundState", "State_Get"))]
	pub fn round_state(self) -> Result<RoundState, GameRulesError> {
		let raw = self.read::<c_int>(c"m_iRoundState")?;

		RoundState::from_raw(raw).ok_or(GameRulesError::UnknownRoundState(raw))
	}

	/// Writes a networked variable of the game rules by name, as [`Self::write`]
	/// does, and records the change, as [`Self::network_state_changed`] does, so
	/// clients receive it.
	///
	/// Fails, without writing, as [`Self::read`] does.
	///
	/// # Safety
	///
	/// As for [`Self::write`].
	pub unsafe fn set<T: NetVar>(
		self,
		engine: ValveEngine<'_>,
		name: &CStr,
		value: T,
	) -> Result<(), GameRulesError> {
		// SAFETY: The caller upholds `write`'s contract.
		unsafe { self.write(name, value) }?;
		self.network_state_changed(engine);
		Ok(())
	}

	/// The networked variable named `name`, found as [`Self::read`] finds it,
	/// and the object holding it.
	fn variable(self, name: &CStr) -> Result<(NetProp<'s>, NonNull<c_void>), GameRulesError> {
		match NetProp::resolve(self.round_rules_table, name, self.proxies) {
			Err(NetPropError::NotFound { .. }) => Ok((
				NetProp::resolve(self.rules_table, name, self.proxies)?,
				self.rules,
			)),

			found => Ok((found?, self.round_rules)),
		}
	}

	/// Writes a networked variable of the game rules by name, as stored, where
	/// [`Self::read`] reads it, without recording the change for clients.
	///
	/// The engine sends the game rules to clients through the `tf_gamerules`
	/// entity, which it only packs again once the game records a change of the
	/// game rules' variables. A written value reaches clients only if it is still
	/// there then, so one put back before the game's next change never does.
	///
	/// Fails, without writing, as [`Self::read`] does.
	///
	/// # Safety
	///
	/// The game must accept `value` for this variable for as long as it holds
	/// it. Game code trusts the game rules to hold values it could have assigned
	/// itself, and assigns some variables together with other state, which a
	/// write leaves as it was. `m_bInWaitingForPlayers` written true outside the
	/// game's own waiting for players, whose end it sets as it starts, makes the
	/// game restart the round at its next check of the waiting.
	pub unsafe fn write<T: NetVar>(self, name: &CStr, value: T) -> Result<(), GameRulesError> {
		let (prop, object) = self.variable(name)?;

		// SAFETY: As for `read`, and the game accepts the value, as the caller
		// promises.
		Ok(unsafe { prop.set_at(object, value) }?)
	}
}

/// What the game rules decide for players.
impl GameRules<'_> {
	/// Whether `player` holds less reserve ammo of `ammo_type` than their max,
	/// as their class and items make it (`CanHaveAmmo`), which pickups,
	/// dispensers and resupply cabinets ask before giving any.
	#[doc(alias("CanHaveAmmo", "GetMaxAmmo"))]
	pub fn can_have_ammo(self, player: TfPlayer<'_>, ammo_type: AmmoType) -> bool {
		// SAFETY: An entity's pointer is never null.
		let player = unsafe { NonNull::new_unchecked(player.player().as_ptr()) };

		// SAFETY: The game rules are TF2's live `CTFGameRules`, as `GameRules::get`
		// checks the game, and the player is a live `CTFPlayer`, as `TfPlayer`
		// checks, whose `CBaseCombatCharacter` base is at its address. Both are
		// only used on the main thread.
		unsafe { can_have_ammo(self.rules, player.cast(), ammo_type.to_raw()) }
	}
}

/// Why TF2's game rules could not be found, read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GameRulesError {
	/// The server does not run TF2.
	#[error("game rules access requires Team Fortress 2")]
	WrongGame,

	/// The game does not export an interface the game rules are found
	/// through.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The game's networked classes do not nest the game rules' send tables as
	/// TF2's do.
	#[error("the game rules' send tables are not nested as TF2 nests them")]
	UnexpectedTables,

	/// No `tf_gamerules` entity exists, as before a level's entities are
	/// created.
	#[error("no `tf_gamerules` entity exists")]
	NoProxyEntity,

	/// The `tf_gamerules` entity's proxies gave no game rules.
	#[error("the game has no game rules object")]
	NoGameRules,

	/// A networked variable could not be found, read or written.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// The round's state is a value [`RoundState`] does not know.
	#[error("the game rules hold an unknown round state, {0}")]
	UnknownRoundState(c_int),

	/// A networked variable holds a value its type does not know, such as a
	/// game type a later update added.
	#[error("the game rules' `{}` holds an unknown value, {value}", variable.to_string_lossy())]
	UnknownValue {
		/// The variable's name, such as `m_nGameType`.
		variable: &'static CStr,

		/// The value it holds.
		value: c_int,
	},
}

/// The vtable of TF2's game rules class, `CTFGameRules`, in the game server
/// module, which [`game_rules_vtable`] finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GameRulesVtable<'s> {
	vtable: NonNull<*mut c_void>,
	_scope: PhantomData<&'s Server<'s>>,
}

impl GameRulesVtable<'_> {
	/// The vtable's address in the game module, which stays loaded until the
	/// server shuts down.
	pub const fn as_ptr(self) -> NonNull<*mut c_void> {
		self.vtable
	}
}

/// Why the vtable of TF2's game rules class could not be found.
#[derive(Debug, thiserror::Error)]
pub enum GameRulesVtableError {
	/// The server does not run TF2.
	#[error("the game rules vtable is only known for Team Fortress 2")]
	WrongGame,

	/// The game module could not be read.
	#[error("the game module could not be inspected")]
	Image(#[from] std::io::Error),

	/// The game module is not an executable image the vtable search supports.
	#[error("the game module has an unsupported executable image")]
	InvalidImage,

	/// The game module's run-time type information has no unique primary
	/// vtable of `CTFGameRules` with a function at `CleanUpMap`'s slot, or, on
	/// Linux, the module's symbols name another function there.
	#[error("the game module has no unique vtable of `CTFGameRules`")]
	NotFound,
}

impl From<util::Error> for GameRulesVtableError {
	fn from(error: util::Error) -> Self {
		match error {
			util::Error::InvalidImage => Self::InvalidImage,
			util::Error::Io(error) => Self::Image(error),
		}
	}
}

/// The state of TF2's round, as the game rules track it
/// (`gamerules_roundstate_t`).
#[doc(alias("gamerules_roundstate_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum RoundState {
	/// The game rules were just created.
	#[doc(alias("GR_STATE_INIT"))]
	Init = GR_STATE_INIT,

	/// No player is ready yet, so the game has not started.
	#[doc(alias("GR_STATE_PREGAME"))]
	Pregame = GR_STATE_PREGAME,

	/// Players are ready, and the first round is set up a tick later.
	#[doc(alias("GR_STATE_STARTGAME"))]
	StartGame = GR_STATE_STARTGAME,

	/// A round was set up, and players wait to move.
	#[doc(alias("GR_STATE_PREROUND"))]
	Preround = GR_STATE_PREROUND,

	/// A round is being played.
	#[doc(alias("GR_STATE_RND_RUNNING"))]
	Running = GR_STATE_RND_RUNNING,

	/// A team won the round.
	#[doc(alias("GR_STATE_TEAM_WIN"))]
	TeamWin = GR_STATE_TEAM_WIN,

	/// The round is restarting.
	#[doc(alias("GR_STATE_RESTART"))]
	Restart = GR_STATE_RESTART,

	/// Sudden death, or an arena round.
	#[doc(alias("GR_STATE_STALEMATE"))]
	Stalemate = GR_STATE_STALEMATE,

	/// The game ended, as the map is about to change.
	#[doc(alias("GR_STATE_GAME_OVER"))]
	GameOver = GR_STATE_GAME_OVER,

	/// A bonus round.
	#[doc(alias("GR_STATE_BONUS"))]
	Bonus = GR_STATE_BONUS,

	/// Players ready up, between Mann vs. Machine's waves or before a
	/// matchmade game.
	#[doc(alias("GR_STATE_BETWEEN_RNDS"))]
	BetweenRounds = GR_STATE_BETWEEN_RNDS,
}

impl RoundState {
	/// Every state, in the game's order.
	pub const ALL: [Self; 11] = [
		Self::Init,
		Self::Pregame,
		Self::StartGame,
		Self::Preround,
		Self::Running,
		Self::TeamWin,
		Self::Restart,
		Self::Stalemate,
		Self::GameOver,
		Self::Bonus,
		Self::BetweenRounds,
	];

	/// The state the game stores as `raw`, or `None` for any other value.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			GR_STATE_INIT => Some(Self::Init),
			GR_STATE_PREGAME => Some(Self::Pregame),
			GR_STATE_STARTGAME => Some(Self::StartGame),
			GR_STATE_PREROUND => Some(Self::Preround),
			GR_STATE_RND_RUNNING => Some(Self::Running),
			GR_STATE_TEAM_WIN => Some(Self::TeamWin),
			GR_STATE_RESTART => Some(Self::Restart),
			GR_STATE_STALEMATE => Some(Self::Stalemate),
			GR_STATE_GAME_OVER => Some(Self::GameOver),
			GR_STATE_BONUS => Some(Self::Bonus),
			GR_STATE_BETWEEN_RNDS => Some(Self::BetweenRounds),
			_ => None,
		}
	}

	/// The value the game stores for the state, as `m_iRoundState` holds it.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

/// Whether a team attacks or defends the objectives, as Attack/Defend levels
/// set it through their `tf_gamerules` entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum TeamRole {
	/// Neither, as on symmetric levels.
	#[doc(alias("TEAM_ROLE_NONE"))]
	None = TEAM_ROLE_NONE,

	/// The team defends the objectives.
	#[doc(alias("TEAM_ROLE_DEFENDERS"))]
	Defenders = TEAM_ROLE_DEFENDERS,

	/// The team attacks the objectives.
	#[doc(alias("TEAM_ROLE_ATTACKERS"))]
	Attackers = TEAM_ROLE_ATTACKERS,
}

impl TeamRole {
	/// The value the game stores for the role.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

/// Finds the vtable of TF2's game rules class, `CTFGameRules`, through the
/// game server module's run-time type information, which needs no game rules
/// object, so no level needs to be loaded.
///
/// Snapshots the whole module, so call it once, such as while loading.
pub fn game_rules_vtable(server: Server<'_>) -> Result<GameRulesVtable<'_>, GameRulesVtableError> {
	if server.game() != Game::TeamFortress2 {
		return Err(GameRulesVtableError::WrongGame);
	}

	// SAFETY: The game server factory is the game module's `CreateInterface`,
	// and the server's callback scope keeps the module loaded while it is
	// inspected (`Server::new` condition 1).
	let vtable = unsafe { find_game_rules_vtable(server.game_server_factory().as_raw()) }?
		.ok_or(GameRulesVtableError::NotFound)?;

	Ok(GameRulesVtable {
		vtable,
		_scope: PhantomData,
	})
}

/// The property of `table` itself named `name` that nests a table.
fn nested<'s>(table: SendTable<'s>, name: &CStr) -> Option<SendProp<'s>> {
	table
		.props()
		.find(|prop| prop.kind() == PropKind::DataTable && prop.name() == name)
}

/// The live `tf_gamerules` entity of `class`, skipping one a map's own is
/// replacing.
fn proxy_entity<'s>(
	server: Server<'s>,
	class: ServerClass<'s>,
) -> Result<Entity<'s>, GameRulesError> {
	let tools = server.server_tools()?;

	std::iter::successors(tools.find_by_class_name(None, PROXY_ENTITY), |&entity| {
		tools.find_by_class_name(Some(entity), PROXY_ENTITY)
	})
	.find(|entity| entity.server_class() == Some(class) && !entity.is_marked_for_deletion())
	.ok_or(GameRulesError::NoProxyEntity)
}

/// The property of `table` named `data.0` nesting the table named `data.1`
/// at offset 0 through a proxy that relocates it, and that table.
fn relocating<'s>(
	table: SendTable<'s>,
	(prop, nested_table): (&CStr, &CStr),
	proxies: StandardSendProxies<'s>,
) -> Result<(SendProp<'s>, SendTable<'s>), GameRulesError> {
	nested(table, prop)
		.filter(|&prop| prop.offset() == 0 && !proxies.is_direct(prop))
		.and_then(|prop| Some((prop, prop.data_table()?)))
		.filter(|(_, table)| table.name() == nested_table)
		.ok_or(GameRulesError::UnexpectedTables)
}
