//! Hooks on the native pusher's `CBaseEntity::Blocked` notification.
//!
//! Physics has already rolled an unsuccessful speculative move back before
//! invoking this callback. Suppressing it leaves movement pending; the next
//! unblocked push resumes the same trajectory. It skips the class's damage,
//! solver and reversal logic. This is not a placement or collision hook.
//! Each class uses two hook managers (before/after); registrations may activate
//! asynchronously on Metamod 2.0, and do not prove live interception.

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::blocked::{BLOCKED_SLOT, BlockedFn as Blocked};
use source_sdk_2013::tf2::blocked::BlockedTarget;
use source_sdk_2013::{Server, ServerBinding};
use std::cell::Cell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::{self, NonNull};
use std::rc::Rc;

/// A callback-scoped server, the notification stage, the pusher and
/// the blocking entity. The action only matters before the game's callback.
/// A panic is contained by the hook dispatcher, and lets the callback run.
pub type BlockedFn = for<'s> fn(Server<'s>, BlockedStage, Entity<'s>, Entity<'s>) -> BlockedAction;

/// `Blocked` in an entity's primary vtable.
const BLOCKED: VirtualFunction<Blocked> = VirtualFunction::new(BLOCKED_SLOT);

static ROUTES: [BlockedRoute; 64] = [const { BlockedRoute::new() }; 64];

/// What a blocked hook does with a notification, before the game's.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum BlockedAction {
	/// Lets the game's callback run.
	#[default]
	Allow,

	/// Skips the game's callback, and the hooks of other plugins that would run
	/// after this one. The hooks after the game's still run.
	Block,
}

/// The hooks one [`MetamodApi::hook_blocked`] installed.
///
/// Metamod disables them while paused and removes them before unloading the
/// plugin. [`Self::remove`] disables them earlier.
#[must_use = "retain blocked hooks to support explicitly removing them"]
#[derive(Debug)]
pub struct BlockedHooks {
	hooks: [HookId; 2],
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl BlockedHooks {
	/// Removes both registrations and releases their callback route.
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

struct BlockedRoute {
	state: Cell<Option<RoutedBlocked>>,
}

impl BlockedRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
		}
	}
}

