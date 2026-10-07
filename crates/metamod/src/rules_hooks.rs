//! TF2 game rules hooks, which [`MetamodApi::hook_rules`] installs on the
//! game rules' decisions that are not about a round's course: whether the
//! teams are kept balanced, switched and scrambled, whether a holiday is
//! active, and whether a player takes an attacker's damage.
//!
//! - [`RulesCallbacks::balance_teams`] runs before
//!   `CTFGameRules::ShouldBalanceTeams`, which the game asks before it
//!   refuses a player a team that would leave the teams more than
//!   `mp_teams_unbalance_limit` players apart, before it balances them under
//!   `mp_autoteambalance 1` and warns that it will, as it adds bots for
//!   `tf_bot_quota`, and as a player joins the smaller team, whom the default
//!   game mode then respawns at once. The game answers no in tournaments,
//!   matchmade and competitive games, Mann vs. Machine and while players are
//!   in hell, and while `mp_teams_unbalance_limit` is 0. TF2's newer
//!   balancing, under `mp_autoteambalance 2`, checks the limit itself, and
//!   [`crate::autobalance_hooks`] decides whom it may move.
//! - [`RulesCallbacks::switch_teams`] runs before
//!   `CTFGameRules::ShouldSwitchTeams`, which the game asks as a round
//!   restarts, before it moves every player to the other team, as a win
//!   that switches teams ([`WinOptions::switch_teams`]) or `mp_switchteams`
//!   asked, as it counts that win's score for the other team after the
//!   switch, and as it announces a restart. The game answers no in Mann vs.
//!   Machine.
//! - [`RulesCallbacks::scramble_teams`] runs before
//!   `CTFGameRules::ShouldScrambleTeams`, which the game asks as a round
//!   restarts, before it scrambles the teams, as a vote, `mp_scrambleteams`
//!   or `mp_scrambleteams_auto` asked, as it announces a restart, and as a
//!   player calls a scramble vote, which they may not while a scramble is
//!   pending. The game answers no in Mann vs. Machine and competitive games.
//! - [`RulesCallbacks::holiday`] runs after `CTFGameRules::IsHolidayActive`,
//!   which the game asks as it decides what players drop, which items its
//!   holiday restrictions allow, the taunts, sounds and models of holidays,
//!   and the gifts dispensers give. Much of TF2 asks `TF_IsHolidayActive`
//!   directly instead, which the hook does not reach: rockets, grenades,
//!   flamethrowers and bottles, the styles of items, parts of players and of
//!   the game rules, and `tf_logic_on_holiday`. Clients decide holidays for
//!   themselves too, so `tf_forced_holiday` remains the way to force one
//!   everywhere.
//! - [`RulesCallbacks::player_damage`] runs after
//!   `CTFGameRules::FPlayerCanTakeDamage`, which a player asks as they take
//!   damage, before any of it is dealt. The game refuses teammates' damage,
//!   other than a player's own, while `mp_friendlyfire` is off, enemies'
//!   during a truce, and a building's to anyone but its builder and enemies.
//!   Many of TF2's attacks never reach teammates, whose traces and
//!   projectiles pass through them, so allowing teammates' damage does not
//!   make every weapon hurt them.
//!
//! The hooks patch the methods in the primary vtable of `CTFGameRules`, as
//! [`crate::round_hooks`] describes: install them once, such as while
//! loading.
//!
//! [`WinOptions::switch_teams`]: source_sdk_2013::tf2::round_end::WinOptions::switch_teams

#[cfg(test)]
#[path = "tests/rules_hooks.rs"]
mod tests;

use crate::MetamodApi;
use crate::hook::{Handler, HookAction, HookCall, HookError, HookId, HookTiming, VirtualFunction};
use crate::round_hooks::{RoundHookError, RoundRoute};
use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::game_rules::{
	IS_HOLIDAY_ACTIVE_SLOT, IsHolidayActiveFn as IsHolidayActive, PLAYER_CAN_TAKE_DAMAGE_SLOT,
	PlayerCanTakeDamageFn as PlayerCanTakeDamage, SHOULD_BALANCE_TEAMS_SLOT,
	SHOULD_SCRAMBLE_TEAMS_SLOT, SHOULD_SWITCH_TEAMS_SLOT,
	ShouldBalanceTeamsFn as ShouldBalanceTeams, ShouldScrambleTeamsFn as ShouldScrambleTeams,
	ShouldSwitchTeamsFn as ShouldSwitchTeams,
};

