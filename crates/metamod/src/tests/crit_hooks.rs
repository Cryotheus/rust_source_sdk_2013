//! Tests of `crate::crit_hooks`: hooks changing crit decisions after the
//! game, on a mock weapon class, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::{self, NonNull};

thread_local! {
	/// What [`on_crit`] decides.
	static ACTION: Cell<CritAction> = const { Cell::new(CritAction::Continue) };

	/// The decisions [`on_crit`] was given since the last check.
	static SEEN: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

/// The game's `CalcIsAttackCriticalHelper`, which rolls no crit.
unsafe extern "C" fn game_random_crits(_: *mut sys::CBaseEntity) -> bool {
	false
}

/// The game's `CalcIsAttackCriticalHelperNoCrits`, which finds a crit boost.
unsafe extern "C" fn game_no_random_crits(_: *mut sys::CBaseEntity) -> bool {
	true
}

#[test]
fn crit_decisions_are_changed_after_the_game() {
	/// A weapon of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_random_crits as PredicateFn as *mut c_void;
			CALC_IS_ATTACK_CRITICAL_HELPER_NO_CRITS_SLOT + 1
		]);
		slots[CALC_IS_ATTACK_CRITICAL_HELPER_NO_CRITS_SLOT] =
			game_no_random_crits as PredicateFn as *mut c_void;
		let mut weapon = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut weapon).cast::<sys::CBaseEntity>();
		// SAFETY: The test only calls the hooked slots, which hold methods of
		// their signature.
		let target =
			unsafe { ClassTarget::<TfWeapon>::from_raw(NonNull::new(weapon.vtable).unwrap()) };
		let binding = tf2_binding(no_interfaces);
		let crits = |check: CritCheck| {
			SEEN.take();
			let crit = harness.call::<PredicateFn>(this, check.function().index(), ());
			(crit, SEEN.take())
		};

		for check in [CritCheck::RandomCrits, CritCheck::NoRandomCrits] {
			let hooks = api.hook_crit_checks(check, binding, on_crit);
			assert_eq!(hooks.cover(api, target), Ok(true));
		}

		ACTION.set(CritAction::Continue);
		assert_eq!(crits(CritCheck::RandomCrits), (false, vec![false]));
		assert_eq!(crits(CritCheck::NoRandomCrits), (true, vec![true]));

		ACTION.set(CritAction::Crit);
		assert_eq!(crits(CritCheck::RandomCrits), (true, vec![false]));

		ACTION.set(CritAction::NoCrit);
		assert_eq!(crits(CritCheck::NoRandomCrits), (false, vec![true]));
	});
}

/// The callback, which notes the game's decision and decides [`ACTION`].
fn on_crit(_server: Server<'_>, _weapon: Entity<'_>, crit: bool) -> CritAction {
	SEEN.with_borrow_mut(|seen| seen.push(crit));
	ACTION.get()
}
