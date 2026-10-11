//! TF2 player respawn hooks, which run before the game's
//! `CTFPlayer::ForceRespawn` and may refuse the respawn.
//!
//! Every way TF2 brings a dead player back calls `ForceRespawn` through the
//! player's vtable: respawn waves, a round's restart, `game_forcerespawn`'s
//! inputs, `trigger_player_respawn_override`, choosing a class, the script
//! bindings' `ForceRespawn` and `ForceRegenerateAndRespawn`, and Mann vs.
//! Machine's revives and buybacks. Only direct calls of the player's `Spawn`
//! bypass it: a player's first spawn as it is put in the server, and scripts'
//! `DispatchSpawn` of a player.
//!
//! Install for each distinct player class (for example on player spawn; bots
//! have a vtable of their own). Hooks cover that class, including
//! subsequently connected players of the same class, until removed or the
//! plugin unloads. As with other Metamod hooks, they stop calling handlers
//! while the plugin is paused.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/respawn.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::respawn::{FORCE_RESPAWN_SLOT, ForceRespawnFn as ForceRespawn};
use source_sdk_2013::raw::util::vtable::vtable_pointer;
use source_sdk_2013::{Game, Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

/// A callback-scoped server and the player about to respawn, which decides
/// whether they do. A panic is contained by the hook dispatcher.
pub type RespawnFn = for<'s> fn(Server<'s>, Entity<'s>) -> RespawnAction;

/// `ForceRespawn` in a TF2 player's primary vtable.
const FORCE_RESPAWN: VirtualFunction<ForceRespawn> = VirtualFunction::new(FORCE_RESPAWN_SLOT);

static ROUTES: [RespawnRoute; 8] = [const { RespawnRoute::new() }; 8];

/// What a respawn hook does with a respawn.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum RespawnAction {
	/// Lets the game respawn the player.
	#[default]
	Allow,

	/// Skips `ForceRespawn`: the player stays as they are, dead or not, and
	/// none of the function's effects apply, including those it applies
	/// before its own checks, such as its respawn statistic and the player's
	/// spawn time.
	Refuse,
}

/// Why a respawn hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum RespawnHookError {
	/// The server does not run TF2, or the entity is not a TF2 player.
	#[error("respawn hooks require a TF2 server and a CTFPlayer entity")]
	NotTfPlayer,

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means this
	/// player's class is already hooked, which callers installing on every
	/// player can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct RespawnRoute {
	state: Cell<Option<RoutedRespawn>>,
}

impl RespawnRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<ForceRespawn> for RespawnRoute {
	fn call(&self, call: &HookCall<'_, ForceRespawn>) -> HookAction<()> {
		// An earlier hook already refused the respawn.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

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
		// SAFETY: The class hook supplies the live player whose method is about
		// to run, which stays in the entity list through the call.
		let player = unsafe { Entity::from_live(server, player) };

		match (route.callback)(server, player) {
			RespawnAction::Allow => HookAction::Ignore,
			RespawnAction::Refuse => HookAction::Supersede(()),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for RespawnRoute {}

#[derive(Clone, Copy)]
struct RoutedRespawn {
	binding: ServerBinding,
	callback: RespawnFn,
	hook: HookId,
	vtable: usize,
}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CTFPlayer::ForceRespawn` of a TF2 player
	/// of this player's class, which respawns the player unless the callback
	/// returns [`RespawnAction::Refuse`].
	///
	/// `player` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Install on each
	/// distinct class encountered, including bots. Repeating a class returns
	/// [`HookError::AlreadyInstalled`]; removing its ID allows replacement.
	///
	/// The callback is not run once an earlier hook refused the respawn, as
	/// far as the hooking library reports it (see [`crate::hook`]).
	///
	/// Every caller of `ForceRespawn` copes with a respawn that does not
	/// happen. During a round's preparation, the game tries again at each of
	/// its thinks, and so does a map's `trigger_player_respawn_override` while
	/// the round runs, so the callback may run every tick for each player it
	/// refuses: it should be cheap. It may also run inside a round's restart,
	/// once the map's cleanup finished: it must not restart the round or
	/// delete entities immediately, as [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that the next
	/// respawn will be intercepted. KHook polls every 5 ms and can retry while
	/// a detour is busy, so respawns shortly after installation can be missed.
	pub fn hook_player_respawn(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		callback: RespawnFn,
	) -> Result<HookId, RespawnHookError> {
		if binding.game() != Game::TeamFortress2
			|| !player
				.server_class()
				.is_some_and(|class| class.name() == c"CTFPlayer")
		{
			return Err(RespawnHookError::NotTfPlayer);
		}

		// SAFETY: The player is a live TF2 player, whose vtable belongs to the
		// server DLL, which outlives this plugin.
		unsafe { self.install_respawn(NonNull::new(player.as_ptr()).unwrap(), binding, callback) }
	}

	/// Hooks `ForceRespawn` on the class of `object`.
	///
	/// # Safety
	///
	/// `object` must be live, and its primary vtable must hold a function of
	/// the signature [`ForceRespawn`] at [`FORCE_RESPAWN_SLOT`], and stay
	/// loaded until Metamod unloads the plugin.
	unsafe fn install_respawn(
		self,
		object: NonNull<sys::CBaseEntity>,
		binding: ServerBinding,
		callback: RespawnFn,
	) -> Result<HookId, RespawnHookError> {
		// SAFETY: The object is live, and starts with its primary vtable
		// pointer, of which only the address is used.
		let vtable = unsafe { vtable_pointer::<c_void>(object.as_ptr()) }.addr();
		if ROUTES.iter().any(|route| {
			route
				.state
				.get()
				.is_some_and(|state| state.vtable == vtable && self.has_hook(state.hook))
		}) {
			return Err(HookError::AlreadyInstalled.into());
		}
		let route = ROUTES
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
				FORCE_RESPAWN,
				HookTarget::class_of(object),
				HookTiming::Pre,
				route,
			)
		}?;
		route.state.set(Some(RoutedRespawn {
			binding,
			callback,
			hook,
			vtable,
		}));
		Ok(hook)
	}
}
