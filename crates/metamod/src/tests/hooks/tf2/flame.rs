//! Tests of `crate::hooks::tf2::flame`: hooks deciding what flames touch
//! before the game, on a mock point manager class, through the mock SourceHook
//! and KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr;

thread_local! {
	/// What ran during the calls since the last check, in order, with the
	/// point manager and the entity it overlaps.
	static CALLS: RefCell<Vec<(&'static str, usize, usize)>> = const { RefCell::new(Vec::new()) };

	/// What [`on_flame_collide`] decides.
	static ACTION: Cell<CollideAction> = const { Cell::new(CollideAction::Continue) };
}

/// The game's `ShouldCollide`, which notes that it ran, and touches nothing,
/// as the flames' does for a prop.
unsafe extern "C" fn game_should_collide(
	this: *mut sys::CBaseEntity,
	other: *mut sys::CBaseEntity,
) -> bool {
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr(), other.addr())));
	false
}

#[test]
fn hooks_decide_what_flames_touch_before_the_game() {
	/// An entity of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_should_collide as PointShouldCollide as *mut c_void;
			POINT_SHOULD_COLLIDE_SLOT + 1
		]);
		let mut flames = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let mut prop = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut flames).cast::<sys::CBaseEntity>();
		let prop = ptr::from_mut(&mut prop).cast::<sys::CBaseEntity>();
		let hooks = api.hook_flame_collisions(tf2_binding(no_interfaces), on_flame_collide);
		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature.
		let target = unsafe {
			ClassTarget::<TfPointManager>::from_raw(NonNull::new(flames.vtable).unwrap())
		};
		let touches = |other| {
			CALLS.take();
			let touches =
				harness.call::<PointShouldCollide>(this, POINT_SHOULD_COLLIDE_SLOT, (other,));
			(touches, CALLS.take())
		};
		let (this_addr, prop_addr) = (this.addr(), prop.addr());

		assert_eq!(hooks.cover(api, target), Ok(true));

		ACTION.set(CollideAction::Continue);
		assert_eq!(
			touches(prop),
			(
				false,
				vec![
					("hook", this_addr, prop_addr),
					("game", this_addr, prop_addr)
				]
			)
		);

		ACTION.set(CollideAction::Collide);
		assert_eq!(touches(prop), (true, vec![("hook", this_addr, prop_addr)]));

		ACTION.set(CollideAction::Pass);
		assert_eq!(touches(prop), (false, vec![("hook", this_addr, prop_addr)]));

		// Without an entity, the game decides alone.
		ACTION.set(CollideAction::Collide);
		assert_eq!(
			touches(ptr::null_mut()),
			(false, vec![("game", this_addr, 0)])
		);
	});
}

/// The callback, which notes the call and decides [`ACTION`].
fn on_flame_collide(_server: Server<'_>, flames: Entity<'_>, other: Entity<'_>) -> CollideAction {
	CALLS.with_borrow_mut(|calls| {
		calls.push(("hook", flames.as_ptr().addr(), other.as_ptr().addr()));
	});
	ACTION.get()
}
