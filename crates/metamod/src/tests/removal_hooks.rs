//! Tests of `crate::removal_hooks`: hooks before entities' removal, on mock
//! entity classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};

thread_local! {
	/// What ran during the calls, in order, with the entity it ran for.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };
}

/// The game's `UpdateOnRemove`, which notes that it ran.
unsafe extern "C" fn game_update_on_remove(this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr())));
}

/// The callback, which notes the removal.
fn on_removal(_server: Server<'_>, entity: Entity<'_>) {
	CALLS.with_borrow_mut(|calls| calls.push(("removal", entity.as_ptr().addr())));
}

#[test]
fn removals_run_the_hook_before_the_game() {
	/// An entity of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_update_on_remove as EntityFn as *mut c_void;
			UPDATE_ON_REMOVE_SLOT + 1
		]);
		let mut entity = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut entity).cast::<sys::CBaseEntity>();
		let hooks = api.hook_removals(tf2_binding(no_interfaces), on_removal);
		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature.
		let target =
			unsafe { ClassTarget::<BaseEntity>::from_raw(NonNull::new(entity.vtable).unwrap()) };

		assert_eq!(hooks.cover(api, target), Ok(true));
		CALLS.take();
		harness.call::<EntityFn>(this, UPDATE_ON_REMOVE_SLOT, ());
		assert_eq!(
			CALLS.take(),
			[("removal", this.addr()), ("game", this.addr())]
		);
	});
}
