//! TF2 observer mode hooks, which run after the game's
//! `CTFPlayer::SetObserverMode` turned a player's roaming into first person
//! or a map-camera chase view.
//!
//! TF2 lets only spectators roam: when a player on a team asks to roam, as
//! cycling through the observer modes with the jump key does, the game follows
//! a teammate in first person or a non-player target in chase view instead (see
//! [`source_sdk_2013::tf2::observer`]). The hook's callback may then let the
//! player roam anyway, with
//! [`PlayerObserver::roam`](source_sdk_2013::tf2::observer::PlayerObserver::roam).
//!
//! Install for each distinct player class (for example on player spawn; bots
//! have a vtable of their own). Hooks cover that class, including
//! subsequently connected players of the same class, until removed or the
//! plugin unloads. As with other Metamod hooks, they stop calling handlers
//! while the plugin is paused.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/observer.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::{Entity, EntityHandle};

use source_sdk_2013::raw::tf2::observer::{
	OBS_MODE_CHASE, OBS_MODE_IN_EYE, OBS_MODE_POI, OBS_MODE_ROAMING, SET_OBSERVER_MODE_SLOT,
	SetObserverModeFn as SetObserverMode,
};

use source_sdk_2013::raw::util::vtable::vtable_pointer;
use source_sdk_2013::raw::vcall;
use source_sdk_2013::{Game, Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

/// A callback-scoped server and a player whom the game just put in first
/// person, or chase view of a non-player target, when they asked to roam.
/// A panic is contained by the hook dispatcher.
pub type RoamingRefusedFn = for<'s> fn(Server<'s>, Entity<'s>);

/// `SetObserverMode` in a TF2 player's primary vtable.
const SET_OBSERVER_MODE: VirtualFunction<SetObserverMode> =
	VirtualFunction::new(SET_OBSERVER_MODE_SLOT);

static ROUTES: [RoamingRoute; 8] = [const { RoamingRoute::new() }; 8];

/// Why an observer mode hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum ObserverHookError {
	/// The server does not run TF2, or the entity is not a TF2 player.
	#[error("observer mode hooks require a TF2 server and a CTFPlayer entity")]
	NotTfPlayer,

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means this
	/// player's class is already hooked, which callers installing on every
	/// player can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct RoamingRoute {
	state: Cell<Option<RoutedRoaming>>,
}

impl RoamingRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<SetObserverMode> for RoamingRoute {
	fn call(&self, call: &HookCall<'_, SetObserverMode>) -> HookAction<bool> {
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (requested,) = call.args();

		// Outside PASS Time, the game turns its point of interest into roaming
		// before it turns roaming into first person.
		if !matches!(requested, OBS_MODE_ROAMING | OBS_MODE_POI)
			|| call.return_value() != Some(true)
		{
			return HookAction::Ignore;
		}

		let Some(player) = NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};
		let this = player.as_ptr();

		// SAFETY: The class hook supplies the live player whose method just
		// ran, of the class whose vtable `install_roaming` checked.
		let mode = unsafe {
			vcall!(this as sys::CTFPlayer__bindgen_vtable => CTFPlayer_GetObserverMode())
		};

		if !matches!(mode, OBS_MODE_IN_EYE | OBS_MODE_CHASE) {
			return HookAction::Ignore;
		}

		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies the live player whose method just
		// ran, which stays in the entity list through the call.
		let player = unsafe { Entity::from_live(server, player) };

		if mode == OBS_MODE_CHASE {
			// TF2 forces in-eye views of non-player targets, including map
			// cameras, into chase. A chase view of another player remains the
			// game's policy; unresolved targets are left alone.
			let Ok(handle) = player.data_field::<EntityHandle>(c"m_hObserverTarget") else {
				return HookAction::Ignore;
			};
			let Some(target) = server
				.server_tools()
				.ok()
				.and_then(|tools| tools.entity_by_handle(handle))
			else {
				return HookAction::Ignore;
			};
			// SAFETY: The current handle resolved to a live CBaseEntity during
			// this callback; IsPlayer is its generated primary-vtable method
			// and does not delete entities or restart the round.
			let target = target.as_ptr();
			if unsafe {
				vcall!(target as sys::CBaseEntity__bindgen_vtable => CBaseEntity_IsPlayer())
			} {
				return HookAction::Ignore;
			}
		}

		(route.callback)(server, player);
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for RoamingRoute {}

#[derive(Clone, Copy)]
struct RoutedRoaming {
	binding: ServerBinding,
	callback: RoamingRefusedFn,
	hook: HookId,
	vtable: usize,
}

impl MetamodApi<'_> {
	/// Runs `callback` after each `CTFPlayer::SetObserverMode` of a TF2 player
	/// of this player's class that asked to roam, accepted the change, and
	/// left the player following their target in first person, or a non-player
	/// target in chase view: a player on a team, whom the game does not let
	/// roam, or one whose match settings forbid choosing a mode
	/// (`CTFPlayer::SetObserverMode`).
	///
	/// `player` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Install on each
	/// distinct class encountered, including bots. Repeating a class returns
	/// [`HookError::AlreadyInstalled`]; removing its ID allows replacement.
	///
	/// The callback may let the player roam with
	/// [`PlayerObserver::roam`](source_sdk_2013::tf2::observer::PlayerObserver::roam),
	/// which calls no observer mode method, so it does not run the hook again.
	/// Players ask to roam as they cycle through the modes, as often as they
	/// press the jump key, so the callback should be cheap. The game also
	/// returns a player to a mode it forced on them from their think: the
	/// callback must not delete entities or restart the round immediately, as
	/// [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that the next mode
	/// change will be intercepted. KHook polls every 5 ms and can retry while
	/// a detour is busy, so changes shortly after installation can be missed.
	pub fn hook_player_roaming_refused(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		callback: RoamingRefusedFn,
	) -> Result<HookId, ObserverHookError> {
		if binding.game() != Game::TeamFortress2
			|| !player
				.server_class()
				.is_some_and(|class| class.name() == c"CTFPlayer")
		{
			return Err(ObserverHookError::NotTfPlayer);
		}

		// SAFETY: The player is a live TF2 player, whose vtable belongs to the
		// server DLL, which outlives this plugin.
		unsafe { self.install_roaming(NonNull::new(player.as_ptr()).unwrap(), binding, callback) }
	}

	/// Hooks `SetObserverMode` on the class of `object`.
	///
	/// # Safety
	///
	/// `object` must be live, and its primary vtable must be laid out as
	/// `CTFPlayer`'s through [`SET_OBSERVER_MODE_SLOT`] and the slot of
	/// `GetObserverMode`, with functions of their signatures, and stay loaded
	/// until Metamod unloads the plugin.
	unsafe fn install_roaming(
		self,
		object: NonNull<sys::CBaseEntity>,
		binding: ServerBinding,
		callback: RoamingRefusedFn,
	) -> Result<HookId, ObserverHookError> {
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
		// `bool (int)` at the slot, and stays loaded.
		let hook = unsafe {
			self.add_hook(
				SET_OBSERVER_MODE,
				HookTarget::class_of(object),
				HookTiming::Post,
				route,
			)
		}?;
		route.state.set(Some(RoutedRoaming {
			binding,
			callback,
			hook,
			vtable,
		}));
		Ok(hook)
	}
}
