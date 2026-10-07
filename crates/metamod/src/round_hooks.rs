//! TF2 round hooks: the map cleanup of a round's restart, which they run
//! before the game's `CTFGameRules::CleanUpMap` and may skip, and the rest of
//! a round's life, as [`MetamodApi::hook_rounds`] hooks it: which entities the
//! map's cleanup keeps and creates anew, the round's end with a win or in
//! sudden death, which they may refuse, its setup and its start, and the
//! game's questions whether control points and flags may be captured, which
//! they may answer no, and whether players block a point's capture.
//!
//! # Map cleanup
//!
//! TF2 cleans up the map as a round restarts in full:
//! `CTeamplayRoundBasedRules::RoundRespawn` calls `CleanUpMap` through the
//! game rules' vtable when the game forces a map reset or the previous round
//! waited for players, as when the game leaves its pre-game, on
//! `mp_restartgame`, and after a win that resets the map.
//! `CTFGameRules::CleanUpMap` removes every player's conditions, then the
//! entities a restart does not keep, and creates the map's entities anew. The
//! rest of the restart happens either way: removing projectiles and
//! buildings, sending every entity `RoundSpawn` and `RoundActivate`, and
//! respawning players.
//!
//! # What runs the round hooks
//!
//! - [`RoundCallbacks::win`] runs before `CTFGameRules::SetWinningTeam`,
//!   which ends the round as objectives, timers, a level's `game_round_win`,
//!   [`GameRules::set_winning_team`], the win and time limits, and the end of
//!   sudden death end it, with a win or without a winner. Refusing the win
//!   skips all of it, TF2's crit boosts and progress included, so the round
//!   goes on: the game may try again, as it does at each check of a time
//!   limit that ran out, or not, as a capture's win is not.
//! - [`RoundCallbacks::stalemate`] runs before `CTFGameRules::SetStalemate`,
//!   which starts sudden death as a control point master's round timer or
//!   the level's time limit runs out, or as a `game_round_win` without a
//!   team is sent `RoundWin`, and which ends the round without a winner
//!   instead while `mp_stalemate_enable` is off. Refusing it lets the round go
//!   on.
//! - [`RoundCallbacks::setup`] runs after `CTFGameRules::SetupOnRoundStart`,
//!   as a round or a mini-round is set up, at the start of the pre-round:
//!   after the map was cleaned up, if it was, and every entity was sent
//!   `RoundSpawn` and `RoundActivate`, before the teams switch or scramble and
//!   players respawn.
//! - [`RoundCallbacks::running`] runs after
//!   `CTFGameRules::SetupOnRoundRunning`, as the pre-round ends and players
//!   may move: after control point masters were sent `RoundStart`, before the
//!   game fires `teamplay_round_active`.
//! - [`RoundCallbacks::points_capturable`] runs before
//!   `CTFGameRules::PointsMayBeCaptured`, which control points, capture areas
//!   and control point masters ask before any capture, and which the game
//!   answers no during setup.
//! - [`RoundCallbacks::team_capture`] and [`RoundCallbacks::player_capture`]
//!   run before `CTFGameRules::TeamMayCapturePoint` and
//!   `PlayerMayCapturePoint`, which capture areas ask for the teams and players
//!   in them, and TF2 as it credits players for defending and capturing
//!   points.
//!
//! - [`RoundCallbacks::player_block`] runs before
//!   `CTFGameRules::PlayerMayBlockPoint`, which capture areas ask for the
//!   living players in them whom [`RoundCallbacks::player_capture`], or the
//!   game, refused, of teams that may capture the point: a player who blocks
//!   it stops the other team's capture as a capturing player would. The game
//!   answers yes only for invulnerable players.
//! - [`RoundCallbacks::flags_capturable`] runs before
//!   `CTFGameRules::FlagsMayBeCapped`, which flags ask before a player picks
//!   one up and as they think, which returns dropped flags, capture zones
//!   before they capture a carried flag, outside Mann vs. Machine, and Robot
//!   Destruction's logic before it scores. The game answers no in the
//!   pre-round and once a team has won the round.
//! - [`RoundCallbacks::cleanup_keep`] runs before
//!   `CTFGameRules::RoundCleanupShouldIgnore`, which the map's cleanup asks
//!   of every entity, and removes those not kept. The game keeps players,
//!   their weapons and wearables, the game rules, the teams and the other
//!   entities of its list of classes. An entity the map placed that is kept
//!   is also created anew from the map, unless
//!   [`RoundCallbacks::cleanup_create`] skips its class.
//! - [`RoundCallbacks::cleanup_create`] runs before
//!   `CTFGameRules::ShouldCreateEntity`, which the map's cleanup asks of each
//!   class name of the map's entities as it creates them anew, after it
//!   removed the others. The game skips the classes it keeps. Only the
//!   cleanup asks, so the map's entities are all created as the level loads.
//!
//! Clients ask their own game rules whether points may be captured, so their
//! HUD can show a capture the server refuses.
//!
//! # Installing
//!
//! The hooks patch the methods in the primary vtable of `CTFGameRules`, which
//! [`game_rules_vtable`] finds in the game server module without any game
//! rules object. The game creates one game rules object as each level loads
//! and deletes it as the level shuts down, all of the same class, so one hook
//! covers every level's: install them once, such as while loading. They last
//! until removed or the plugin unloads. As with other Metamod hooks, they stop
//! calling back while the plugin is paused, so the game runs its rounds as
//! usual then.
//!
//! [`GameRules::set_winning_team`]: source_sdk_2013::tf2::game_rules::GameRules::set_winning_team