use source_sdk_2013::tf2::damage::DamageInfo;
use source_sdk_2013::tf2::game_mode::Holiday;
use source_sdk_2013::tf2::game_rules::{GameRulesVtableError, game_rules_vtable};
use source_sdk_2013::{Game, Server, ServerBinding};
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

/// A callback-scoped server, a holiday the game asks about, and whether the
/// game, or an earlier hook, found it active, returning whether it is active
/// instead, if anything. A panic is contained by the hook dispatcher, and
/// keeps the answer.
pub type HolidayFn = for<'s> fn(Server<'s>, Holiday, bool) -> Option<bool>;

/// A callback-scoped server, and a player's damage the game asks about,
/// returning whether the player takes it instead of what the check says, if
/// anything. A panic is contained by the hook dispatcher, and keeps the
/// answer.
pub type PlayerDamageFn = for<'s> fn(Server<'s>, PlayerDamageCheck<'s>) -> Option<bool>;

/// A callback-scoped server whose game asks whether it balances, switches or
/// scrambles the teams, which decides whether it may. A panic is contained
/// by the hook dispatcher, and lets the game decide.
pub type TeamsFn = for<'s> fn(Server<'s>) -> TeamsAction;

/// `IsHolidayActive` in TF2's game rules' primary vtable.
const IS_HOLIDAY_ACTIVE: VirtualFunction<IsHolidayActive> =
	VirtualFunction::new(IS_HOLIDAY_ACTIVE_SLOT);

/// `FPlayerCanTakeDamage` in TF2's game rules' primary vtable.
const PLAYER_CAN_TAKE_DAMAGE: VirtualFunction<PlayerCanTakeDamage> =
	VirtualFunction::new(PLAYER_CAN_TAKE_DAMAGE_SLOT);

/// `ShouldBalanceTeams` in TF2's game rules' primary vtable.
const SHOULD_BALANCE_TEAMS: VirtualFunction<ShouldBalanceTeams> =
	VirtualFunction::new(SHOULD_BALANCE_TEAMS_SLOT);

/// `ShouldScrambleTeams` in TF2's game rules' primary vtable.
const SHOULD_SCRAMBLE_TEAMS: VirtualFunction<ShouldScrambleTeams> =
	VirtualFunction::new(SHOULD_SCRAMBLE_TEAMS_SLOT);

/// `ShouldSwitchTeams` in TF2's game rules' primary vtable.
const SHOULD_SWITCH_TEAMS: VirtualFunction<ShouldSwitchTeams> =
	VirtualFunction::new(SHOULD_SWITCH_TEAMS_SLOT);

static BALANCE_ROUTE: RoundRoute<TeamsFn> = RoundRoute::new();
static HOLIDAY_ROUTE: RoundRoute<HolidayFn> = RoundRoute::new();
static PLAYER_DAMAGE_ROUTE: RoundRoute<PlayerDamageFn> = RoundRoute::new();
static SCRAMBLE_ROUTE: RoundRoute<TeamsFn> = RoundRoute::new();
static SWITCH_ROUTE: RoundRoute<TeamsFn> = RoundRoute::new();

/// A player's damage the game asks about, as a player is about to take it
/// (`FPlayerCanTakeDamage`).
#[derive(Debug)]
pub struct PlayerDamageCheck<'s> {
	/// The player taking the damage.
	pub player: Entity<'s>,

	/// The entity dealing it, such as a player, a building, or the world.
	pub attacker: Entity<'s>,

	/// An owned copy of the damage's arguments.
	pub info: DamageInfo,

	/// Whether the player takes the damage, as the game, or an earlier hook,
	/// decided.
	pub takes_damage: bool,
}

/// The game rules decisions to hook, each run where the
/// [module documentation](self) says, and hooked only if given.
#[derive(Debug, Clone, Copy, Default)]
pub struct RulesCallbacks {
	/// Runs before the game asks whether it keeps the teams balanced, and
	/// decides whether it may.
	pub balance_teams: Option<TeamsFn>,

	/// Runs after the game rules decide whether a holiday is active, and may
	/// decide otherwise.
	pub holiday: Option<HolidayFn>,

	/// Runs after the game rules decide whether a player takes an attacker's
	/// damage, and may decide otherwise.
	pub player_damage: Option<PlayerDamageFn>,

	/// Runs before the game asks whether it scrambles the teams, and decides
	/// whether it may.
	pub scramble_teams: Option<TeamsFn>,

	/// Runs before the game asks whether the teams switch sides, and decides
	/// whether they may.
	pub switch_teams: Option<TeamsFn>,
}

