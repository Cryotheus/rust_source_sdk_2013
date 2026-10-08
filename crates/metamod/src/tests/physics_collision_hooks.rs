//! Tests of `crate::physics_collision_hooks`: hooks forcing physics
//! collisions before the game, on mock entity classes, through the mock
//! SourceHook and KHook.

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
	/// entity asked and the other entity.
	static CALLS: RefCell<Vec<(&'static str, usize, usize)>> = const { RefCell::new(Vec::new()) };

	/// What [`on_force_collide`] decides.
	static ACTION: Cell<ForceCollideAction> = const { Cell::new(ForceCollideAction::Continue) };
}

/// The game's `ForceVPhysicsCollide`, which notes that it ran, and forces no
/// pair, as `CBaseEntity`'s.
unsafe extern "C" fn game_force_collide(
	this: *mut sys::CBaseEntity,
	other: *mut sys::CBaseEntity,
) -> bool {
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr(), other.addr())));
	false
}

#[test]
fn hooks_force_collisions_before_the_game() {
	/// An entity of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_force_collide as ForceVPhysicsCollide
				as *mut c_void;
			FORCE_VPHYSICS_COLLIDE_SLOT + 1
		]);
		let mut entity = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let mut other = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut entity).cast::<sys::CBaseEntity>();
		let other = ptr::from_mut(&mut other).cast::<sys::CBaseEntity>();
		let hooks = api.hook_force_physics_collisions(tf2_binding(no_interfaces), on_force_collide);
		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature.
		let target =
			unsafe { ClassTarget::<BaseEntity>::from_raw(NonNull::new(entity.vtable).unwrap()) };
		let forced = |other| {
			CALLS.take();
			let forced =
				harness.call::<ForceVPhysicsCollide>(this, FORCE_VPHYSICS_COLLIDE_SLOT, (other,));
			(forced, CALLS.take())
		};
		let (this_addr, other_addr) = (this.addr(), other.addr());

		assert_eq!(hooks.cover(api, target), Ok(true));

		ACTION.set(ForceCollideAction::Continue);
		assert_eq!(
			forced(other),
			(
				false,
				vec![
					("hook", this_addr, other_addr),
					("game", this_addr, other_addr)
				]
			)
		);

		ACTION.set(ForceCollideAction::Collide);
		assert_eq!(forced(other), (true, vec![("hook", this_addr, other_addr)]));

		// Without another entity, the game decides alone.
		assert_eq!(
			forced(ptr::null_mut()),
			(false, vec![("game", this_addr, 0)])
		);
	});
}

/// The callback, which notes the call and decides [`ACTION`].
fn on_force_collide(
	_server: Server<'_>,
	entity: Entity<'_>,
	other: Entity<'_>,
) -> ForceCollideAction {
	CALLS.with_borrow_mut(|calls| {
		calls.push(("hook", entity.as_ptr().addr(), other.as_ptr().addr()));
	});
	ACTION.get()
}