#[cfg(test)]
#[path = "tests/round_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, Signature,
	VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::players::TEAM_UNASSIGNED;

use source_sdk_2013::raw::tf2::game_rules::{
	CLEAN_UP_MAP_SLOT, CleanUpMapFn as CleanUpMap, FLAGS_MAY_BE_CAPPED_SLOT,
	FlagsMayBeCappedFn as FlagsMayBeCapped, PLAYER_MAY_BLOCK_POINT_SLOT,
	PLAYER_MAY_CAPTURE_POINT_SLOT, POINTS_MAY_BE_CAPTURED_SLOT,
	PlayerMayBlockPointFn as PlayerMayBlockPoint, PlayerMayCapturePointFn as PlayerMayCapturePoint,
	PointsMayBeCapturedFn as PointsMayBeCaptured, ROUND_CLEANUP_SHOULD_IGNORE_SLOT,
	RoundCleanupShouldIgnoreFn as RoundCleanupShouldIgnore, RoundSetupFn as RoundSetup,
	SET_STALEMATE_SLOT, SET_WINNING_TEAM_SLOT, SETUP_ON_ROUND_RUNNING_SLOT,
	SETUP_ON_ROUND_START_SLOT, SHOULD_CREATE_ENTITY_SLOT, SetStalemateFn as SetStalemate,
	SetWinningTeamFn as SetWinningTeam, ShouldCreateEntityFn as ShouldCreateEntity,
	TEAM_MAY_CAPTURE_POINT_SLOT, TeamMayCapturePointFn as TeamMayCapturePoint,
};

use source_sdk_2013::tf2::game_rules::{GameRulesVtableError, game_rules_vtable};
use source_sdk_2013::tf2::objectives::WinReason;
use source_sdk_2013::tf2::round_end::{StalemateOptions, StalemateReason, WinOptions};
use source_sdk_2013::tf2::scoreboard::ScoringTeam;
use source_sdk_2013::tf2::teams::Team;
use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

/// A callback-scoped server, and the class name of the map's entities the
/// map's cleanup is about to create anew, which decides whether it does. A
/// panic is contained by the hook dispatcher, and lets the game decide.
pub type CleanupCreateFn = for<'s> fn(Server<'s>, &CStr) -> CleanupCreateAction;

/// A callback-scoped server, and an entity the map's cleanup is about to
/// remove, unless it keeps it, which decides whether it is kept. A panic is
/// contained by the hook dispatcher, and lets the game decide.
pub type CleanupKeepFn = for<'s> fn(Server<'s>, Entity<'s>) -> CleanupKeepAction;

/// A callback-scoped server whose flags are asked about, which decides
/// whether they may be picked up and captured. A panic is contained by the
/// hook dispatcher, and lets the game decide.
pub type FlagsCapturableFn = for<'s> fn(Server<'s>) -> CaptureAction;

/// A callback-scoped server whose map is about to be cleaned up, which
/// decides whether it is. A panic is contained by the hook dispatcher, and
/// lets the cleanup run.
pub type MapCleanupFn = for<'s> fn(Server<'s>) -> MapCleanupAction;

/// A callback-scoped server, and a player a capture area asks about, who may
/// not capture the point, returning whether the player blocks its capture, or
/// `None` to let the game decide. A panic is contained by the hook dispatcher,
/// and lets the game decide.
pub type PlayerBlockFn = for<'s> fn(Server<'s>, PlayerCaptureCheck<'s>) -> Option<bool>;

/// A callback-scoped server, and a player a capture area or TF2 asks about,
/// which decides whether the player may capture the point. A panic is
/// contained by the hook dispatcher, and lets the game decide.
pub type PlayerCaptureFn = for<'s> fn(Server<'s>, PlayerCaptureCheck<'s>) -> CaptureAction;

/// A callback-scoped server whose control points are asked about, which
/// decides whether any may be captured. A panic is contained by the hook
/// dispatcher, and lets the game decide.
pub type PointsCapturableFn = for<'s> fn(Server<'s>) -> CaptureAction;

/// A callback-scoped server whose round was just set up, or just started
/// running. A panic is contained by the hook dispatcher.
pub type RoundFn = for<'s> fn(Server<'s>);

/// A callback-scoped server, and a round's end with a win, or without a
/// winner, which decides whether the round ends. A panic is contained by the
/// hook dispatcher, and lets the round end.
pub type RoundWinFn = for<'s> fn(Server<'s>, RoundWinEvent) -> RoundEndAction;

/// A callback-scoped server, and a round's sudden death, which decides
/// whether it starts. A panic is contained by the hook dispatcher, and lets
/// it start.
pub type StalemateFn = for<'s> fn(Server<'s>, StalemateEvent) -> RoundEndAction;

/// A callback-scoped server, and a team a capture area or TF2 asks about,
/// which decides whether the team may capture the point. A panic is
/// contained by the hook dispatcher, and lets the game decide.
pub type TeamCaptureFn = for<'s> fn(Server<'s>, TeamCaptureCheck) -> CaptureAction;

