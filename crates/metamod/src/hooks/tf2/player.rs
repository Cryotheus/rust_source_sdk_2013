//! TF2 player hooks that run after the game's `CTFPlayer::Spawn`,
//! `CTFPlayer::ResetScores` or `CTFPlayer::TakeHealth`.
//!
//! Install for each distinct player class (for example as each player is put
//! in the server, before the game spawns it; bots have a vtable of their own).
//! Hooks cover that class, including subsequently connected players of the
//! same class, until removed or the plugin unloads. As with other Metamod
//! hooks, they stop calling handlers while the plugin is paused.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/player.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::entities::health::{TAKE_HEALTH_SLOT, TakeHealthFn as TakeHealth};
use source_sdk_2013::raw::entities::{SPAWN_SLOT, SpawnFn as PlayerMethod};
use source_sdk_2013::raw::tf2::scoreboard::{RESET_SCORES_SLOT, ResetScoresFn};
use source_sdk_2013::raw::util::vtable::vtable_pointer;
use source_sdk_2013::{Game, Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

/// A callback-scoped server, the player the game healed, and the health they
/// gained. A panic is contained by the hook dispatcher.
pub type HealedFn = for<'s> fn(Server<'s>, Entity<'s>, c_int);

/// A callback-scoped server and the player the game's method ran for. A
/// panic is contained by the hook dispatcher.
pub type PlayerFn = for<'s> fn(Server<'s>, Entity<'s>);

/// `ResetScores` in a TF2 player's primary vtable.
const RESET_SCORES: VirtualFunction<PlayerMethod> = VirtualFunction::new(RESET_SCORES_SLOT);

/// `Spawn` in a TF2 player's primary vtable.
const SPAWN: VirtualFunction<PlayerMethod> = VirtualFunction::new(SPAWN_SLOT);

/// `TakeHealth` in a TF2 player's primary vtable.
const TAKE_HEALTH: VirtualFunction<TakeHealth> = VirtualFunction::new(TAKE_HEALTH_SLOT);

static HEALED_ROUTES: [HealedRoute; 8] = [const { HealedRoute::new() }; 8];
static SCORES_RESET_ROUTES: [PlayerRoute; 8] = [const { PlayerRoute::new() }; 8];
static SPAWNED_ROUTES: [PlayerRoute; 8] = [const { PlayerRoute::new() }; 8];

// Both methods take no argument and return nothing, so one route type serves
// them.
const _: fn(ResetScoresFn) -> PlayerMethod = |method| method;

/// Why a player hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum PlayerHookError {
	/// The server does not run TF2, or the entity is not a TF2 player.
	#[error("player hooks require a TF2 server and a CTFPlayer entity")]
	NotTfPlayer,

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means this
	/// player's class is already hooked, which callers installing on every
	/// player can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct HealedRoute {
	state: Cell<Option<RoutedHealed>>,
}

