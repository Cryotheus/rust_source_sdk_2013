//! TF2 Medi Gun targeting hooks: a say in whom a Medi Gun heals.
//!
//! A Medi Gun asks `CWeaponMedigun::AllowedToHealTarget` of each entity it
//! might heal: of what its search hits, and of its target each time it heals,
//! dropping a target it is no longer allowed. The game allows living players
//! of its owner's team, disguised Spies of the other team, and its team's
//! revive markers, as
//! [`MedigunTargets`](source_sdk_2013::raw::tf2::medigun_targets) describes.
//! The search has no team filter of its own, so allowing a player of the other
//! team is enough for the Medi Gun to heal them.
//!
//! [`MetamodApi::hook_medigun_targets`] runs a callback after each of these
//! questions, with what the game answered, and lets it answer otherwise. The
//! callback runs several times a frame for each Medic who heals, or holds the
//! fire button: ask it to be quick.
//!
//! `AllowedToHealTarget` is not virtual, so the hook detours it, which only
//! KHook does (Metamod 2.0): with SourceHook the install fails with
//! [`HookError::DetourUnsupported`]. As with other Metamod hooks, the callback
//! stops running while the plugin is paused, and the Medi Guns then heal whom
//! the game allows. A client predicts its own Medi Gun with the game's rule,
//! so a Medic healing a target only the callback allows may see their beam
//! falter until the server's target arrives.

use crate::MetamodApi;
use crate::hook::{DetouredFunction, Handler, HookAction, HookCall, HookError, HookId, HookTiming};
use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::medigun_targets::{
	AllowedToHealTargetFn, MedigunTargets, MedigunTargetsError,
};

use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;
use std::ptr::NonNull;

/// A callback-scoped server, a Medi Gun, the entity it asks about, and whether
/// the game allows it to heal that entity, returning whether it may. A panic is
/// contained by the hook dispatcher, and keeps the game's answer.
pub type MedigunTargetFn = for<'s> fn(Server<'s>, Entity<'s>, Entity<'s>, bool) -> bool;

static ROUTE: TargetRoute = TargetRoute {
	state: Cell::new(None),
};

/// Why a Medi Gun hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum MedigunHookError {
	/// The server does not run TF2.
	#[error("Medi Gun hooks require a TF2 server")]
	NotTf2,

	/// The game's `AllowedToHealTarget` was not found.
	#[error(transparent)]
	Unresolved(#[from] MedigunTargetsError),

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means it is
	/// installed, which callers installing more than once can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

#[derive(Clone, Copy)]
struct Routed {
	binding: ServerBinding,
	callback: MedigunTargetFn,
	hook: HookId,
}

struct TargetRoute {
	state: Cell<Option<Routed>>,
}

impl Handler<AllowedToHealTargetFn> for TargetRoute {
	fn call(&self, call: &HookCall<'_, AllowedToHealTargetFn>) -> HookAction<bool> {
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};

		let (target,) = call.args();
		let (Some(medigun), Some(target)) = (NonNull::new(call.this()), NonNull::new(target))
		else {
			return HookAction::Ignore;
		};

		let allowed = call.return_value().unwrap_or_default();
		let scope = ();

		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };

		// SAFETY: The detour supplies the live Medi Gun and the live entity it
		// asked about, which both stay in the entity list through the call.
		let (medigun, target) = unsafe {
			(
				Entity::from_live(server, medigun),
				Entity::from_live(server, target),
			)
		};

		let decided = (route.callback)(server, medigun, target, allowed);

		if decided == allowed {
			HookAction::Ignore
		} else {
			HookAction::Override(decided)
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for TargetRoute {}

impl MetamodApi<'_> {
	/// Runs `callback` after each time a Medi Gun asks whether it may heal an
	/// entity, which may answer otherwise; see the
	/// [module documentation](crate::hooks::tf2::medigun).
	///
	/// `binding` must describe the same running server as `server`. Finds the
	/// game's `AllowedToHealTarget`, which on Windows scans the game server
	/// module on the first call: install from a callback where that pause does
	/// not matter. Returns a removable hook ID. Installing again while the hook
	/// is installed returns [`HookError::AlreadyInstalled`]; removing the ID
	/// allows replacement.
	///
	/// The callback must not delete entities immediately, as [`Server::new`]
	/// requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the function already has a detour, including just after a reload. A
	/// returned ID means registration was accepted, not that the next question
	/// will be intercepted.
	pub fn hook_medigun_targets(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callback: MedigunTargetFn,
	) -> Result<HookId, MedigunHookError> {
		if binding.game() != Game::TeamFortress2 || server.game() != Game::TeamFortress2 {
			return Err(MedigunHookError::NotTf2);
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
		let targets = unsafe { MedigunTargets::cached(server.game_server_factory().as_raw()) }?;

		// SAFETY: The resolver verified the entry of `AllowedToHealTarget` in the
		// module, which stays loaded until Metamod unloads the plugin.
		let function = unsafe {
			DetouredFunction::<AllowedToHealTargetFn>::new(targets.allowed_to_heal_target())
		};

		let hook = self.add_function_hook(function, HookTiming::Post, &ROUTE)?;

		ROUTE.state.set(Some(Routed {
			binding,
			callback,
			hook,
		}));
		Ok(hook)
	}
}
