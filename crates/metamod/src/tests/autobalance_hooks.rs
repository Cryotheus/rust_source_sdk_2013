//! Tests of `crate::autobalance_hooks`: hooks changing whether players can be
//! autobalanced, on a mock player class, through the mock SourceHook and
//! KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::{self, NonNull};

thread_local! {
	/// What [`on_autobalance`] decides.
	static ACTION: Cell<AutobalanceAction> = const { Cell::new(AutobalanceAction::Continue) };

	/// The decisions [`on_autobalance`] was given since the last check.
	static SEEN: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

/// The game's `CanBeAutobalanced`, which allows it.
unsafe extern "C" fn game_can_be_autobalanced(_: *mut sys::CBaseEntity) -> bool {
	true
}

#[test]
fn autobalance_decisions_are_changed_after_the_game() {
	/// A player of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_can_be_autobalanced as PredicateFn as *mut c_void;
			CAN_BE_AUTOBALANCED_SLOT + 1
		]);
		let mut player = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut player).cast::<sys::CBaseEntity>();
		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature.
		let target =
			unsafe { ClassTarget::<TfPlayer>::from_raw(NonNull::new(player.vtable).unwrap()) };
		let hooks = api.hook_autobalance_checks(tf2_binding(no_interfaces), on_autobalance);
		let check = || {
			SEEN.take();
			let allowed = harness.call::<PredicateFn>(this, CAN_BE_AUTOBALANCED_SLOT, ());
			(allowed, SEEN.take())
		};

		assert_eq!(hooks.cover(api, target), Ok(true));

		ACTION.set(AutobalanceAction::Continue);
		assert_eq!(check(), (true, vec![true]));

		ACTION.set(AutobalanceAction::Refuse);
		assert_eq!(check(), (false, vec![true]));

		ACTION.set(AutobalanceAction::Allow);
		assert_eq!(check(), (true, vec![true]));
	});
}

/// The callback, which notes the game's decision and decides [`ACTION`].
fn on_autobalance(_server: Server<'_>, _player: Entity<'_>, allowed: bool) -> AutobalanceAction {
	SEEN.with_borrow_mut(|seen| seen.push(allowed));
	ACTION.get()
}