impl Handler<Blocked> for BlockedRoute {
	fn call(&self, call: &HookCall<'_, Blocked>) -> HookAction<()> {
		let stage = match call.timing() {
			HookTiming::Pre => BlockedStage::Before,
			HookTiming::Post => BlockedStage::After,
		};

		// An earlier hook already suppressed the notification.
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
		// its live blocking entity, which stay in the entity list through the
		// call: the game only marks entities it removes then for deletion.
		let (entity, other) = unsafe {
			(
				Entity::from_live(server, entity),
				Entity::from_live(server, other),
			)
		};

		match ((route.callback)(server, stage, entity, other), stage) {
			(BlockedAction::Block, BlockedStage::Before) => HookAction::Supersede(()),
			_ => HookAction::Ignore,
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. Cell borrows are never held over calls.
unsafe impl Sync for BlockedRoute {}

/// When a blocked hook runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockedStage {
	/// Before the game's callback, which the hook may block. Not run once an
	/// earlier hook blocked it, as far as the hooking library reports it (see
	/// [`crate::hook`]).
	Before,

	/// After the game's callback, or after a hook blocked it.
	After,
}

#[derive(Clone, Copy)]
struct RoutedBlocked {
	binding: ServerBinding,
	callback: BlockedFn,
	/// The hook before the game's callback, then the one after.
	hooks: [HookId; 2],
	vtable: usize,
}

impl MetamodApi<'_> {
	/// Runs `callback` before and after each `CBaseEntity::Blocked` of the
	/// entities of `target`'s class: as the game tells one of the class's
	/// entities of its blocker.
	///
	/// `target` and `binding` must come from the same running server. Returns
	/// the hooks, which stay until removed or the plugin unloads. Hooking a
	/// class with a callback it is already hooked with returns
	/// [`HookError::AlreadyInstalled`]; other callbacks can hook it too, and
	/// run in the order they were installed.
	///
	/// The game calls `Blocked` at every tick a speculative push fails, so the
	/// callback should be cheap for the notifications it ignores. It must not delete
	/// entities immediately, as [`Server::new`] requires.
	///
	/// With Metamod 2.0, KHook can queue native activation on its worker when
	/// the vtable slot already has a detour, such as from the hook before the
	/// game's for the one after it. A returned value means registration was
	/// accepted, not that the next notification will be intercepted at both stages.
	/// KHook polls every 5 ms and can retry while a detour is busy, so notifications
	/// shortly after installation can reach only one stage.
	///
	/// # Safety
	///
	/// `target`'s class must derive from `CBaseEntity` through its primary
	/// bases, as the game's entity classes do, so that its vtable holds `Blocked`
	/// at [`BLOCKED_SLOT`] and its instances are entities. The search only checks
	/// that the slot holds code, which any class with enough virtual methods
	/// has.
	pub unsafe fn hook_blocked(
		self,
		target: BlockedTarget<'_>,
		binding: ServerBinding,
		callback: BlockedFn,
	) -> Result<BlockedHooks, HookError> {
		// SAFETY: The target is a primary vtable from TF2's game module, whose
		// class the caller promises is an entity class, so the slot holds its
		// `Blocked`. The game module stays loaded until Metamod unloads this
		// plugin, and the target holds no entity.
		unsafe { self.install_blocked(target.as_ptr(), binding, callback) }
	}

	/// Hooks `Blocked` before and after the game's, on the class `vtable`
	/// belongs to.
	///
	/// # Safety
	///
	/// `vtable` must be a live entity vtable with a function of the signature
	/// [`Blocked`] at [`BLOCKED_SLOT`], and stay loaded until Metamod unloads the
	/// plugin.
	unsafe fn install_blocked(
		self,
		vtable: NonNull<*mut c_void>,
		binding: ServerBinding,
		callback: BlockedFn,
	) -> Result<BlockedHooks, HookError> {
		let address = vtable.addr().get();
		let installed = |state: RoutedBlocked| state.hooks.iter().any(|&hook| self.has_hook(hook));

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
		// SAFETY: As the caller promises, the vtable is live and has `Blocked` at
		// the slot, and stays loaded.
		let before = unsafe { self.add_hook(BLOCKED, target, HookTiming::Pre, route) }?;
		// SAFETY: As above.
		let after = match unsafe { self.add_hook(BLOCKED, target, HookTiming::Post, route) } {
			Ok(after) => after,

			Err(error) => {
				self.remove_hook(before);
				return Err(error);
			}
		};
		let hooks = [before, after];

		route.state.set(Some(RoutedBlocked {
			binding,
			callback,
			hooks,
			vtable: address,
		}));

		Ok(BlockedHooks {
			hooks,
			_not_thread_safe: PhantomData,
		})
	}
}

#[cfg(test)]
mod tests {
	//! Tests of `crate::hooks::tf2::blocked`: hooks of `Blocked` before and after the
	//! game's, on mock entity classes, through the mock SourceHook and KHook.

	use super::*;
	use crate::test_support::harness::{Harness, on_both};
	use crate::test_support::server::{no_interfaces, tf2_binding};
	use source_sdk_2013::sys;
	use std::cell::RefCell;

	thread_local! {
		/// What ran during the calls since the last [`blocked`], in order, with the
		/// pusher and its blocker.
		static CALLS: RefCell<Vec<(&'static str, usize, usize)>> = const { RefCell::new(Vec::new()) };

		/// The address of the entity whose notifications [`on_blocked`] blocks.
		static BLOCKED: Cell<usize> = const { Cell::new(0) };
	}

	/// An entity of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	impl Mock {
		/// An entity of a new class, whose vtable holds `blocked` at [`BLOCKED_SLOT`].
		fn of_new_class(blocked: Blocked) -> Box<Self> {
			let slots = Vec::leak(vec![blocked as *mut c_void; BLOCKED_SLOT + 1]);

			Box::new(Self {
				vtable: slots.as_mut_ptr(),
			})
		}

		fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
			NonNull::from(self).cast()
		}

		fn vtable(&self) -> NonNull<*mut c_void> {
			NonNull::new(self.vtable).unwrap()
		}
	}

