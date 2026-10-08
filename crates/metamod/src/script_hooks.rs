//! TF2 entity script hooks, which run before the game's
//! `CBaseEntity::RunVScripts`, as an entity of a hooked class is about to run
//! the VScripts its `vscripts` key value names.
//!
//! The game runs an entity's scripts through `RunVScripts`, which it calls
//! through the entity's vtable as it spawns the entity: for the map's entities
//! as a level loads, again for those a round's restart creates, and for those
//! spawned later. A hook sees the entity before any of its scripts runs, while
//! its key values can still change which run: once its `vscripts` is empty,
//! the entity runs no script, no `thinkfunction`, and none of the scripts'
//! `Precache` and `OnPostSpawn`, which the game only runs for an entity that
//! names a script.
//!
//! Install for each entity class to hook, from the class's vtable, which
//! [`ScriptTargets`](source_sdk_2013::tf2::scripts::ScriptTargets) finds by the
//! class's C++ name even before any of its entities exists, such as
//! `CLogicScript` for `logic_script`. Hooks cover that class, including its
//! entities created later, until removed or the plugin unloads, but not the
//! classes deriving from it, which have vtables of their own. As with other
//! Metamod hooks, they stop calling handlers while the plugin is paused.

#[cfg(test)]
#[path = "tests/script_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::scripts::{RUN_VSCRIPTS_SLOT, RunVScriptsFn as RunVScripts};

use source_sdk_2013::tf2::scripts::ScriptTarget;

use source_sdk_2013::{Game, Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

/// A callback-scoped server, and the entity about to run the scripts its
/// `vscripts` key value names, whose key values the callback may change. A
/// panic is contained by the hook dispatcher.
pub type ScriptsFn = for<'s> fn(Server<'s>, Entity<'s>);

/// `RunVScripts` in a TF2 entity's primary vtable.
const RUN_VSCRIPTS: VirtualFunction<RunVScripts> = VirtualFunction::new(RUN_VSCRIPTS_SLOT);

static ROUTES: [ScriptRoute; 8] = [const { ScriptRoute::new() }; 8];

#[derive(Clone, Copy)]
struct RoutedScripts {
	binding: ServerBinding,
	callback: ScriptsFn,
	hook: HookId,
	vtable: usize,
}

/// Why a script hook could not be installed.
#[derive(Debug, thiserror::Error)]
pub enum ScriptHookError {
	/// The server does not run TF2, whose vtable slot of `RunVScripts` is the
	/// only one known.
	#[error("script hooks require a TF2 server")]
	NotTf2,

	/// Metamod refused the hook. [`HookError::AlreadyInstalled`] means the
	/// class is already hooked.
	#[error(transparent)]
	Hook(#[from] HookError),
}

struct ScriptRoute {
	state: Cell<Option<RoutedScripts>>,
}

impl ScriptRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<RunVScripts> for ScriptRoute {
	fn call(&self, call: &HookCall<'_, RunVScripts>) -> HookAction<()> {
		// An earlier hook already kept the scripts from running.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let Some(entity) = NonNull::new(call.this()) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies the live entity whose method is about
		// to run, which stays in the entity list through the call.
		let entity = unsafe { Entity::from_live(server, entity) };

		(route.callback)(server, entity);
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for ScriptRoute {}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CBaseEntity::RunVScripts` of a TF2 entity
	/// of `target`'s class: as the entity, spawning, is about to run the
	/// scripts its `vscripts` key value names. Find the vtables of the classes
	/// to hook together with
	/// [`ScriptTargets::find_all`](source_sdk_2013::tf2::scripts::ScriptTargets::find_all).
	///
	/// The callback may change the entity's key values, through
	/// [`ServerTools::set_key_value`](source_sdk_2013::interfaces::ServerTools::set_key_value).
	/// Emptying its `vscripts` keeps all of its scripts from running, and the
	/// rest of its spawn from looking for their functions.
	///
	/// `binding` must describe the running server the target was found in.
	/// Returns a removable hook ID. The hook holds no entity, so it lasts
	/// through level changes, until removed or the plugin unloads. Repeating a
	/// class returns [`HookError::AlreadyInstalled`]; removing its ID allows
	/// replacement. At most 8 classes can be hooked at once, past which
	/// installation returns [`HookError::TooManyFunctions`].
	///
	/// The callback is not run once an earlier hook superseded the call, as
	/// far as the hooking library reports it (see [`crate::hook`]). It runs
	/// while the game spawns the map's entities as a level loads, and inside a
	/// round's restart: it must not restart the round or delete entities
	/// immediately, as [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload.
	/// A returned ID means registration was accepted, not that the next call
	/// will be intercepted. KHook polls every 5 ms and can retry while a
	/// detour is busy, so calls shortly after installation can be missed.
	///
	/// # Safety
	///
	/// `target`'s class must derive from `CBaseEntity` through its primary
	/// bases, as the game's entity classes do, so that its vtable holds
	/// `RunVScripts` at [`RUN_VSCRIPTS_SLOT`]. The search only checks that the
	/// slot holds code, which any class with enough virtual methods has.
	pub unsafe fn hook_class_scripts(
		self,
		target: ScriptTarget<'_>,
		binding: ServerBinding,
		callback: ScriptsFn,
	) -> Result<HookId, ScriptHookError> {
		if binding.game() != Game::TeamFortress2 {
			return Err(ScriptHookError::NotTf2);
		}

		// SAFETY: The target is a primary vtable from TF2's game module, whose
		// class the caller promises is an entity class, so the slot holds its
		// `RunVScripts`. The game module stays loaded until Metamod unloads this
		// plugin.
		unsafe { self.install_scripts(target.as_ptr(), binding, callback) }
	}

	/// Hooks `RunVScripts` through `vtable`, for every entity of its class.
	///
	/// # Safety
	///
	/// `vtable` must be a live primary vtable that holds a function of the
	/// signature [`RunVScripts`] at [`RUN_VSCRIPTS_SLOT`], and stays loaded
	/// until Metamod unloads the plugin.
	unsafe fn install_scripts(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: ScriptsFn,
	) -> Result<HookId, ScriptHookError> {
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
		// SAFETY: As the caller promises, the vtable is live, has `void ()` at
		// the slot, and stays loaded.
		let hook = unsafe { self.add_hook(RUN_VSCRIPTS, target, HookTiming::Pre, route) }?;
		route.state.set(Some(RoutedScripts {
			binding,
			callback,
			hook,
			vtable: address,
		}));
		Ok(hook)
	}
}