/// Handles for the game rules hooks installed by one call.
///
/// Metamod disables them while paused and removes them before unloading the
/// plugin. [`Self::remove`] disables them earlier. No game rules object is
/// retained, so the hooks last through level changes.
#[must_use = "retain game rules hooks to support explicitly removing them"]
#[derive(Debug)]
pub struct RulesHooks {
	hooks: Vec<HookId>,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl RulesHooks {
	pub fn remove(self, api: MetamodApi<'_>) {
		for hook in self.hooks {
			api.remove_hook(hook);
			BALANCE_ROUTE.clear(hook);
			HOLIDAY_ROUTE.clear(hook);
			PLAYER_DAMAGE_ROUTE.clear(hook);
			SCRAMBLE_ROUTE.clear(hook);
			SWITCH_ROUTE.clear(hook);
		}
	}
}

impl Handler<IsHolidayActive> for RoundRoute<HolidayFn> {
	fn call(&self, call: &HookCall<'_, IsHolidayActive>) -> HookAction<bool> {
		let Some(holiday) = Holiday::from_raw(call.args().0) else {
			return HookAction::Ignore;
		};

		let scope = ();
		let Some((callback, server)) = self.enter(&scope) else {
			return HookAction::Ignore;
		};
		let active = call.return_value().unwrap_or_default();

		match callback(server, holiday, active) {
			Some(active) => HookAction::Override(active),
			None => HookAction::Ignore,
		}
	}
}

impl Handler<PlayerCanTakeDamage> for RoundRoute<PlayerDamageFn> {
	fn call(&self, call: &HookCall<'_, PlayerCanTakeDamage>) -> HookAction<bool> {
		let (player, attacker, info) = call.args();

		// The game refuses damage without either.
		let (Some(player), Some(attacker), Some(info)) = (
			NonNull::new(player),
			NonNull::new(attacker),
			NonNull::new(info.cast_mut()),
		) else {
			return HookAction::Ignore;
		};

		let scope = ();
		let Some((callback, server)) = self.enter(&scope) else {
			return HookAction::Ignore;
		};
		let check = PlayerDamageCheck {
			// SAFETY: The game passes a live player and attacker, which stay in
			// the entity list through the call, and whose `CBaseEntity` bases
			// are at their addresses.
			player: unsafe { Entity::from_live(server, player.cast()) },
			attacker: unsafe { Entity::from_live(server, attacker) },
			// SAFETY: The game passes the live damage record it is dealing.
			info: unsafe { DamageInfo::copy_from_raw(info) },
			takes_damage: call.return_value().unwrap_or_default(),
		};

		match callback(server, check) {
			Some(takes_damage) => HookAction::Override(takes_damage),
			None => HookAction::Ignore,
		}
	}
}

// `ShouldScrambleTeams` and `ShouldSwitchTeams` have the same signature, so
// their routes take this handler too.
impl Handler<ShouldBalanceTeams> for RoundRoute<TeamsFn> {
	fn call(&self, call: &HookCall<'_, ShouldBalanceTeams>) -> HookAction<bool> {
		// An earlier hook already answered.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let scope = ();
		let Some((callback, server)) = self.enter(&scope) else {
			return HookAction::Ignore;
		};

		match callback(server) {
			TeamsAction::Allow => HookAction::Ignore,
			TeamsAction::Refuse => HookAction::Supersede(false),
		}
	}
}

/// What a team hook does with the game's question whether it balances,
/// switches or scrambles the teams.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TeamsAction {
	/// Lets the game decide, through the other plugins' hooks.
	#[default]
	Allow,

	/// Answers no. For balance, players may join any team, and the teams count
	/// as balanced whatever their sizes; for a switch or a scramble, the teams
	/// stay as they are, and the switch or scramble stays pending.
	Refuse,
}