/// `CleanUpMap` in TF2's game rules' primary vtable.
const CLEAN_UP_MAP: VirtualFunction<CleanUpMap> = VirtualFunction::new(CLEAN_UP_MAP_SLOT);

/// `FlagsMayBeCapped` in TF2's game rules' primary vtable.
const FLAGS_MAY_BE_CAPPED: VirtualFunction<FlagsMayBeCapped> =
	VirtualFunction::new(FLAGS_MAY_BE_CAPPED_SLOT);

/// `PlayerMayBlockPoint` in TF2's game rules' primary vtable.
const PLAYER_MAY_BLOCK_POINT: VirtualFunction<PlayerMayBlockPoint> =
	VirtualFunction::new(PLAYER_MAY_BLOCK_POINT_SLOT);

/// `PlayerMayCapturePoint` in TF2's game rules' primary vtable.
const PLAYER_MAY_CAPTURE_POINT: VirtualFunction<PlayerMayCapturePoint> =
	VirtualFunction::new(PLAYER_MAY_CAPTURE_POINT_SLOT);

/// `PointsMayBeCaptured` in TF2's game rules' primary vtable.
const POINTS_MAY_BE_CAPTURED: VirtualFunction<PointsMayBeCaptured> =
	VirtualFunction::new(POINTS_MAY_BE_CAPTURED_SLOT);

/// `RoundCleanupShouldIgnore` in TF2's game rules' primary vtable.
const ROUND_CLEANUP_SHOULD_IGNORE: VirtualFunction<RoundCleanupShouldIgnore> =
	VirtualFunction::new(ROUND_CLEANUP_SHOULD_IGNORE_SLOT);

/// `SetStalemate` in TF2's game rules' primary vtable.
const SET_STALEMATE: VirtualFunction<SetStalemate> = VirtualFunction::new(SET_STALEMATE_SLOT);

/// `SetWinningTeam` in TF2's game rules' primary vtable.
const SET_WINNING_TEAM: VirtualFunction<SetWinningTeam> =
	VirtualFunction::new(SET_WINNING_TEAM_SLOT);

/// `SetupOnRoundRunning` in TF2's game rules' primary vtable.
const SETUP_ON_ROUND_RUNNING: VirtualFunction<RoundSetup> =
	VirtualFunction::new(SETUP_ON_ROUND_RUNNING_SLOT);

/// `SetupOnRoundStart` in TF2's game rules' primary vtable.
const SETUP_ON_ROUND_START: VirtualFunction<RoundSetup> =
	VirtualFunction::new(SETUP_ON_ROUND_START_SLOT);

/// `ShouldCreateEntity` in TF2's game rules' primary vtable.
const SHOULD_CREATE_ENTITY: VirtualFunction<ShouldCreateEntity> =
	VirtualFunction::new(SHOULD_CREATE_ENTITY_SLOT);

/// `TeamMayCapturePoint` in TF2's game rules' primary vtable.
const TEAM_MAY_CAPTURE_POINT: VirtualFunction<TeamMayCapturePoint> =
	VirtualFunction::new(TEAM_MAY_CAPTURE_POINT_SLOT);

static ROUTE: CleanupRoute = CleanupRoute::new();

static CLEANUP_CREATE_ROUTE: RoundRoute<CleanupCreateFn> = RoundRoute::new();
static CLEANUP_KEEP_ROUTE: RoundRoute<CleanupKeepFn> = RoundRoute::new();
static FLAGS_ROUTE: RoundRoute<FlagsCapturableFn> = RoundRoute::new();
static PLAYER_BLOCK_ROUTE: RoundRoute<PlayerBlockFn> = RoundRoute::new();
static PLAYER_CAPTURE_ROUTE: RoundRoute<PlayerCaptureFn> = RoundRoute::new();
static POINTS_ROUTE: RoundRoute<PointsCapturableFn> = RoundRoute::new();
static RUNNING_ROUTE: RoundRoute<RoundFn> = RoundRoute::new();
static SETUP_ROUTE: RoundRoute<RoundFn> = RoundRoute::new();
static STALEMATE_ROUTE: RoundRoute<StalemateFn> = RoundRoute::new();
static TEAM_CAPTURE_ROUTE: RoundRoute<TeamCaptureFn> = RoundRoute::new();
static WIN_ROUTE: RoundRoute<RoundWinFn> = RoundRoute::new();

/// What a capture hook does with the game's question whether a capture may
/// happen.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CaptureAction {
	/// Lets the game decide, through the other plugins' hooks.
	#[default]
	Allow,

	/// Answers that the capture may not happen, without asking the game.
	Refuse,
}

/// What a cleanup hook does with the map's entities of a class, which the
/// map's cleanup is about to create anew.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CleanupCreateAction {
	/// Lets the game decide, through the other plugins' hooks: it creates them
	/// unless it keeps the class's entities through cleanups.
	#[default]
	Allow,

	/// Skips them: the map's entities of the class are not created anew, so
	/// those the cleanup removed are gone until the level loads again.
	Skip,
}

/// What a cleanup hook does with an entity the map's cleanup is about to
/// remove, unless it keeps it.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CleanupKeepAction {
	/// Lets the game decide, through the other plugins' hooks.
	#[default]
	Allow,

	/// Keeps the entity through the cleanup.
	Keep,
}

