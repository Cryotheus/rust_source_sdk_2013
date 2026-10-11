//! TF2 entity touch hooks, which run before and after the game's
//! `CBaseEntity::Touch` for the entities of one class, and may block a touch.
//!
//! The engine calls `Touch` on an entity at every tick another touches it,
//! such as a player standing on an item or in a resupply zone. Find a class's
//! vtable by its C++ name with
//! [`TouchTargets`](source_sdk_2013::tf2::touch::TouchTargets): a hook covers
//! every entity of that class, but not of the classes deriving from it, which
//! have vtables of their own. Hooks hold no entity, so they last through level
//! changes, until removed or the plugin unloads. As with other Metamod hooks,
//! they stop calling handlers while the plugin is paused.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/touch.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::touch::{TOUCH_SLOT, TouchFn as Touch};
use source_sdk_2013::tf2::touch::TouchTarget;
use source_sdk_2013::{Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::rc::Rc;

/// A callback-scoped server, the stage of the touch, the touched entity and
/// the entity touching it. The action only matters before the game's touch.
/// A panic is contained by the hook dispatcher, and lets the touch happen.
pub type TouchFn = for<'s> fn(Server<'s>, TouchStage, Entity<'s>, Entity<'s>) -> TouchAction;

/// `Touch` in an entity's primary vtable.
const TOUCH: VirtualFunction<Touch> = VirtualFunction::new(TOUCH_SLOT);

static ROUTES: [TouchRoute; 64] = [const { TouchRoute::new() }; 64];

#[derive(Clone, Copy)]
struct RoutedTouch {
	binding: ServerBinding,
	callback: TouchFn,
	/// The hook before the game's touch, then the one after.
	hooks: [HookId; 2],
	vtable: usize,
}

/// What a touch hook does with a touch, before the game's.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TouchAction {
	/// Lets the game's touch run.
	#[default]
	Allow,

	/// Skips the game's touch, and the hooks of other plugins that would run
	/// after this one. The hooks after the game's still run.
	Block,
}

/// The hooks one [`MetamodApi::hook_touches`] installed.
///
/// Metamod disables them while paused and removes them before unloading the
/// plugin. [`Self::remove`] disables them earlier.
#[must_use = "retain touch hooks to support explicitly removing them"]
#[derive(Debug)]
pub struct TouchHooks {
	hooks: [HookId; 2],
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl TouchHooks {
	pub fn remove(self, api: MetamodApi<'_>) {
		for hook in self.hooks {
			api.remove_hook(hook);
		}

		for route in &ROUTES {
			if route
				.state
				.get()
				.is_some_and(|state| state.hooks == self.hooks)
			{
				route.state.set(None);
			}
		}
	}
}

struct TouchRoute {
	state: Cell<Option<RoutedTouch>>,
}

impl TouchRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<Touch> for TouchRoute {
	fn call(&self, call: &HookCall<'_, Touch>) -> HookAction<()> {
		let stage = match call.timing() {
			HookTiming::Pre => TouchStage::Before,
			HookTiming::Post => TouchStage::After,
		};

		// An earlier hook already blocked the touch.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some(route) = self.state.get() else {
			return HookAction::Ignore;
		};
		let (other,) = call.args();
		let (Some(entity), Some(other)) = (NonNull::new(call.this()), NonNull::new(other)) else {
			return HookAction::Ignore;
		};
		let scope = ();
		// SAFETY: The hook dispatcher runs on the main thread during one live
		// engine invocation. Binding was supplied during plugin integration.
		let server = unsafe { route.binding.server(&scope) };
		// SAFETY: The class hook supplies the live entity whose method runs, and
		// the live entity touching it, which stay in the entity list through the
		// call: the game only marks entities it removes then for deletion.
		let (entity, other) = unsafe {
			(
				Entity::from_live(server, entity),
				Entity::from_live(server, other),
			)
		};

