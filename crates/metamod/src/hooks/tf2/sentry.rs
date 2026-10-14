//! TF2 sentry targeting hooks: a say in whom a sentry gun shoots.
//!
//! A sentry looks for a target in `CObjectSentrygun::FindTarget`, as it
//! searches and as it attacks. The game's search covers dummies, the enemy
//! team's players, non-player bots (`INextBot`s) and the enemy team's
//! buildings, and returns whether the sentry has a target. A sentry whose
//! search returns `false` drops its target and goes on searching.
//!
//! [`MetamodApi::hook_sentry_targets`] runs a callback after each search, with
//! what the game decided, and lets it name another target: an entity the game
//! does not know to be a target, such as a prop. The hook then makes the sentry
//! attack it, as [`Sentry::acquire`] describes, and the search returns that the
//! sentry has a target, so the sentry keeps it. The callback runs for every
//! search, once or twice a sentry's think, while the sentry looks for targets
//! or attacks one: ask it to be quick, and give the same answer for a target it
//! keeps. [`Sentry::is_hunting`] tells the sentry is a candidate at all.
//!
//! `FindTarget` is not virtual, so the hook detours it, which only KHook does
//! (Metamod 2.0): with SourceHook the install fails with
//! [`HookError::DetourUnsupported`]. The function is found by
//! [`SentryTargets`](source_sdk_2013::raw::tf2::sentry_targets::SentryTargets).
//! As with other Metamod hooks, the callback stops running while the plugin is
//! paused, and the sentries then target as the game does.

use crate::MetamodApi;
use crate::hook::{DetouredFunction, Handler, HookAction, HookCall, HookError, HookId, HookTiming};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::sentry_targets::{FindTargetFn, SentryTargets, SentryTargetsError};
use source_sdk_2013::tf2::buildings::{Building, Sentry};
use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;

/// What a callback of [`MetamodApi::hook_sentry_targets`] decides.
#[derive(Debug, Clone, Copy)]
pub enum SentryTarget<'s> {
	/// Leaves the sentry with what the game decided.
	Keep,

	/// Has the sentry attack this entity.
	Attack(Entity<'s>),
}

/// A callback-scoped server, a sentry that has searched for a target, and
/// whether the game found it one, deciding whom the sentry attacks. A panic is
/// contained by the hook dispatcher, and keeps the game's decision.
pub type SentryTargetFn = for<'s> fn(Server<'s>, Entity<'s>, bool) -> SentryTarget<'s>;

/// Why a sentry hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum SentryHookError {
	/// The server does not run TF2.
	#[error("sentry hooks require a TF2 server")]
	NotTf2,

	/// The game's `FindTarget` was not found.
	#[error(transparent)]
	Unresolved(#[from] SentryTargetsError),

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means it is
	/// installed, which callers installing more than once can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

#[derive(Clone, Copy)]
struct Routed {
	binding: ServerBinding,
	callback: SentryTargetFn,
	hook: HookId,
}

static ROUTE: SearchRoute = SearchRoute {
	state: Cell::new(None),
};

struct SearchRoute {
	state: Cell<Option<Routed>>,
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for SearchRoute {}

impl Handler<FindTargetFn> for SearchRoute {
	fn call(&self, call: &HookCall<'_, FindTargetFn>) -> HookAction<bool> {
		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};

		let Some(sentry) = std::ptr::NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};

		let found = call.return_value().unwrap_or_default();
		let scope = ();

		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };

		// SAFETY: The detour supplies the live sentry whose search just ran,
		// which stays in the entity list through the call.
		let sentry = unsafe { Entity::from_live(server, sentry) };

		let SentryTarget::Attack(target) = (route.callback)(server, sentry, found) else {
			return HookAction::Ignore;
		};

		let acquired = Building::new(server, sentry)
			.and_then(Sentry::new)
			.and_then(|sentry| sentry.acquire(target));

		match acquired {
			Ok(()) => HookAction::Override(true),
			Err(_) => HookAction::Ignore,
		}
	}
}

impl MetamodApi<'_> {
	/// Runs `callback` after each search of a sentry gun for a target, which
	/// may name the entity it attacks; see the
	/// [module documentation](crate::hooks::tf2::sentry).
	///
	/// `binding` must describe the same running server as `server`. Finds the
	/// game's `FindTarget`, which on Windows scans the game server module on the
	/// first call: install from a callback where that pause does not matter.
	/// Returns a removable hook ID. Installing again while the hook is installed
	/// returns [`HookError::AlreadyInstalled`]; removing the ID allows
	/// replacement.
	///
	/// The callback must not delete entities immediately, as [`Server::new`]
	/// requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the function already has a detour, including just after a reload. A
	/// returned ID means registration was accepted, not that the next search
	/// will be intercepted.
	pub fn hook_sentry_targets(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		callback: SentryTargetFn,
	) -> Result<HookId, SentryHookError> {
		if binding.game() != Game::TeamFortress2 || server.game() != Game::TeamFortress2 {
			return Err(SentryHookError::NotTf2);
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
		let targets = unsafe { SentryTargets::cached(server.game_server_factory().as_raw()) }?;

		// SAFETY: The resolver verified the entry of `FindTarget` in the module,
		// which stays loaded until Metamod unloads the plugin.
		let function = unsafe { DetouredFunction::<FindTargetFn>::new(targets.find_target()) };

		let hook = self.add_function_hook(function, HookTiming::Post, &ROUTE)?;

		ROUTE.state.set(Some(Routed {
			binding,
			callback,
			hook,
		}));
		Ok(hook)
	}
}