	/// Another callback, which only notes that it ran.
	fn also_on_blocked(
		_server: Server<'_>,
		stage: BlockedStage,
		entity: Entity<'_>,
		other: Entity<'_>,
	) -> BlockedAction {
		if stage == BlockedStage::Before {
			note("also", entity.as_ptr().addr(), other.as_ptr().addr());
		}

		BlockedAction::Block
	}

	/// Calls `entity`'s hooked `Blocked` with `other`, and returns what ran.
	fn blocked(
		harness: &Harness,
		entity: &mut Mock,
		other: &mut Mock,
	) -> Vec<(&'static str, usize, usize)> {
		CALLS.take();
		harness.call::<Blocked>(entity.ptr().as_ptr(), BLOCKED_SLOT, (other.ptr().as_ptr(),));
		CALLS.take()
	}

	/// The game's `Blocked`, which notes that it ran.
	unsafe extern "C" fn game_blocked(this: *mut sys::CBaseEntity, other: *mut sys::CBaseEntity) {
		note("game", this.addr(), other.addr());
	}

	/// Notes a call of `name` for the pusher and its blocker.
	fn note(name: &'static str, entity: usize, other: usize) {
		CALLS.with_borrow_mut(|calls| calls.push((name, entity, other)));
	}

	#[test]
	fn notifications_run_between_the_hooks_unless_blocked() {
		on_both(|harness| {
			let api = harness.api();
			let mut item = Mock::of_new_class(game_blocked);
			let mut zone = Mock::of_new_class(game_blocked);
			let mut player = Mock::of_new_class(game_blocked);
			let (item_address, zone_address, player_address) = (
				item.ptr().addr().get(),
				zone.ptr().addr().get(),
				player.ptr().addr().get(),
			);

			for mock in [&item, &zone] {
				// SAFETY: The mock classes have `Blocked` at the slot, and are leaked.
				let _ = unsafe {
					api.install_blocked(mock.vtable(), tf2_binding(no_interfaces), on_blocked)
				}
				.unwrap();
			}

			// A class is hooked once per callback, so that each notification reaches it
			// once.
			assert!(matches!(
				// SAFETY: As above.
				unsafe {
					api.install_blocked(item.vtable(), tf2_binding(no_interfaces), on_blocked)
				},
				Err(HookError::AlreadyInstalled)
			));

			BLOCKED.set(zone_address);
			assert_eq!(
				blocked(harness, &mut item, &mut player),
				[
					("before", item_address, player_address),
					("game", item_address, player_address),
					("after", item_address, player_address),
				]
			);

			// A superseded notification skips the game's callback, but keeps the post hook.
			assert_eq!(
				blocked(harness, &mut zone, &mut player),
				[
					("before", zone_address, player_address),
					("after", zone_address, player_address)
				]
			);

			// Another callback hooks the class too, after the first, and no longer
			// runs once removed.
			// SAFETY: As above.
			let also = unsafe {
				api.install_blocked(item.vtable(), tf2_binding(no_interfaces), also_on_blocked)
			}
			.unwrap();

			assert_eq!(
				blocked(harness, &mut item, &mut player),
				[
					("before", item_address, player_address),
					("also", item_address, player_address),
					("after", item_address, player_address),
				]
			);

			also.remove(api);
			assert_eq!(
				blocked(harness, &mut item, &mut player),
				[
					("before", item_address, player_address),
					("game", item_address, player_address),
					("after", item_address, player_address),
				]
			);
		});
	}

	/// The callback, which notes the stage, and blocks the notifications of [`BLOCKED`].
	fn on_blocked(
		_server: Server<'_>,
		stage: BlockedStage,
		entity: Entity<'_>,
		other: Entity<'_>,
	) -> BlockedAction {
		let entity = entity.as_ptr().addr();

		note(
			match stage {
				BlockedStage::Before => "before",
				BlockedStage::After => "after",
			},
			entity,
			other.as_ptr().addr(),
		);

		if entity == BLOCKED.get() {
			BlockedAction::Block
		} else {
			BlockedAction::Allow
		}
	}
}