/// What a map cleanup hook does with a cleanup.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MapCleanupAction {
	/// Lets the game clean up the map.
	#[default]
	Allow,

	/// Skips `CleanUpMap`: the map's entities stay as they are, and none of
	/// the function's effects apply, including removing players' conditions.
	/// The rest of the round's restart runs.
	Skip,
}

/// Why a map cleanup hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum MapCleanupHookError {
	/// The server does not run TF2, or the game rules class's vtable could
	/// not be found.
	#[error(transparent)]
	Target(#[from] GameRulesVtableError),

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means the
	/// game rules class is already hooked, which callers installing more than
	/// once can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct CleanupRoute {
	state: Cell<Option<RoutedCleanup>>,
}

impl CleanupRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<CleanUpMap> for CleanupRoute {
	fn call(&self, call: &HookCall<'_, CleanUpMap>) -> HookAction<()> {
		// An earlier hook already skipped the cleanup.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };

		match (route.callback)(server) {
			MapCleanupAction::Allow => HookAction::Ignore,
			MapCleanupAction::Skip => HookAction::Supersede(()),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for CleanupRoute {}

#[derive(Clone, Copy)]
struct RoutedCleanup {
	binding: ServerBinding,
	callback: MapCleanupFn,
	hook: HookId,
	vtable: usize,
}

/// A player a capture area or TF2 asks about.
#[derive(Debug, Clone, Copy)]
pub struct PlayerCaptureCheck<'s> {
	/// The player.
	pub player: Entity<'s>,

	/// The control point's index, as its `point_index` key sets it.
	pub point: c_int,
}

/// The callbacks of the round hooks, each of which hooks its method of TF2's
/// game rules; see the [module documentation](crate::round_hooks).
#[derive(Debug, Clone, Copy, Default)]
pub struct RoundCallbacks {
	/// Runs before the map's cleanup creates the map's entities of a class
	/// anew, and decides whether it does.
	pub cleanup_create: Option<CleanupCreateFn>,

	/// Runs before the map's cleanup removes or keeps an entity, and decides
	/// whether it keeps the entity.
	pub cleanup_keep: Option<CleanupKeepFn>,

	/// Runs before a flag, a capture zone or Robot Destruction's logic asks
	/// whether flags may be picked up and captured, and decides whether they
	/// may.
	pub flags_capturable: Option<FlagsCapturableFn>,

	/// Runs before a capture area asks whether a player who may not capture a
	/// control point blocks its capture, and decides whether the player does.
	pub player_block: Option<PlayerBlockFn>,

	/// Runs before a capture area or TF2 asks whether a player may capture a
	/// control point, and decides whether the player may.
	pub player_capture: Option<PlayerCaptureFn>,

	/// Runs before a control point, a capture area, or a control point master
	/// asks whether any point may be captured, and decides whether any may.
	pub points_capturable: Option<PointsCapturableFn>,

	/// Runs after the pre-round ends and the round starts running.
	pub running: Option<RoundFn>,

	/// Runs after a round, or a mini-round, is set up, before players
	/// respawn.
	pub setup: Option<RoundFn>,

	/// Runs before a round goes to sudden death, and decides whether it does.
	pub stalemate: Option<StalemateFn>,

	/// Runs before a capture area or TF2 asks whether a team may capture a
	/// control point, and decides whether the team may.
	pub team_capture: Option<TeamCaptureFn>,

	/// Runs before a round ends with a win, or without a winner, and decides
	/// whether it ends.
	pub win: Option<RoundWinFn>,
}

/// What a round win or stalemate hook does with a round's end.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RoundEndAction {
	/// Lets the round end, through the other plugins' hooks.
	#[default]
	Allow,

	/// Refuses the round's end: the method is skipped, so the round goes on
	/// as if nothing had ended it, and TF2 neither crit boosts the winners
	/// nor counts the teams' progress.
	Refuse,
}

/// Why the round hooks, or the game rules hooks of
/// [`crate::rules_hooks`], could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum RoundHookError {
	/// The server does not run TF2, or the game rules class's vtable could
	/// not be found.
	#[error(transparent)]
	Target(#[from] GameRulesVtableError),

	/// Metamod refused a hook. [`HookError::AlreadyInstalled`] means the
	/// rounds are already hooked.
	#[error(transparent)]
	Hook(#[from] HookError),
}

/// Handles for the round hooks installed by one call.
///
/// Metamod disables them while paused and removes them before unloading the
/// plugin. [`Self::remove`] disables them earlier. No game rules object is
/// retained, so the hooks last through level changes.
#[must_use = "retain round hooks to support explicitly removing them"]
#[derive(Debug)]
pub struct RoundHooks {
	hooks: Vec<HookId>,
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl RoundHooks {
	pub fn remove(self, api: MetamodApi<'_>) {
		for hook in self.hooks {
			api.remove_hook(hook);
			CLEANUP_CREATE_ROUTE.clear(hook);
			CLEANUP_KEEP_ROUTE.clear(hook);
			FLAGS_ROUTE.clear(hook);
			PLAYER_BLOCK_ROUTE.clear(hook);
			PLAYER_CAPTURE_ROUTE.clear(hook);
			POINTS_ROUTE.clear(hook);
			RUNNING_ROUTE.clear(hook);
			SETUP_ROUTE.clear(hook);
			STALEMATE_ROUTE.clear(hook);
			TEAM_CAPTURE_ROUTE.clear(hook);
			WIN_ROUTE.clear(hook);
		}
	}
}

/// A hook's route to its callback, for the methods of the game rules.
pub(crate) struct RoundRoute<C: 'static> {
	state: Cell<Option<RoutedRound<C>>>,
}

impl<C: Copy> RoundRoute<C> {
	pub(crate) const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}

	/// Forgets `hook`, if the route was installed with it.
	pub(crate) fn clear(&self, hook: HookId) {
		if self.state.get().is_some_and(|state| state.hook == hook) {
			self.state.set(None);
		}
	}

	/// The route's callback and server, unless the route is not installed.
	pub(crate) fn enter<'s>(&self, scope: &'s ()) -> Option<(C, Server<'s>)> {
		let route = self.state.get()?;
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(scope) };