		match ((route.callback)(server, stage, entity, other), stage) {
			(TouchAction::Block, TouchStage::Before) => HookAction::Supersede(()),
			_ => HookAction::Ignore,
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for TouchRoute {}

/// When a touch hook runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TouchStage {
	/// Before the game's touch, which the hook may block. Not run once an
	/// earlier hook blocked it, as far as the hooking library reports it (see
	/// [`crate::hook`]).
	Before,

	/// After the game's touch, or after a hook blocked it.
	After,
}

impl MetamodApi<'_> {
	/// Runs `callback` before and after each `CBaseEntity::Touch` of the
	/// entities of `target`'s class: as the game tells one of the class's
	/// entities of another touching it.
	///
	/// `target` and `binding` must come from the same running server. Returns
	/// the hooks, which stay until removed or the plugin unloads. Hooking a
	/// class with a callback it is already hooked with returns
	/// [`HookError::AlreadyInstalled`]; other callbacks can hook it too, and
	/// run in the order they were installed.
	///
	/// The game calls `Touch` at every tick two entities touch, so the
	/// callback should be cheap for the touches it ignores. It must not delete
	/// entities immediately, as [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, such as from the hook before the
	/// game's for the one after it. A returned value means registration was
	/// accepted, not that the next touch will be intercepted at both stages.
	/// KHook polls every 5 ms and can retry while a detour is busy, so touches
	/// shortly after installation can reach only one stage.
	///
	/// # Safety
	///
	/// `target`'s class must derive from `CBaseEntity` through its primary
	/// bases, as the game's entity classes do, so that its vtable holds `Touch`
	/// at [`TOUCH_SLOT`] and its instances are entities. The search only checks
	/// that the slot holds code, which any class with enough virtual methods
	/// has.
	pub unsafe fn hook_touches(
		self,
		target: TouchTarget<'_>,
		binding: ServerBinding,
		callback: TouchFn,
	) -> Result<TouchHooks, HookError> {
		// SAFETY: The target is a primary vtable from TF2's game module, whose
		// class the caller promises is an entity class, so the slot holds its
		// `Touch`. The game module stays loaded until Metamod unloads this
		// plugin, and the target holds no entity.
		unsafe { self.install_touches(target.as_ptr(), binding, callback) }
	}

	/// Hooks `Touch` before and after the game's, on the class `vtable`
	/// belongs to.
	///
	/// # Safety
	///
	/// `vtable` must be a live entity vtable with a function of the signature
	/// [`Touch`] at [`TOUCH_SLOT`], and stay loaded until Metamod unloads the
	/// plugin.
	unsafe fn install_touches(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: TouchFn,
	) -> Result<TouchHooks, HookError> {
		let address = vtable.addr().get();
		let installed = |state: RoutedTouch| state.hooks.iter().any(|&hook| self.has_hook(hook));

		if ROUTES.iter().any(|route| {
			route.state.get().is_some_and(|state| {
				state.vtable == address
					&& ptr::fn_addr_eq(state.callback, callback)
					&& installed(state)
			})
		}) {
			return Err(HookError::AlreadyInstalled);
		}

		let route = ROUTES
			.iter()
			.find(|route| route.state.get().is_none_or(|state| !installed(state)))
			.ok_or(HookError::TooManyFunctions)?;
		let target = HookTarget::vtable(vtable);
		// SAFETY: As the caller promises, the vtable is live and has `Touch` at
		// the slot, and stays loaded.
		let before = unsafe { self.add_hook(TOUCH, target, HookTiming::Pre, route) }?;
		// SAFETY: As above.
		let after = match unsafe { self.add_hook(TOUCH, target, HookTiming::Post, route) } {
			Ok(after) => after,

			Err(error) => {
				self.remove_hook(before);
				return Err(error);
			}
		};
		let hooks = [before, after];

		route.state.set(Some(RoutedTouch {
			binding,
			callback,
			hooks,
			vtable: address,
		}));

		Ok(TouchHooks {
			hooks,
			_not_thread_safe: PhantomData,
		})
	}
}
