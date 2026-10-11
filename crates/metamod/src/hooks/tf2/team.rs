//! TF2 entity team hooks, which run before the game's
//! `CBaseEntity::ChangeTeam` and may put the entity on another team, or keep
//! it on its own.
//!
//! The game changes an entity's team through `ChangeTeam`, which it calls
//! through the entity's vtable: for the `SetTeam` input, when the entity is
//! activated with the team its `TeamNum` key value gives, and in classes of
//! its own, such as a respawn room taking the team of the spawn points inside
//! it as each round starts, and moving its visualizers to its team. A class
//! that overrides the method, as respawn rooms do, is hooked at its override,
//! before it does more than store the team. The `teamnumber` key value stores
//! the team without the method, and so does the game wherever it assigns the
//! member itself.
//!
//! Install for each entity class to hook, from one of its entities, or from
//! the class's vtable, which
//! [`TeamTargets`](source_sdk_2013::tf2::teams::TeamTargets) finds by the
//! class's C++ name even before any of its entities exists. Hooks cover that
//! class, including its entities created later, such as those a round's
//! restart creates again, until removed or the plugin unloads, but not the
//! classes deriving from it, which have vtables of their own. As with other
//! Metamod hooks, they stop calling handlers while the plugin is paused.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/team.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::teams::{CHANGE_TEAM_SLOT, ChangeTeamFn as ChangeTeam};

use source_sdk_2013::tf2::teams::TeamTarget;

use source_sdk_2013::raw::util::vtable::vtable_pointer;
use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;
use std::ffi::{c_int, c_void};
use std::ptr::NonNull;

/// A callback-scoped server, and the entity about to be put on the team
/// numbered as the last argument, which decides what team it goes on. A panic
/// is contained by the hook dispatcher.
pub type TeamChangeFn = for<'s> fn(Server<'s>, Entity<'s>, c_int) -> TeamChange;

/// `ChangeTeam` in a TF2 entity's primary vtable.
const CHANGE_TEAM: VirtualFunction<ChangeTeam> = VirtualFunction::new(CHANGE_TEAM_SLOT);

static ROUTES: [TeamRoute; 16] = [const { TeamRoute::new() }; 16];

#[derive(Clone, Copy)]
struct RoutedTeam {
	binding: ServerBinding,
	callback: TeamChangeFn,
	hook: HookId,
	original: ChangeTeam,
	vtable: usize,
}

/// What a team hook does with a change of an entity's team.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TeamChange {
	/// Lets the game put the entity on the team it asked for.
	#[default]
	Allow,

	/// Puts the entity on this team instead, through the class's own
	/// `ChangeTeam`, which does what it does for any change, such as a
	/// respawn room moving its visualizers. The other plugins' hooks on the
	/// call do not run, unless this is the team the game asked for, which
	/// [`Self::Allow`]s the change.
	Replace(c_int),

	/// Skips `ChangeTeam`: the entity stays on its team, and none of what the
	/// class does for a change happens.
	Refuse,
}

/// Why a team hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum TeamHookError {
	/// The server does not run TF2, whose vtable slot of `ChangeTeam` is the
	/// only one known.
	#[error("team hooks require a TF2 server")]
	NotTf2,

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means the
	/// entity's class is already hooked, which callers installing on every
	/// entity of a class can ignore.
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct TeamRoute {
	state: Cell<Option<RoutedTeam>>,
}

