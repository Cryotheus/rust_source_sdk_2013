//! Tests of `crate::think_hooks`: hooks around players' thinks and entities'
//! simulations, on mock entity classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};

thread_local! {
	/// What ran during the calls since the last [`run`], in order.
	static CALLS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// An entity of a C++ class, as far as hooks know it.
#[repr(C)]
struct Mock {
	vtable: *mut *mut c_void,
}

impl Mock {
	/// An entity of a new class, whose vtable holds [`game_method`] in every
	/// slot up to the last of those hooked.
	fn of_new_class() -> Box<Self> {
		let last = PRE_THINK_SLOT
			.max(POST_THINK_SLOT)
			.max(PHYSICS_SIMULATE_SLOT)
			.max(THINK_SLOT);
		let slots = Vec::leak(vec![game_method as EntityFn as *mut c_void; last + 1]);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> *mut sys::CBaseEntity {
		ptr::from_mut(self).cast()
	}

	/// The mock's class, as a class of the kind `K`.
	fn target<K: source_sdk_2013::tf2::class_targets::ClassKind>(&self) -> ClassTarget<'static, K> {
		// SAFETY: The tests only call the hooked slots, which hold a method of
		// their signature.
		unsafe { ClassTarget::from_raw(NonNull::new(self.vtable).unwrap()) }
	}
}

/// The game's method, which notes that it ran.
unsafe extern "C" fn game_method(_: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push("game"));
}

/// The callback, which notes the timing.
fn on_think(_server: Server<'_>, timing: HookTiming, _entity: Entity<'_>) {
	CALLS.with_borrow_mut(|calls| {
		calls.push(match timing {
			HookTiming::Pre => "before",
			HookTiming::Post => "after",
		})
	});
}

#[test]
fn entity_thinks_run_between_the_hooks() {
	on_both(|harness| {
		let api = harness.api();
		let mut entity = Mock::of_new_class();
		let hooks = api.hook_thinks(tf2_binding(no_interfaces), on_think);

		assert_eq!(hooks.cover(api, entity.target::<BaseEntity>()), Ok(true));
		assert_eq!(
			run(harness, &mut entity, THINK_SLOT),
			["before", "game", "after"]
		);

		// The simulation around the think is not hooked.
		assert_eq!(run(harness, &mut entity, PHYSICS_SIMULATE_SLOT), ["game"]);

		// Removed, the hooks no longer run.
		hooks.remove(api);
		assert_eq!(run(harness, &mut entity, THINK_SLOT), ["game"]);
	});
}

#[test]
fn player_thinks_run_between_the_hooks() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Mock::of_new_class();
		let post =
			api.hook_player_thinks(PlayerThink::PostThink, tf2_binding(no_interfaces), on_think);

		assert_eq!(post.cover(api, player.target::<TfPlayer>()), Ok(true));
		assert_eq!(
			run(harness, &mut player, POST_THINK_SLOT),
			["before", "game", "after"]
		);

		// The other think is not hooked.
		assert_eq!(run(harness, &mut player, PRE_THINK_SLOT), ["game"]);

		let pre =
			api.hook_player_thinks(PlayerThink::PreThink, tf2_binding(no_interfaces), on_think);

		assert_eq!(pre.cover(api, player.target::<TfPlayer>()), Ok(true));
		assert_eq!(
			run(harness, &mut player, PRE_THINK_SLOT),
			["before", "game", "after"]
		);
	});
}

/// Calls the method at `slot` of `entity`, and returns what ran.
fn run(harness: &Harness, entity: &mut Mock, slot: usize) -> Vec<&'static str> {
	CALLS.take();
	harness.call::<EntityFn>(entity.ptr(), slot, ());
	CALLS.take()
}

#[test]
fn simulations_run_between_the_hooks() {
	on_both(|harness| {
		let api = harness.api();
		let mut entity = Mock::of_new_class();
		let hooks = api.hook_simulations(tf2_binding(no_interfaces), on_think);

		assert_eq!(hooks.cover(api, entity.target::<BaseEntity>()), Ok(true));
		assert_eq!(
			run(harness, &mut entity, PHYSICS_SIMULATE_SLOT),
			["before", "game", "after"]
		);
	});
}