impl HealedRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<TakeHealth> for HealedRoute {
	fn call(&self, call: &HookCall<'_, TakeHealth>) -> HookAction<c_int> {
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (Some(player), Some(gained)) = (NonNull::new(call.this()), call.return_value()) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies the live player who was healed, who
		// stays in the entity list through the call.
		let player = unsafe { Entity::from_live(server, player) };

		(route.callback)(server, player, gained);
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for HealedRoute {}

struct PlayerRoute {
	state: Cell<Option<RoutedPlayer>>,
}

impl PlayerRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<PlayerMethod> for PlayerRoute {
	fn call(&self, call: &HookCall<'_, PlayerMethod>) -> HookAction<()> {
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let Some(player) = NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies the live player whose method ran, which
		// stays in the entity list through the call.
		let player = unsafe { Entity::from_live(server, player) };

		(route.callback)(server, player);
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for PlayerRoute {}

#[derive(Clone, Copy)]
struct RoutedHealed {
	binding: ServerBinding,
	callback: HealedFn,
	hook: HookId,
	vtable: usize,
}

#[derive(Clone, Copy)]
struct RoutedPlayer {
	binding: ServerBinding,
	callback: PlayerFn,
	hook: HookId,
	vtable: usize,
}

impl MetamodApi<'_> {
	/// Runs `callback` after each `CTFPlayer::TakeHealth` of a TF2 player of
	/// this player's class, with the health the call returned as gained, or the
	/// value an earlier hook returned instead.
	///
	/// The game heals players through it from healers, such as Medics' heal
	/// guns, dispensers and Crusader's Crossbow bolts, and from health packs,
	/// regeneration and the like. Healing with `DMG_IGNORE_MAXHEALTH`, as healers
	/// heal, can take a player's health beyond their maximum: overheal. The game
	/// credits healers with the healing after the call returns
	/// (`tf_player_shared.cpp:2571-2597`, `tf_projectile_arrow.cpp:1230-1237`),
	/// so their healing statistics do not count it yet as the callback runs.
	///
	/// As for [`Self::hook_player_spawned`], which describes installing it.
	pub fn hook_player_healed(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		callback: HealedFn,
	) -> Result<HookId, PlayerHookError> {
		let object = tf_player(player, binding)?;

		// SAFETY: The player is a live TF2 player, whose vtable belongs to the
		// server DLL, which outlives this plugin.
		unsafe { self.install_player_healed(object, binding, callback) }
	}

	/// Runs `callback` after each `CTFPlayer::ResetScores` of a TF2 player of
	/// this player's class: their scoring data and statistics, and with them
	/// their Score, frags and deaths, were reset.
	///
	/// The game resets a player's scores as it first spawns them
	/// (`CTFPlayer::InitialSpawn`), for each player as `mp_restartgame`, a
	/// tournament restart, or the end of the wait for players resets every
	/// player's scores, right before it fires `scorestats_accumulated_reset`
	/// (`teamplayroundbased_gamerules.cpp:3235-3268`), and in Mann vs. Machine,
	/// from its population manager, which fires no event.
	///
	/// As for [`Self::hook_player_spawned`], which describes installing it.
	pub fn hook_player_scores_reset(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		callback: PlayerFn,
	) -> Result<HookId, PlayerHookError> {
		self.hook_player_method(
			player,
			binding,
			callback,
			RESET_SCORES,
			&SCORES_RESET_ROUTES,
		)
	}

	/// Runs `callback` after each `CTFPlayer::Spawn` of a TF2 player of this
	/// player's class: the game fired `player_spawn` during it
	/// (`tf_player.cpp:3764-3772`), and the player is in the game with their
	/// team and class, or a spectator, or without a class yet.
	///
	/// The game spawns a player as they enter the game, in its `ClientActive`,
	/// and each time they respawn, through `ForceRespawn`. `CTFBot::Spawn`
	/// calls `CTFPlayer::Spawn` directly, not through the vtable, so a bot's
	/// spawn reaches only the bot class's hook, once.
	///
	/// `player` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Install on each
	/// distinct class encountered, including bots, before its players spawn,
	/// such as after the game's `ClientPutInServer` (see
	/// [`ClientEvents`](crate::ClientEvents)). Repeating a class returns
	/// [`HookError::AlreadyInstalled`]; removing its ID allows replacement.
	///
	/// The hook runs after the call even if another plugin's hook superseded
	/// it, so that the player did not spawn.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that the next spawn
	/// will be intercepted. KHook polls every 5 ms and can retry while a
	/// detour is busy, so spawns shortly after installation can be missed.
	pub fn hook_player_spawned(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		callback: PlayerFn,
	) -> Result<HookId, PlayerHookError> {
		self.hook_player_method(player, binding, callback, SPAWN, &SPAWNED_ROUTES)
	}

	/// Hooks `function` after the call, on the class of `player`, through one
	/// of `routes`.
	fn hook_player_method(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		callback: PlayerFn,
		function: VirtualFunction<PlayerMethod>,
		routes: &'static [PlayerRoute],
	) -> Result<HookId, PlayerHookError> {
		let object = tf_player(player, binding)?;

		// SAFETY: The player is a live TF2 player, whose vtable belongs to the
		// server DLL, which outlives this plugin.
		unsafe { self.install_player_method(object, binding, callback, function, routes) }
	}

	/// Hooks `TakeHealth` after the call, on the class of `object`, through one
	/// of [`HEALED_ROUTES`].
	///
	/// # Safety
	///
	/// `object` must be live, and its primary vtable must hold a function of
	/// the signature [`TakeHealth`] at [`TAKE_HEALTH_SLOT`], and stay loaded
	/// until Metamod unloads the plugin.
	unsafe fn install_player_healed(
		self,
		object: NonNull<sys::CBaseEntity>,
		binding: ServerBinding,
		callback: HealedFn,
	) -> Result<HookId, PlayerHookError> {
		// SAFETY: The object is live, and starts with its primary vtable
		// pointer, of which only the address is used.
		let vtable = unsafe { vtable_pointer::<c_void>(object.as_ptr()) }.addr();
		if HEALED_ROUTES.iter().any(|route| {
			route
				.state
				.get()
				.is_some_and(|state| state.vtable == vtable && self.has_hook(state.hook))
		}) {
			return Err(HookError::AlreadyInstalled.into());
		}
		let route = HEALED_ROUTES
			.iter()
			.find(|route| {
				route
					.state
					.get()
					.is_none_or(|state| !self.has_hook(state.hook))
			})
			.ok_or(HookError::TooManyFunctions)?;
		// SAFETY: As the caller promises, the object is live, and its vtable has
		// `int (float, int)` at the slot, and stays loaded.
		let hook = unsafe {
			self.add_hook(
				TAKE_HEALTH,
				HookTarget::class_of(object),
				HookTiming::Post,
				route,
			)
		}?;
		route.state.set(Some(RoutedHealed {
			binding,
			callback,
			hook,
			vtable,
		}));
		Ok(hook)
	}

	/// Hooks `function` after the call, on the class of `object`, through one
	/// of `routes`.
	///
	/// # Safety
	///
	/// `object` must be live, and its primary vtable must hold a function of
	/// the signature [`PlayerMethod`] at `function`'s slot, and stay loaded
	/// until Metamod unloads the plugin.
	unsafe fn install_player_method(
		self,
		object: NonNull<sys::CBaseEntity>,
		binding: ServerBinding,
		callback: PlayerFn,
		function: VirtualFunction<PlayerMethod>,
		routes: &'static [PlayerRoute],
	) -> Result<HookId, PlayerHookError> {
		// SAFETY: The object is live, and starts with its primary vtable
		// pointer, of which only the address is used.
		let vtable = unsafe { vtable_pointer::<c_void>(object.as_ptr()) }.addr();
		if routes.iter().any(|route| {
			route
				.state
				.get()
				.is_some_and(|state| state.vtable == vtable && self.has_hook(state.hook))
		}) {
			return Err(HookError::AlreadyInstalled.into());
		}
		let route = routes
			.iter()
			.find(|route| {
				route
					.state
					.get()
					.is_none_or(|state| !self.has_hook(state.hook))
			})
			.ok_or(HookError::TooManyFunctions)?;
		// SAFETY: As the caller promises, the object is live, and its vtable has
		// `void ()` at the slot, and stays loaded.
		let hook = unsafe {
			self.add_hook(
				function,
				HookTarget::class_of(object),
				HookTiming::Post,
				route,
			)
		}?;
		route.state.set(Some(RoutedPlayer {
			binding,
			callback,
			hook,
			vtable,
		}));
		Ok(hook)
	}
}

/// The entity of `player`, if the server runs TF2 and `player` is a TF2
/// player.
fn tf_player(
	player: Entity<'_>,
	binding: ServerBinding,
) -> Result<NonNull<sys::CBaseEntity>, PlayerHookError> {
	if binding.game() != Game::TeamFortress2
		|| !player
			.server_class()
			.is_some_and(|class| class.name() == c"CTFPlayer")
	{
		return Err(PlayerHookError::NotTfPlayer);
	}

	Ok(NonNull::new(player.as_ptr()).unwrap())
}
