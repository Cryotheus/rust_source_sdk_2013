//! Tests of `crate::hooks::tf2::max_health`: hooks changing entities' maximum
//! health after the game, on a mock entity class, through the mock SourceHook
//! and KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};

thread_local! {
	/// The maximums [`on_max_health`] was given since the last check.
	static SEEN: RefCell<Vec<c_int>> = const { RefCell::new(Vec::new()) };
}

/// The game's `GetMaxHealth`, a Heavy's.
unsafe extern "C" fn game_max_health(_: *mut sys::CBaseEntity) -> c_int {
	300
}

#[test]
fn maximums_are_changed_after_the_game() {
	/// An entity of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_max_health as GetMaxHealth as *mut c_void;
			TF2_GET_MAX_HEALTH_SLOT + 1
		]);
		let mut entity = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut entity).cast::<sys::CBaseEntity>();
		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature.
		let target =
			unsafe { ClassTarget::<BaseEntity>::from_raw(NonNull::new(entity.vtable).unwrap()) };
		let binding = tf2_binding(no_interfaces);
		let first = api.hook_max_health(binding, on_max_health);
		let second = api.hook_max_health(binding, on_max_health);

		assert_eq!(first.cover(api, target), Ok(true));

		SEEN.take();
		assert_eq!(
			harness.call::<GetMaxHealth>(this, TF2_GET_MAX_HEALTH_SLOT, ()),
			150
		);
		assert_eq!(SEEN.take(), [300]);

		// A later hook sees the maximum the earlier one gave.
		assert_eq!(second.cover(api, target), Ok(true));
		assert_eq!(
			harness.call::<GetMaxHealth>(this, TF2_GET_MAX_HEALTH_SLOT, ()),
			75
		);
		assert_eq!(SEEN.take(), [300, 150]);
	});
}

/// The callback, which notes the maximum and halves it.
fn on_max_health(_server: Server<'_>, _entity: Entity<'_>, maximum: c_int) -> Option<c_int> {
	SEEN.with_borrow_mut(|seen| seen.push(maximum));
	Some(maximum / 2)
}
