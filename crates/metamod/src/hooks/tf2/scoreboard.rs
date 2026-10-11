//! TF2 scoreboard hooks: the player resource's updates of the scoreboard,
//! which they run before.
//!
//! TF2's player resource, `tf_player_manager` (`CTFPlayerResource`), updates
//! the scoreboard at its think, ten times a second
//! (`player_resource.cpp:94-101`): it copies each player's frags and deaths,
//! scores their statistics, and reports what each Score gained as Strange
//! progress, as [`source_sdk_2013::tf2::scoreboard`] describes. A callback
//! that sets the statistics, then rebases the Scores, has the update show
//! them without reporting anything. The resource also updates outside its
//! think as a Mann vs. Machine wave completes and as a matchmade game reports
//! its result (`tf_player_resource.cpp:70-80`, `tf_gamerules.cpp:2575`),
//! which the hook does not run before.
//!
//! The engine runs an entity's think through `CBaseEntity::Think`, which calls
//! the entity's think function. The hook patches `Think` in the primary vtable
//! of the resource's class, which no other entity has, before the resource's
//! think function runs. The game creates the resource as each level loads and
//! keeps it through rounds, so install the hook once a level is loaded: it
//! covers the resource of every later level, of the same class, until removed
//! or the plugin unloads. As with other Metamod hooks, it stops calling back
//! while the plugin is paused, so the game updates the scoreboard as usual
//! then.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/scoreboard.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::entities::{THINK_SLOT, ThinkFn as Think};
use source_sdk_2013::raw::util::vtable::vtable_pointer;
use source_sdk_2013::{Game, InterfaceError, Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::{CStr, c_void};
use std::ptr::NonNull;

/// A callback-scoped server and its player resource, which is about to update
/// the scoreboard. A panic is contained by the hook dispatcher, and lets the
/// update run.
pub type ScoreboardUpdateFn = for<'s> fn(Server<'s>, Entity<'s>);

/// The class name of TF2's player resource (`tf_player_resource.cpp:52`).
const RESOURCE_CLASS_NAME: &CStr = c"tf_player_manager";

/// The server class of TF2's player resource.
const RESOURCE_SERVER_CLASS: &CStr = c"CTFPlayerResource";

/// `Think` in the player resource's primary vtable.
const THINK: VirtualFunction<Think> = VirtualFunction::new(THINK_SLOT);

static ROUTE: UpdateRoute = UpdateRoute::new();

/// Why a scoreboard hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum ScoreboardHookError {
	/// The server does not run TF2.
	#[error("scoreboard hooks require a TF2 server")]
	NotTf2,

	/// No player resource exists, so no level is loaded.
	#[error("no `tf_player_manager` entity of the class `CTFPlayerResource` exists")]
	NoPlayerResource,

	/// The interface that finds the player resource is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means the
	/// player resource's class is already hooked, which callers installing
	/// more than once can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

#[derive(Clone, Copy)]
struct RoutedUpdate {
	binding: ServerBinding,
	callback: ScoreboardUpdateFn,
	hook: HookId,
	vtable: usize,
}

struct UpdateRoute {
	state: Cell<Option<RoutedUpdate>>,
}

impl UpdateRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<Think> for UpdateRoute {
	fn call(&self, call: &HookCall<'_, Think>) -> HookAction<()> {
		// An earlier hook already skipped the think.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let Some(resource) = NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies the live resource whose think runs,
		// which stays in the entity list through the call.
		let resource = unsafe { Entity::from_live(server, resource) };

		(route.callback)(server, resource);
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for UpdateRoute {}

impl MetamodApi<'_> {
	/// Runs `callback` before each think of TF2's player resource, which
	/// updates the scoreboard; see the
	/// [module documentation](crate::hooks::tf2::scoreboard).
	///
	/// `binding` must describe the same running server as `server`. Finds the
	/// player resource, so a level must be loaded, as from the game's
	/// `ServerActivate` on. Returns a removable hook ID. Installing again while
	/// the hook is installed returns [`HookError::AlreadyInstalled`] without
	/// looking for the resource; removing the ID allows replacement.
	///
	/// The callback is not run once an earlier hook skipped the think, as far
	/// as the hooking library reports it (see [`crate::hook`]). It runs among
	/// the entities' thinks: it must not delete entities immediately, as
	/// [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that the next
	/// update will be intercepted. KHook polls every 5 ms and can retry while
	/// a detour is busy, so updates shortly after installation can be missed.
	pub fn hook_scoreboard_updates(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callback: ScoreboardUpdateFn,
	) -> Result<HookId, ScoreboardHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(ScoreboardHookError::NotTf2);
		}

		// The resource's class is the same for the whole load.
		if ROUTE
			.state
			.get()
			.is_some_and(|state| self.has_hook(state.hook))
		{
			return Err(HookError::AlreadyInstalled.into());
		}

		let resource = server
			.server_tools()?
			.find_by_class_name(None, RESOURCE_CLASS_NAME)
			.filter(|resource| {
				resource
					.server_class()
					.is_some_and(|class| class.name() == RESOURCE_SERVER_CLASS)
			})
			.ok_or(ScoreboardHookError::NoPlayerResource)?;

		// SAFETY: The resource is a live `CTFPlayerResource`, whose vtable
		// belongs to the server DLL, which outlives this plugin.
		Ok(unsafe {
			self.install_scoreboard_updates(
				NonNull::new(resource.as_ptr()).unwrap(),
				binding,
				callback,
			)
		}?)
	}

	/// Hooks `Think` on the class of `resource`.
	///
	/// # Safety
	///
	/// `resource` must be live, and its primary vtable must hold a function of
	/// the signature [`Think`] at [`THINK_SLOT`], and stay loaded until Metamod
	/// unloads the plugin.
	unsafe fn install_scoreboard_updates(
		self,
		resource: NonNull<sys::CBaseEntity>,
		binding: ServerBinding,
		callback: ScoreboardUpdateFn,
	) -> Result<HookId, HookError> {
		// SAFETY: The resource is live, and starts with its primary vtable
		// pointer, of which only the address is used.
		let vtable = unsafe { vtable_pointer::<c_void>(resource.as_ptr()) }.addr();

		match ROUTE.state.get() {
			Some(state) if self.has_hook(state.hook) => {
				return Err(match state.vtable == vtable {
					true => HookError::AlreadyInstalled,
					false => HookError::TooManyFunctions,
				});
			}

			_ => {}
		}

		// SAFETY: As the caller promises.
		let hook = unsafe {
			self.add_hook(
				THINK,
				HookTarget::class_of(resource),
				HookTiming::Pre,
				&ROUTE,
			)
		}?;

		ROUTE.state.set(Some(RoutedUpdate {
			binding,
			callback,
			hook,
			vtable,
		}));
		Ok(hook)
	}
}