		Some((route.callback, server))
	}

	/// Whether the route has a hook installed, for this load of the plugin.
	pub(crate) fn is_routed(&self, api: MetamodApi<'_>) -> bool {
		self.state
			.get()
			.is_some_and(|state| api.has_hook(state.hook))
	}
}

impl Handler<PlayerMayBlockPoint> for RoundRoute<PlayerBlockFn> {
	fn call(&self, call: &HookCall<'_, PlayerMayBlockPoint>) -> HookAction<bool> {
		// An earlier hook already answered.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let scope = ();
		let Some((callback, server)) = self.enter(&scope) else {
			return HookAction::Ignore;
		};
		let (player, point, reason, reason_size) = call.args();
		let Some(player) = NonNull::new(player) else {
			return HookAction::Ignore;
		};
		// SAFETY: The game passes a live player, which stays in the entity list
		// through the call, and whose `CBaseEntity` base is at its address.
		let player = unsafe { Entity::from_live(server, player.cast()) };
		let Some(blocks) = callback(server, PlayerCaptureCheck { player, point }) else {
			return HookAction::Ignore;
		};

		// SAFETY: The caller passes a writable buffer of the size, or null.
		unsafe { clear_reason(reason, reason_size) };
		HookAction::Supersede(blocks)
	}
}

impl Handler<PlayerMayCapturePoint> for RoundRoute<PlayerCaptureFn> {
	fn call(&self, call: &HookCall<'_, PlayerMayCapturePoint>) -> HookAction<bool> {
		// An earlier hook already answered.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let scope = ();
		let Some((callback, server)) = self.enter(&scope) else {
			return HookAction::Ignore;
		};
		let (player, point, reason, reason_size) = call.args();
		let Some(player) = NonNull::new(player) else {
			return HookAction::Ignore;
		};
		// SAFETY: The game passes a live player, which stays in the entity list
		// through the call, and whose `CBaseEntity` base is at its address.
		let player = unsafe { Entity::from_live(server, player.cast()) };

		match callback(server, PlayerCaptureCheck { player, point }) {
			CaptureAction::Allow => HookAction::Ignore,

			CaptureAction::Refuse => {
				// SAFETY: The caller passes a writable buffer of the size, or
				// null.
				unsafe { clear_reason(reason, reason_size) };
				HookAction::Supersede(false)
			}
		}
	}
}

impl Handler<PointsMayBeCaptured> for RoundRoute<PointsCapturableFn> {
	fn call(&self, call: &HookCall<'_, PointsMayBeCaptured>) -> HookAction<bool> {
		// An earlier hook already answered.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let scope = ();
		let Some((callback, server)) = self.enter(&scope) else {
			return HookAction::Ignore;
		};

		match callback(server) {
			CaptureAction::Allow => HookAction::Ignore,
			CaptureAction::Refuse => HookAction::Supersede(false),
		}
	}
}

impl Handler<RoundCleanupShouldIgnore> for RoundRoute<CleanupKeepFn> {
	fn call(&self, call: &HookCall<'_, RoundCleanupShouldIgnore>) -> HookAction<bool> {
		// An earlier hook already answered.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(entity) = NonNull::new(call.args().0) else {
			return HookAction::Ignore;
		};

		let scope = ();
		let Some((callback, server)) = self.enter(&scope) else {
			return HookAction::Ignore;
		};
		// SAFETY: The cleanup passes an entity of the entity list, which it
		// removes, if it does, only through deferred deletion after the call.
		let entity = unsafe { Entity::from_live(server, entity) };

		match callback(server, entity) {
			CleanupKeepAction::Allow => HookAction::Ignore,
			CleanupKeepAction::Keep => HookAction::Supersede(true),
		}
	}
}

impl Handler<RoundSetup> for RoundRoute<RoundFn> {
	fn call(&self, _call: &HookCall<'_, RoundSetup>) -> HookAction<()> {
		let scope = ();

		if let Some((callback, server)) = self.enter(&scope) {
			callback(server);
		}

		HookAction::Ignore
	}
}

