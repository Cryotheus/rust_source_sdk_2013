//! TF2 round hooks: the map cleanup of a round's restart, which they run
//! before the game's `CTFGameRules::CleanUpMap` and may skip.
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
//! The hook patches `CleanUpMap` in the primary vtable of `CTFGameRules`,
//! which [`game_rules_vtable`] finds in the game server module without any
//! game rules object. The game creates one game rules object as each level
//! loads and deletes it as the level shuts down, all of the same class, so
//! one hook covers every level's: install it once, such as while loading. It
//! lasts until removed or the plugin unloads. As with other Metamod hooks, it
//! stops calling back while the plugin is paused, so the game cleans up the
//! map as usual then.

#[cfg(test)]
#[path = "tests/round_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::raw::tf2::game_rules::{CLEAN_UP_MAP_SLOT, CleanUpMapFn as CleanUpMap};
use source_sdk_2013::tf2::game_rules::{GameRulesVtableError, game_rules_vtable};
use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

/// A callback-scoped server whose map is about to be cleaned up, which
/// decides whether it is. A panic is contained by the hook dispatcher, and
/// lets the cleanup run.
pub type MapCleanupFn = for<'s> fn(Server<'s>) -> MapCleanupAction;

/// `CleanUpMap` in TF2's game rules' primary vtable.
const CLEAN_UP_MAP: VirtualFunction<CleanUpMap> = VirtualFunction::new(CLEAN_UP_MAP_SLOT);

static ROUTE: CleanupRoute = CleanupRoute::new();

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
