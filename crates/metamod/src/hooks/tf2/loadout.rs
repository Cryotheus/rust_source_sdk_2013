//! TF2 loadout-change hooks: a say in how a player's new loadout reaches them.
//!
//! A player's loadout lives in their Steam inventory, so the server hears of a
//! change from the game coordinator, which runs
//! `CTFPlayer::CheckInstantLoadoutRespawn` on the player. The game then
//! respawns a living player with their new loadout only in a respawn room of
//! their own team; elsewhere the loadout waits for their next respawn or
//! resupply.
//!
//! [`MetamodApi::hook_loadout_changes`] runs a callback before the game's
//! check, which may handle the change itself, as by resupplying the player
//! where they stand, and then skips the game's.
//!
//! The function is not virtual, so the hook detours it, which only KHook does
//! (Metamod 2.0): with SourceHook the install fails with
//! [`HookError::DetourUnsupported`]. The function is found by
//! [`LoadoutRespawn`](source_sdk_2013::raw::tf2::loadout_respawn::LoadoutRespawn).
//! As with other Metamod hooks, the callback stops running while the plugin is
//! paused, and loadout changes then go as the game has them.

use crate::MetamodApi;
use crate::hook::{DetouredFunction, Handler, HookAction, HookCall, HookError, HookId, HookTiming};
use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::loadout_respawn::{
	CheckInstantLoadoutRespawnFn, LoadoutRespawn, LoadoutRespawnError,
};

use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;

/// A callback-scoped server and the player whose loadout changed, returning
/// whether the callback handled the change, which skips the game's own check.
/// A panic is contained by the hook dispatcher, and leaves the change to the
/// game.
pub type LoadoutChangeFn = for<'s> fn(Server<'s>, Entity<'s>) -> bool;

static ROUTE: ChangeRoute = ChangeRoute {
	state: Cell::new(None),
};

struct ChangeRoute {
	state: Cell<Option<Routed>>,
}

impl Handler<CheckInstantLoadoutRespawnFn> for ChangeRoute {
	fn call(&self, call: &HookCall<'_, CheckInstantLoadoutRespawnFn>) -> HookAction<()> {
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};

		let Some(player) = std::ptr::NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};

		let scope = ();

		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };

		// SAFETY: The detour supplies the live player the game coordinator's job
		// found in the entity list, which stays there through the call.
		let player = unsafe { Entity::from_live(server, player) };

		match (route.callback)(server, player) {
			true => HookAction::Supersede(()),
			false => HookAction::Ignore,
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for ChangeRoute {}

/// Why a loadout-change hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum LoadoutHookError {
	/// The server does not run TF2.
	#[error("loadout-change hooks require a TF2 server")]
	NotTf2,

	/// The game's `CheckInstantLoadoutRespawn` was not found.
	#[error(transparent)]
	Unresolved(#[from] LoadoutRespawnError),

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means it is
	/// installed, which callers installing more than once can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

#[derive(Clone, Copy)]
struct Routed {
	binding: ServerBinding,
	callback: LoadoutChangeFn,
	hook: HookId,
}

impl MetamodApi<'_> {
	/// Runs `callback` before the game checks whether to respawn a player whose
	/// loadout changed, which may handle the change instead; see the
	/// [module documentation](crate::hooks::tf2::loadout).
	///
	/// `binding` must describe the same running server as `server`. Finds the
	/// game's `CheckInstantLoadoutRespawn`, which on Windows scans the game
	/// server module on the first call: install from a callback where that pause
	/// does not matter. Returns a removable hook ID. Installing again while the
	/// hook is installed returns [`HookError::AlreadyInstalled`]; removing the
	/// ID allows replacement.
	///
	/// The callback must not delete entities immediately, as [`Server::new`]
	/// requires.
	pub fn hook_loadout_changes(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callback: LoadoutChangeFn,
	) -> Result<HookId, LoadoutHookError> {
		if binding.game() != Game::TeamFortress2 || server.game() != Game::TeamFortress2 {
			return Err(LoadoutHookError::NotTf2);
		}

		if ROUTE
			.state
			.get()
			.is_some_and(|state| self.has_hook(state.hook))
		{
			return Err(HookError::AlreadyInstalled.into());
		}

		// SAFETY: `Server::new` guarantees that the game server module, whose
		// factory this is, stays loaded through the callback, and for a cached
		// resolution of the same factory and base, the image is the same one.
		let respawn = unsafe { LoadoutRespawn::cached(server.game_server_factory().as_raw()) }?;

		// SAFETY: The resolver verified the entry of `CheckInstantLoadoutRespawn`
		// in the module, which stays loaded until Metamod unloads the plugin.
		let function = unsafe {
			DetouredFunction::<CheckInstantLoadoutRespawnFn>::new(
				respawn.check_instant_loadout_respawn(),
			)
		};

		let hook = self.add_function_hook(function, HookTiming::Pre, &ROUTE)?;

		ROUTE.state.set(Some(Routed {
			binding,
			callback,
			hook,
		}));
		Ok(hook)
	}
}