impl Handler<SetStalemate> for RoundRoute<StalemateFn> {
	fn call(&self, call: &HookCall<'_, SetStalemate>) -> HookAction<()> {
		// An earlier hook already skipped the stalemate.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let scope = ();
		let Some((callback, server)) = self.enter(&scope) else {
			return HookAction::Ignore;
		};
		let (reason, reset_map, switch_teams) = call.args();
		let event = StalemateEvent {
			reason: StalemateReason::from_raw(reason),
			options: StalemateOptions {
				reset_map,
				switch_teams,
			},
		};

		match callback(server, event) {
			RoundEndAction::Allow => HookAction::Ignore,
			RoundEndAction::Refuse => HookAction::Supersede(()),
		}
	}
}

impl Handler<SetWinningTeam> for RoundRoute<RoundWinFn> {
	fn call(&self, call: &HookCall<'_, SetWinningTeam>) -> HookAction<()> {
		// An earlier hook already skipped the win.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let (team, reason, reset_map, switch_teams, dont_add_score, final_round) = call.args();

		// The game refuses other teams.
		let winner = match team {
			TEAM_UNASSIGNED => None,

			team => match ScoringTeam::from_raw(team) {
				Some(team) => Some(team),
				None => return HookAction::Ignore,
			},
		};

		let scope = ();
		let Some((callback, server)) = self.enter(&scope) else {
			return HookAction::Ignore;
		};
		let event = RoundWinEvent {
			winner,
			reason: WinReason::from_raw(reason),
			options: WinOptions {
				reset_map,
				switch_teams,
				add_score: !dont_add_score,
				final_round,
			},
		};

		match callback(server, event) {
			RoundEndAction::Allow => HookAction::Ignore,
			RoundEndAction::Refuse => HookAction::Supersede(()),
		}
	}
}

impl Handler<ShouldCreateEntity> for RoundRoute<CleanupCreateFn> {
	fn call(&self, call: &HookCall<'_, ShouldCreateEntity>) -> HookAction<bool> {
		// An earlier hook already answered.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let class_name = call.args().0;

		if class_name.is_null() {
			return HookAction::Ignore;
		}

		let scope = ();
		let Some((callback, server)) = self.enter(&scope) else {
			return HookAction::Ignore;
		};
		// SAFETY: The map's entity parser passes the class name it read, a
		// string that lives through the call.
		let class_name = unsafe { CStr::from_ptr(class_name) };

		match callback(server, class_name) {
			CleanupCreateAction::Allow => HookAction::Ignore,
			CleanupCreateAction::Skip => HookAction::Supersede(false),
		}
	}
}

impl Handler<TeamMayCapturePoint> for RoundRoute<TeamCaptureFn> {
	fn call(&self, call: &HookCall<'_, TeamMayCapturePoint>) -> HookAction<bool> {
		// An earlier hook already answered.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let (team, point) = call.args();
		let Some(team) = Team::from_raw(team) else {
			return HookAction::Ignore;
		};

		let scope = ();
		let Some((callback, server)) = self.enter(&scope) else {
			return HookAction::Ignore;
		};

		match callback(server, TeamCaptureCheck { team, point }) {
			CaptureAction::Allow => HookAction::Ignore,
			CaptureAction::Refuse => HookAction::Supersede(false),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl<C> Sync for RoundRoute<C> {}

/// Clears the buffer a capture area passes for the reason the game answers
/// as it does, which the game writes, if given one: a hook's answer gives
/// none.
///
/// # Safety
///
/// `reason` must be null, or writable for `reason_size` bytes.
unsafe fn clear_reason(reason: *mut c_char, reason_size: c_int) {
	if !reason.is_null() && reason_size > 0 {
		// SAFETY: As the caller promises.
		unsafe { reason.write(0) };
	}
}

/// A round's end with a win, or without a winner, about to happen, as TF2's
/// game rules are told to end it (`SetWinningTeam`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundWinEvent {
	/// The winning team, or `None` for a round's end without a winner, as
	/// after a stalemate.
	pub winner: Option<ScoringTeam>,

	/// Why the round ends, or `None` for a reason this crate does not know.
	pub reason: Option<WinReason>,

	/// What follows the round's end.
	pub options: WinOptions,
}

#[derive(Clone, Copy)]
struct RoutedRound<C> {
	binding: ServerBinding,
	callback: C,
	hook: HookId,
}

/// A round's sudden death about to start, as TF2's game rules are told to
/// start it (`SetStalemate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StalemateEvent {
	/// Why the round goes to sudden death, or `None` for a reason this crate
	/// does not know.
	pub reason: Option<StalemateReason>,

	/// What follows the stalemate.
	pub options: StalemateOptions,
}

/// A team a capture area or TF2 asks about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamCaptureCheck {
	/// The team.
	pub team: Team,

