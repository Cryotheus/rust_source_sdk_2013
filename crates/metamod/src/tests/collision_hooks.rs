//! Tests of `crate::collision_hooks`: hooks deciding collisions before the
//! game, on mock entity classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::{self, NonNull};

thread_local! {
	/// What ran during the calls since the last check, in order, with the
	/// group and contents mask it was given.
	static CALLS: RefCell<Vec<(&'static str, c_int, c_uint)>> = const { RefCell::new(Vec::new()) };

	/// What [`on_should_collide`] decides.
	static ACTION: Cell<CollideAction> = const { Cell::new(CollideAction::Continue) };
}

/// The game's `ShouldCollide`, which notes that it ran, and collides with
/// group 5 alone.
unsafe extern "C" fn game_should_collide(
	_: *mut sys::CBaseEntity,
	group: c_int,
	contents: c_int,
) -> bool {
	CALLS.with_borrow_mut(|calls| calls.push(("game", group, contents.cast_unsigned())));
	group == 5
}

#[test]
fn hooks_decide_collisions_before_the_game() {
	/// An entity of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_should_collide as ShouldCollide as *mut c_void;
			SHOULD_COLLIDE_SLOT + 1
		]);
		let mut entity = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut entity).cast::<sys::CBaseEntity>();
		let hooks = api.hook_should_collide(tf2_binding(no_interfaces), on_should_collide);
		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature.
		let target =
			unsafe { ClassTarget::<BaseEntity>::from_raw(NonNull::new(entity.vtable).unwrap()) };
		// A contents mask with the sign bit set, which the hook gets unsigned.
		let contents = c_int::MIN | 1;
		let unsigned = contents.cast_unsigned();
		let collides = |group| {
			CALLS.take();
			let collides =
				harness.call::<ShouldCollide>(this, SHOULD_COLLIDE_SLOT, (group, contents));
			(collides, CALLS.take())
		};

		assert_eq!(hooks.cover(api, target), Ok(true));

		ACTION.set(CollideAction::Continue);
		assert_eq!(
			collides(5),
			(
				true,
				vec![("hook", 5, unsigned), ("game", 5, contents.cast_unsigned())]
			)
		);

		ACTION.set(CollideAction::Pass);
		assert_eq!(collides(5), (false, vec![("hook", 5, unsigned)]));

		ACTION.set(CollideAction::Collide);
		assert_eq!(collides(4), (true, vec![("hook", 4, unsigned)]));
	});
}

/// The callback, which notes the call and decides [`ACTION`].
fn on_should_collide(
	_server: Server<'_>,
	_entity: Entity<'_>,
	group: c_int,
	contents: c_uint,
) -> CollideAction {
	CALLS.with_borrow_mut(|calls| calls.push(("hook", group, contents)));
	ACTION.get()
}
