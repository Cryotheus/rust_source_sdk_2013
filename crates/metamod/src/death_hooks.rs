//! TF2 player death hooks, which run before or after the game's
//! `Event_Killed` with an owned copy of the damage that kills the player.
//!
//! Install for each distinct player class (for example on player spawn; bots
//! have a vtable of their own). Hooks cover that class, including
//! subsequently connected players of the same class, until removed or the
//! plugin unloads. As with other Metamod hooks, they stop calling handlers
//! while the plugin is paused.
//!
//! `CTFBot::Event_Killed` calls `CTFPlayer::Event_Killed` directly, not
//! through the vtable. Both hooking libraries patch vtable entries, so a bot's
//! death reaches only the bot class's hook, once. Dead Ringer feigned deaths
//! never call `Event_Killed`.

#[cfg(test)]
#[path = "tests/death_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::entities::health::{EVENT_KILLED_SLOT, EventKilledFn as EventKilled};
use source_sdk_2013::raw::util::vtable::vtable_pointer;
use source_sdk_2013::tf2::damage::{DamageEvent, DamageInfo};
use source_sdk_2013::{Game, Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

/// A callback-scoped server, the player about to die, and an owned copy of
/// the damage killing them. A panic is contained by the hook dispatcher.
pub type DyingFn = for<'s> fn(Server<'s>, Entity<'s>, &DamageInfo);

/// A callback-scoped server, the player who died, and an owned copy of the
/// damage that killed them. A panic is contained by the hook dispatcher.
pub type KilledFn = for<'s> fn(Server<'s>, Entity<'s>, &DamageInfo);

/// `Event_Killed` in a TF2 player's primary vtable.
const EVENT_KILLED: VirtualFunction<EventKilled> = VirtualFunction::new(EVENT_KILLED_SLOT);

/// The routes of the hooks before `Event_Killed`, one per hooked class.
static DYING_ROUTES: [DeathRoute; 8] = [const { DeathRoute::new() }; 8];

/// The routes of the hooks after `Event_Killed`, one per hooked class.
static KILLED_ROUTES: [DeathRoute; 8] = [const { DeathRoute::new() }; 8];

/// Why a death hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum DeathHookError {
	/// The server does not run TF2, or the entity is not a TF2 player.
	#[error("death hooks require a TF2 server and a CTFPlayer entity")]
	NotTfPlayer,

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means this
	/// player's class is already hooked, which callers installing on every
	/// player can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct DeathRoute {
	state: Cell<Option<RoutedDeath>>,
}

impl DeathRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<EventKilled> for DeathRoute {
	fn call(&self, call: &HookCall<'_, EventKilled>) -> HookAction<()> {
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (info,) = call.args();
		let (Some(victim), Some(info)) = (NonNull::new(call.this()), NonNull::new(info.cast_mut()))
		else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies a live player and its const damage
		// reference, which the game keeps live from its pre hooks through its
		// post hooks.
		unsafe { dispatch(server, route.callback, victim, info) };
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for DeathRoute {}

#[derive(Clone, Copy)]
struct RoutedDeath {
	binding: ServerBinding,
	callback: KilledFn,
	hook: HookId,
	vtable: usize,
}

impl MetamodApi<'_> {
	/// Runs `callback` before each death of a TF2 player of this player's
	/// class is handled, as `CTFPlayer::Event_Killed` is called: the player is
	/// still alive, and neither the game rules (`CTFGameRules::PlayerKilled`),
	/// which decide dominations and revenges and count the kill, nor the
	/// `player_death` event have seen the death.
	///
	/// The callback may change what the game reads as it handles the death,
	/// such as the victim's statistics, but not the damage, of which it gets a
	/// copy. Another plugin's pre hook may supersede the call, so that the
	/// player does not die after all.
	///
	/// Installs as [`Self::hook_player_killed`] does, with the same
	/// requirements, refusals, and caveats about KHook's activation, and a
	/// class of its own: either hook can be installed without the other.
	pub fn hook_player_dying(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		callback: DyingFn,
	) -> Result<HookId, DeathHookError> {
		self.hook_player_death(player, binding, callback, HookTiming::Pre)
	}

	/// Checks the player, then hooks `Event_Killed` on its class, at `timing`.
	fn hook_player_death(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		callback: KilledFn,
		timing: HookTiming,
	) -> Result<HookId, DeathHookError> {
		if binding.game() != Game::TeamFortress2
			|| !player
				.server_class()
				.is_some_and(|class| class.name() == c"CTFPlayer")
		{
			return Err(DeathHookError::NotTfPlayer);
		}

		// SAFETY: The player is a live TF2 player, whose vtable belongs to the
		// server DLL, which outlives this plugin.
		unsafe {
			self.install_death(
				NonNull::new(player.as_ptr()).unwrap(),
				binding,
				callback,
				timing,
			)
		}
	}

	/// Runs `callback` after each death of a TF2 player of this player's
	/// class, once `CTFPlayer::Event_Killed` has run: the player is dead, the
	/// `player_death` event was fired, and the `tf_ragdoll` from which clients
	/// make their ragdoll or gibs was created in the player's `m_hRagdoll`.
	///
	/// `player` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Install on each
	/// distinct class encountered, including bots. Repeating a class returns
	/// [`HookError::AlreadyInstalled`]; removing its ID allows replacement.
	///
	/// The hook runs after the call even if another plugin's hook superseded
	/// it, so that the player did not die. Callers that act on a death should
	/// check its effects, such as a fresh `tf_ragdoll`, before relying on it.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that the next death
	/// will be intercepted. KHook polls every 5 ms and can retry while a
	/// detour is busy, so deaths shortly after installation can be missed.
	pub fn hook_player_killed(
		self,
		player: Entity<'_>,
		binding: ServerBinding,
		callback: KilledFn,
	) -> Result<HookId, DeathHookError> {
		self.hook_player_death(player, binding, callback, HookTiming::Post)
	}

	/// Hooks `Event_Killed` on the class of `object`, before the call or after
	/// it, as `timing` says.
	///
	/// # Safety
	///
	/// `object` must be live, and its primary vtable must hold a function of
	/// the signature [`EventKilled`] at [`EVENT_KILLED_SLOT`], and stay loaded
	/// until Metamod unloads the plugin.
	unsafe fn install_death(
		self,
		object: NonNull<sys::CBaseEntity>,
		binding: ServerBinding,
		callback: KilledFn,
		timing: HookTiming,
	) -> Result<HookId, DeathHookError> {
		let routes = match timing {
			HookTiming::Pre => &DYING_ROUTES,
			HookTiming::Post => &KILLED_ROUTES,
		};

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
		// `void (const CTakeDamageInfo &)` at the slot, and stays loaded.
		let hook =
			unsafe { self.add_hook(EVENT_KILLED, HookTarget::class_of(object), timing, route) }?;
		route.state.set(Some(RoutedDeath {
			binding,
			callback,
			hook,
			vtable,
		}));
		Ok(hook)
	}
}

/// # Safety
/// The pointers must be the live arguments of a native `Event_Killed` call
/// that is about to run or has run, within the engine's invocation `server`
/// is scoped to.
unsafe fn dispatch(
	server: Server<'_>,
	callback: KilledFn,
	victim: NonNull<sys::CBaseEntity>,
	info: NonNull<sys::CTakeDamageInfo>,
) {
	// SAFETY: The caller supplies live callback-scoped arguments. Only
	// byte-copying reads the const damage object.
	let event = unsafe { DamageEvent::from_raw(server, victim, info) };
	callback(server, event.victim, &event.info);
}
