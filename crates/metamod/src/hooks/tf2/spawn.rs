//! TF2 entity spawn hooks, which run after the game's `CBaseEntity::Spawn`
//! for the entities of one class.
//!
//! The game spawns an entity once it created it and gave it its keys, through
//! `DispatchSpawn`, which calls `Spawn` through the entity's vtable: the map's
//! entities as the level loads, and those the game creates as it runs, such
//! as projectiles and flames. `Spawn` sets what the class's entities start
//! with, such as their solid flags and collision group, overwriting what was
//! set on them as they were created, so changes to those belong after it.
//!
//! Find a class's vtable by its C++ name with
//! [`SpawnTargets`](source_sdk_2013::tf2::spawn::SpawnTargets): a hook covers
//! every entity of that class, including those created later, but not the
//! classes deriving from it, which have vtables of their own. Hooks hold no
//! entity, so they last through level changes, until removed or the plugin
//! unloads. As with other Metamod hooks, they stop calling handlers while the
//! plugin is paused.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/spawn.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::entities::{SPAWN_SLOT, SpawnFn as Spawn};
use source_sdk_2013::tf2::spawn::SpawnTarget;
use source_sdk_2013::{Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};

/// A callback-scoped server and the entity the game spawned. A panic is
/// contained by the hook dispatcher.
pub type SpawnFn = for<'s> fn(Server<'s>, Entity<'s>);

/// `Spawn` in an entity's primary vtable.
const SPAWN: VirtualFunction<Spawn> = VirtualFunction::new(SPAWN_SLOT);

static ROUTES: [SpawnRoute; 16] = [const { SpawnRoute::new() }; 16];

#[derive(Clone, Copy)]
struct RoutedSpawn {
	binding: ServerBinding,
	callback: SpawnFn,
	hook: HookId,
	vtable: usize,
}

struct SpawnRoute {
	state: Cell<Option<RoutedSpawn>>,
}

impl SpawnRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<Spawn> for SpawnRoute {
	fn call(&self, call: &HookCall<'_, Spawn>) -> HookAction<()> {
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
		// SAFETY: The class hook supplies the live entity whose `Spawn` ran,
		// which stays in the entity list through the call: the game only marks
		// an entity it removes then for deletion.
		let entity = unsafe { Entity::from_live(server, entity) };

		(route.callback)(server, entity);
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for SpawnRoute {}

impl MetamodApi<'_> {
	/// Runs `callback` after each `CBaseEntity::Spawn` of the entities of
	/// `target`'s class: once the game spawned one of them, or once a hook
	/// superseded its spawn.
	///
	/// `target` and `binding` must come from the same running server. Returns
	/// a removable hook ID: the hook stays until removed or the plugin
	/// unloads. Hooking a class with a callback it is already hooked with
	/// returns [`HookError::AlreadyInstalled`]; other callbacks can hook it
	/// too, and run in the order they were installed. At most 16 classes and
	/// callbacks can be hooked at once, past which installation returns
	/// [`HookError::TooManyFunctions`].
	///
	/// The callback may run as the game loads the level's entities, and
	/// before the code that created the entity is done with it: the entity
	/// may still lack what its creator sets after spawning it, such as a
	/// flame's damage. It must not delete entities immediately, as
	/// [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, including just after a reload. A
	/// returned ID means registration was accepted, not that the next spawn
	/// will be intercepted. KHook polls every 5 ms and can retry while a
	/// detour is busy, so spawns shortly after installation can be missed.
	///
	/// # Safety
	///
	/// `target`'s class must derive from `CBaseEntity` through its primary
	/// bases, as the game's entity classes do, so that its vtable holds
	/// `Spawn` at [`SPAWN_SLOT`] and its instances are entities. The search
	/// only checks that the slot holds code, which any class with enough
	/// virtual methods has.
	pub unsafe fn hook_spawns(
		self,
		target: SpawnTarget<'_>,
		binding: ServerBinding,
		callback: SpawnFn,
	) -> Result<HookId, HookError> {
		// SAFETY: The target is a primary vtable from TF2's game module, whose
		// class the caller promises is an entity class, so the slot holds its
		// `Spawn`. The game module stays loaded until Metamod unloads this
		// plugin, and the target holds no entity.
		unsafe { self.install_spawns(target.as_ptr(), binding, callback) }
	}

	/// Hooks `Spawn` after the game's, on the class `vtable` belongs to.
	///
	/// # Safety
	///
	/// `vtable` must be a live entity vtable with a function of the signature
	/// [`Spawn`] at [`SPAWN_SLOT`], and stay loaded until Metamod unloads the
	/// plugin.
	unsafe fn install_spawns(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: SpawnFn,
	) -> Result<HookId, HookError> {
		let address = vtable.addr().get();

		if ROUTES.iter().any(|route| {
			route.state.get().is_some_and(|state| {
				state.vtable == address
					&& ptr::fn_addr_eq(state.callback, callback)
					&& self.has_hook(state.hook)
			})
		}) {
			return Err(HookError::AlreadyInstalled);
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
		// SAFETY: As the caller promises, the vtable is live and has `Spawn` at
		// the slot, and stays loaded.
		let hook =
			unsafe { self.add_hook(SPAWN, HookTarget::vtable(vtable), HookTiming::Post, route) }?;

		route.state.set(Some(RoutedSpawn {
			binding,
			callback,
			hook,
			vtable: address,
		}));

		Ok(hook)
	}
}