impl MetamodApi<'_> {
	/// Runs `callbacks` as TF2's game rules decide whether they keep the teams
	/// balanced, switch or scramble them, whether a holiday is active, and whether a player takes an
	/// attacker's damage, hooking only the methods `callbacks` names a
	/// callback for; see the [module documentation](crate::rules_hooks).
	///
	/// `binding` must describe the same running server as `server`. Finds the
	/// game rules class in the game server module, as
	/// [`Self::hook_rounds`] does, which needs no level to be loaded, and
	/// snapshots the module to do so: install once, such as while loading. A
	/// refused hook rolls back the others. Installing again while any of the
	/// hooks is installed returns [`HookError::AlreadyInstalled`] without
	/// searching the module again; removing them allows replacement. Without
	/// any callback, nothing is searched for or hooked.
	///
	/// The game asks often, such as for each hit and each item a player
	/// equips, so the callbacks should be cheap. They must not delete
	/// entities immediately, as [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload. A
	/// returned handle means registration was accepted, not that the next call
	/// will be intercepted.
	pub fn hook_rules(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callbacks: RulesCallbacks,
	) -> Result<RulesHooks, RoundHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(GameRulesVtableError::WrongGame.into());
		}

		// The class's vtable is the same for the whole load.
		if self.rules_hooked() {
			return Err(HookError::AlreadyInstalled.into());
		}

		let none = callbacks.balance_teams.is_none()
			&& callbacks.holiday.is_none()
			&& callbacks.player_damage.is_none()
			&& callbacks.scramble_teams.is_none()
			&& callbacks.switch_teams.is_none();

		if none {
			return Ok(RulesHooks {
				hooks: Vec::new(),
				_not_thread_safe: PhantomData,
			});
		}

		let vtable = game_rules_vtable(server)?;

		// SAFETY: Run-time type information identified `CTFGameRules`' primary
		// vtable in the game module, which holds the methods at the generated
		// binding's slots, and outlives the plugin.
		Ok(unsafe { self.install_rules(vtable.as_ptr(), binding, callbacks) }?)
	}

	/// Hooks the methods `callbacks` names a callback for on the class whose
	/// primary vtable is `vtable`, all or none.
	///
	/// # Safety
	///
	/// `vtable` must be live, hold functions of the signatures
	/// [`IsHolidayActive`], [`PlayerCanTakeDamage`], [`ShouldBalanceTeams`],
	/// [`ShouldScrambleTeams`] and [`ShouldSwitchTeams`] at the slots of
	/// `CTFGameRules`' methods `IsHolidayActive`, `FPlayerCanTakeDamage`,
	/// `ShouldBalanceTeams`, `ShouldScrambleTeams` and `ShouldSwitchTeams`,
	/// called on game rules objects, and stay loaded until Metamod unloads
	/// the plugin.
	unsafe fn install_rules(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callbacks: RulesCallbacks,
	) -> Result<RulesHooks, HookError> {
		if self.rules_hooked() {
			return Err(HookError::AlreadyInstalled);
		}

		let mut installed = RulesHooks {
			hooks: Vec::new(),
			_not_thread_safe: PhantomData,
		};

		// SAFETY: As the caller promises.
		match unsafe { self.route_rules(vtable, binding, callbacks, &mut installed.hooks) } {
			Ok(()) => Ok(installed),

			Err(error) => {
				installed.remove(self);
				Err(error)
			}
		}
	}

	/// Hooks the methods `callbacks` names a callback for on the class whose
	/// primary vtable is `vtable`, and adds the hooks to `hooks`, as far as
	/// Metamod accepts them.
	///
	/// # Safety
	///
	/// As for [`Self::install_rules`].
	unsafe fn route_rules(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callbacks: RulesCallbacks,
		hooks: &mut Vec<HookId>,
	) -> Result<(), HookError> {
		let RulesCallbacks {
			balance_teams,
			holiday,
			player_damage,
			scramble_teams,
			switch_teams,
		} = callbacks;

		// SAFETY: As the caller promises, the vtable has each method at its
		// slot, called on game rules objects, and stays loaded.
		unsafe {
			if let Some(callback) = balance_teams {
				self.route_round(
					SHOULD_BALANCE_TEAMS,
					HookTiming::Pre,
					&BALANCE_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = holiday {
				self.route_round(
					IS_HOLIDAY_ACTIVE,
					HookTiming::Post,
					&HOLIDAY_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = player_damage {
				self.route_round(
					PLAYER_CAN_TAKE_DAMAGE,
					HookTiming::Post,
					&PLAYER_DAMAGE_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = scramble_teams {
				self.route_round(
					SHOULD_SCRAMBLE_TEAMS,
					HookTiming::Pre,
					&SCRAMBLE_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = switch_teams {
				self.route_round(
					SHOULD_SWITCH_TEAMS,
					HookTiming::Pre,
					&SWITCH_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}
		}

		Ok(())
	}

	/// Whether any game rules hook is installed, for this load of the plugin.
	fn rules_hooked(self) -> bool {
		BALANCE_ROUTE.is_routed(self)
			|| HOLIDAY_ROUTE.is_routed(self)
			|| PLAYER_DAMAGE_ROUTE.is_routed(self)
			|| SCRAMBLE_ROUTE.is_routed(self)
			|| SWITCH_ROUTE.is_routed(self)
	}
}