	/// The control point's index, as its `point_index` key sets it.
	pub point: c_int,
}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CTFGameRules::CleanUpMap`, which cleans
	/// up the map unless the callback returns [`MapCleanupAction::Skip`]; see
	/// the [module documentation](crate::round_hooks).
	///
	/// `binding` must describe the same running server as `server`. Finds the
	/// game rules class in the game server module, which needs no level to be
	/// loaded, and snapshots the module to do so: install once, such as while
	/// loading. Returns a removable hook ID. Installing again while the hook
	/// is installed returns [`HookError::AlreadyInstalled`] without searching
	/// the module again; removing the ID allows replacement.
	///
	/// The callback is not run once an earlier hook skipped the cleanup, as
	/// far as the hooking library reports it (see [`crate::hook`]). It runs
	/// inside a round's restart, before players respawn: it must not restart
	/// the round or delete entities immediately, as [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that the next
	/// cleanup will be intercepted. KHook polls every 5 ms and can retry while
	/// a detour is busy, so cleanups shortly after installation can be missed.
	pub fn hook_map_cleanup(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callback: MapCleanupFn,
	) -> Result<HookId, MapCleanupHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(GameRulesVtableError::WrongGame.into());
		}

		// The class's vtable is the same for the whole load.
		if ROUTE
			.state
			.get()
			.is_some_and(|state| self.has_hook(state.hook))
		{
			return Err(HookError::AlreadyInstalled.into());
		}

		let vtable = game_rules_vtable(server)?;

		// SAFETY: Run-time type information identified `CTFGameRules`' primary
		// vtable in the game module, which holds `void ()` at the slot, and
		// outlives the plugin.
		Ok(unsafe { self.install_map_cleanup(vtable.as_ptr(), binding, callback) }?)
	}

	/// Hooks `CleanUpMap` on the class whose primary vtable is `vtable`.
	///
	/// # Safety
	///
	/// `vtable` must be live, hold a function of the signature [`CleanUpMap`]
	/// at [`CLEAN_UP_MAP_SLOT`], and stay loaded until Metamod unloads the
	/// plugin.
	unsafe fn install_map_cleanup(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: MapCleanupFn,
	) -> Result<HookId, HookError> {
		let address = vtable.addr().get();

		match ROUTE.state.get() {
			Some(state) if self.has_hook(state.hook) => {
				return Err(match state.vtable == address {
					true => HookError::AlreadyInstalled,
					false => HookError::TooManyFunctions,
				});
			}

			_ => {}
		}

		// SAFETY: As the caller promises.
		let hook = unsafe {
			self.add_hook(
				CLEAN_UP_MAP,
				HookTarget::vtable(vtable),
				HookTiming::Pre,
				&ROUTE,
			)
		}?;

		ROUTE.state.set(Some(RoutedCleanup {
			binding,
			callback,
			hook,
			vtable: address,
		}));
		Ok(hook)
	}
}

impl MetamodApi<'_> {
	/// Runs `callbacks` as TF2's rounds end, go to sudden death, are set up
	/// and start running, as the game asks whether control points and flags
	/// may be captured and whether players block captures, and as the map's
	/// cleanup decides what it keeps and creates anew, hooking only the
	/// methods `callbacks` names a callback for; see the
	/// [module documentation](crate::round_hooks).
	///
	/// `binding` must describe the same running server as `server`. Finds the
	/// game rules class in the game server module, as
	/// [`Self::hook_map_cleanup`] does, which needs no level to be loaded, and
	/// snapshots the module to do so: install once, such as while loading. A
	/// refused hook rolls back the others. Installing again while any of the
	/// hooks is installed returns [`HookError::AlreadyInstalled`] without
	/// searching the module again; removing them allows replacement. Without
	/// any callback, nothing is searched for or hooked.
	///
	/// The callbacks run as the game runs, inside its round's logic: they must
	/// not restart the round or delete entities immediately, as
	/// [`Server::new`] requires, and the win and stalemate callbacks must not
	/// end the round themselves.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload. A
	/// returned handle means registration was accepted, not that the next call
	/// will be intercepted.
	pub fn hook_rounds(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callbacks: RoundCallbacks,
	) -> Result<RoundHooks, RoundHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(GameRulesVtableError::WrongGame.into());
		}

		// The class's vtable is the same for the whole load.
		if self.rounds_hooked() {
			return Err(HookError::AlreadyInstalled.into());
		}

		let none = callbacks.cleanup_create.is_none()
			&& callbacks.cleanup_keep.is_none()
			&& callbacks.flags_capturable.is_none()
			&& callbacks.player_block.is_none()
			&& callbacks.player_capture.is_none()
			&& callbacks.points_capturable.is_none()
			&& callbacks.running.is_none()
			&& callbacks.setup.is_none()
			&& callbacks.stalemate.is_none()
			&& callbacks.team_capture.is_none()
			&& callbacks.win.is_none();

		if none {
			return Ok(RoundHooks {
				hooks: Vec::new(),
				_not_thread_safe: PhantomData,
			});
		}

		let vtable = game_rules_vtable(server)?;

		// SAFETY: Run-time type information identified `CTFGameRules`' primary
		// vtable in the game module, which holds the methods at the generated
		// binding's slots, and outlives the plugin.
		Ok(unsafe { self.install_rounds(vtable.as_ptr(), binding, callbacks) }?)
	}

	/// Hooks the methods `callbacks` names a callback for on the class whose
	/// primary vtable is `vtable`, all or none.
	///
	/// # Safety
	///
	/// `vtable` must be live, hold functions of the signatures
	/// [`FlagsMayBeCapped`], [`PlayerMayBlockPoint`], [`PlayerMayCapturePoint`],
	/// [`PointsMayBeCaptured`], [`RoundCleanupShouldIgnore`], [`RoundSetup`],
	/// [`SetStalemate`], [`SetWinningTeam`], [`ShouldCreateEntity`] and
	/// [`TeamMayCapturePoint`] at the slots of `CTFGameRules`' methods of those
	/// names, called on game rules objects, and stay loaded until Metamod
	/// unloads the plugin.
	unsafe fn install_rounds(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callbacks: RoundCallbacks,
	) -> Result<RoundHooks, HookError> {
		if self.rounds_hooked() {
			return Err(HookError::AlreadyInstalled);
		}

		let mut installed = RoundHooks {
			hooks: Vec::new(),
			_not_thread_safe: PhantomData,
		};

		// SAFETY: As the caller promises.
		match unsafe { self.route_rounds(vtable, binding, callbacks, &mut installed.hooks) } {
			Ok(()) => Ok(installed),

			Err(error) => {
				installed.remove(self);
				Err(error)
			}
		}
	}

	/// Hooks `function` on the class whose primary vtable is `vtable`, through
	/// `route`, and adds the hook to `hooks`.
	///
	/// # Safety
	///
	/// `vtable` must be live, hold a function of the signature `S` at the
	/// function's slot, called on game rules objects, and stay loaded until
	/// Metamod unloads the plugin.
	pub(crate) unsafe fn route_round<S: Signature, C: Copy>(
		self,
		function: VirtualFunction<S>,
		timing: HookTiming,
		route: &'static RoundRoute<C>,
		vtable: NonNull<*mut c_void>,
		(binding, callback): (ServerBinding, C),
		hooks: &mut Vec<HookId>,
	) -> Result<(), HookError>
	where
		RoundRoute<C>: Handler<S>,
	{
		// SAFETY: As the caller promises.
		let hook = unsafe { self.add_hook(function, HookTarget::vtable(vtable), timing, route) }?;

		route.state.set(Some(RoutedRound {
			binding,
			callback,
			hook,
		}));
		hooks.push(hook);
		Ok(())
	}

	/// Hooks the methods `callbacks` names a callback for on the class whose
	/// primary vtable is `vtable`, and adds the hooks to `hooks`, as far as
	/// Metamod accepts them.
	///
	/// # Safety
	///
	/// As for [`Self::install_rounds`].
	unsafe fn route_rounds(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callbacks: RoundCallbacks,
		hooks: &mut Vec<HookId>,
	) -> Result<(), HookError> {
		let RoundCallbacks {
			cleanup_create,
			cleanup_keep,
			flags_capturable,
			player_block,
			player_capture,
			points_capturable,
			running,
			setup,
			stalemate,
			team_capture,
			win,
		} = callbacks;

		// SAFETY: As the caller promises, the vtable has each method at its
		// slot, called on game rules objects, and stays loaded.
		unsafe {
			if let Some(callback) = cleanup_create {
				self.route_round(
					SHOULD_CREATE_ENTITY,
					HookTiming::Pre,
					&CLEANUP_CREATE_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = cleanup_keep {
				self.route_round(
					ROUND_CLEANUP_SHOULD_IGNORE,
					HookTiming::Pre,
					&CLEANUP_KEEP_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = flags_capturable {
				self.route_round(
					FLAGS_MAY_BE_CAPPED,
					HookTiming::Pre,
					&FLAGS_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = player_block {
				self.route_round(
					PLAYER_MAY_BLOCK_POINT,
					HookTiming::Pre,
					&PLAYER_BLOCK_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = player_capture {
				self.route_round(
					PLAYER_MAY_CAPTURE_POINT,
					HookTiming::Pre,
					&PLAYER_CAPTURE_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = points_capturable {
				self.route_round(
					POINTS_MAY_BE_CAPTURED,
					HookTiming::Pre,
					&POINTS_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = running {
				self.route_round(
					SETUP_ON_ROUND_RUNNING,
					HookTiming::Post,
					&RUNNING_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = setup {
				self.route_round(
					SETUP_ON_ROUND_START,
					HookTiming::Post,
					&SETUP_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = stalemate {
				self.route_round(
					SET_STALEMATE,
					HookTiming::Pre,
					&STALEMATE_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = team_capture {
				self.route_round(
					TEAM_MAY_CAPTURE_POINT,
					HookTiming::Pre,
					&TEAM_CAPTURE_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}

			if let Some(callback) = win {
				self.route_round(
					SET_WINNING_TEAM,
					HookTiming::Pre,
					&WIN_ROUTE,
					vtable,
					(binding, callback),
					hooks,
				)?;
			}
		}

		Ok(())
	}

	/// Whether any round hook is installed, for this load of the plugin.
	fn rounds_hooked(self) -> bool {
		CLEANUP_CREATE_ROUTE.is_routed(self)
			|| CLEANUP_KEEP_ROUTE.is_routed(self)
			|| FLAGS_ROUTE.is_routed(self)
			|| PLAYER_BLOCK_ROUTE.is_routed(self)
			|| PLAYER_CAPTURE_ROUTE.is_routed(self)
			|| POINTS_ROUTE.is_routed(self)
			|| RUNNING_ROUTE.is_routed(self)
			|| SETUP_ROUTE.is_routed(self)
			|| STALEMATE_ROUTE.is_routed(self)
			|| TEAM_CAPTURE_ROUTE.is_routed(self)
			|| WIN_ROUTE.is_routed(self)
	}
}