impl TeamRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<ChangeTeam> for TeamRoute {
	fn call(&self, call: &HookCall<'_, ChangeTeam>) -> HookAction<()> {
		// An earlier hook already decided the change.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let Some(entity) = NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};
		let (team,) = call.args();
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies the live entity whose method is about
		// to run, which stays in the entity list through the call.
		let entity = unsafe { Entity::from_live(server, entity) };

		match (route.callback)(server, entity, team) {
			TeamChange::Allow => HookAction::Ignore,
			TeamChange::Replace(replacement) if replacement == team => HookAction::Ignore,

			TeamChange::Replace(replacement) => {
				// SAFETY: The original is the unhooked `ChangeTeam` of this
				// entity's class, called on the main thread with the live entity,
				// as the game called it.
				unsafe { (route.original)(entity.as_ptr(), replacement) };
				HookAction::Supersede(())
			}

			TeamChange::Refuse => HookAction::Supersede(()),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for TeamRoute {}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CBaseEntity::ChangeTeam` of a TF2 entity
	/// of this entity's class, which puts the entity on the team the callback
	/// decides.
	///
	/// `entity` must come from this server, and `binding` must describe the
	/// same running server. Returns a removable hook ID. Install on each class
	/// to hook. Repeating a class returns [`HookError::AlreadyInstalled`];
	/// removing its ID allows replacement. At most 16 classes can be hooked at
	/// once, past which installation returns [`HookError::TooManyFunctions`].
	/// [`Self::hook_class_team_changes`] hooks a class without any of its
	/// entities.
	///
	/// The callback is not run once an earlier hook superseded the change, as
	/// far as the hooking library reports it (see [`crate::hook`]). It may run
	/// while the game activates the map's entities, and inside a round's
	/// restart: it must not restart the round or delete entities immediately,
	/// as [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that the next change
	/// will be intercepted. KHook polls every 5 ms and can retry while a
	/// detour is busy, so changes shortly after installation can be missed.
	pub fn hook_team_changes(
		self,
		entity: Entity<'_>,
		binding: ServerBinding,
		callback: TeamChangeFn,
	) -> Result<HookId, TeamHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(TeamHookError::NotTf2);
		}

		// SAFETY: The entity is live, and starts with its primary vtable pointer.
		let vtable = unsafe { vtable_pointer::<*mut c_void>(entity.as_ptr()) };
		let vtable = NonNull::new(vtable.cast_mut()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: The entity is live on a TF2 server, whose entities' vtables
		// belong to the server DLL, which outlives this plugin.
		unsafe { self.install_team(vtable, binding, callback) }
	}

	/// Runs `callback` before each `CBaseEntity::ChangeTeam` of a TF2 entity
	/// of `target`'s class, as [`Self::hook_team_changes`] does, but from the
	/// class's vtable, so that it covers the class's entities created later
	/// even when none exists yet. Find the vtables of the classes to hook
	/// together with
	/// [`TeamTargets::find_all`](source_sdk_2013::tf2::teams::TeamTargets::find_all).
	///
	/// `binding` must describe the running server the target was found in. The
	/// hook holds no entity, so it lasts through level changes, until removed
	/// or the plugin unloads. A class hooked from one of its entities is
	/// already installed, and the other way around.
	///
	/// # Safety
	///
	/// `target`'s class must derive from `CBaseEntity` through its primary
	/// bases, as the game's entity classes do, so that its vtable holds
	/// `ChangeTeam` at [`CHANGE_TEAM_SLOT`]. The search only checks that the
	/// slot holds code, which any class with enough virtual methods has.
	pub unsafe fn hook_class_team_changes(
		self,
		target: TeamTarget<'_>,
		binding: ServerBinding,
		callback: TeamChangeFn,
	) -> Result<HookId, TeamHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(TeamHookError::NotTf2);
		}

		// SAFETY: The target is a primary vtable from TF2's game module, whose
		// class the caller promises is an entity class, so the slot holds its
		// `ChangeTeam`. The game module stays loaded until Metamod unloads this
		// plugin.
		unsafe { self.install_team(target.as_ptr(), binding, callback) }
	}

	/// Hooks `ChangeTeam` through `vtable`, for every entity of its class.
	///
	/// # Safety
	///
	/// `vtable` must be a live primary vtable that holds a function of the
	/// signature [`ChangeTeam`] at [`CHANGE_TEAM_SLOT`], and stays loaded until
	/// Metamod unloads the plugin.
	unsafe fn install_team(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: TeamChangeFn,
	) -> Result<HookId, TeamHookError> {
		let address = vtable.as_ptr().addr();
		if ROUTES.iter().any(|route| {
			route
				.state
				.get()
				.is_some_and(|state| state.vtable == address && self.has_hook(state.hook))
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
		let target = HookTarget::vtable(vtable);
		// SAFETY: As the caller promises, the vtable is live, has `void (int)` at
		// the slot, and stays loaded.
		let original = unsafe { self.original_function(CHANGE_TEAM, target) }?;
		// SAFETY: As above.
		let hook = unsafe { self.add_hook(CHANGE_TEAM, target, HookTiming::Pre, route) }?;
		route.state.set(Some(RoutedTeam {
			binding,
			callback,
			hook,
			original,
			vtable: address,
		}));
		Ok(hook)
	}
}
