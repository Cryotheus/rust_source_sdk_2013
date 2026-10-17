//! Tests of `crate::hooks::tf2::projectile_target`: hooks changing what the
//! game takes an entity for, after the game, on a mock entity class, through
//! the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::{self, NonNull};

thread_local! {
	/// What [`on_target`] decides.
	static ACTION: Cell<TargetAction> = const { Cell::new(TargetAction::Continue) };

	/// The answers [`on_target`] was given since the last check.
	static SEEN: RefCell<Vec<bool>> = const { RefCell::new(Vec::new()) };
}

/// A game's method that answers no.
unsafe extern "C" fn game_no(_: *mut sys::CBaseEntity) -> bool {
	false
}

/// A game's method that answers yes.
unsafe extern "C" fn game_yes(_: *mut sys::CBaseEntity) -> bool {
	true
}

#[test]
fn hooks_change_the_game_s_answers_after_it() {
	/// An entity of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		// The combat item's slot answers no, the projectiles' target's yes.
		let mut slots =
			vec![game_no as PredicateFn as *mut c_void; IS_PROJECTILE_COLLISION_TARGET_SLOT + 1];
		slots[IS_PROJECTILE_COLLISION_TARGET_SLOT] = game_yes as PredicateFn as *mut c_void;
		let slots = Vec::leak(slots);
		let mut entity = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut entity).cast::<sys::CBaseEntity>();
		let binding = tf2_binding(no_interfaces);
		let combat_items = api.hook_combat_items(binding, on_target);
		let targets = api.hook_projectile_collision_targets(binding, on_target);
		// SAFETY: The test only calls the hooked slots, which hold methods of
		// their signature.
		let target =
			unsafe { ClassTarget::<BaseEntity>::from_raw(NonNull::new(entity.vtable).unwrap()) };
		let ask = |slot| {
			SEEN.take();
			let answer = harness.call::<PredicateFn>(this, slot, ());
			(answer, SEEN.take())
		};

		assert_eq!(combat_items.cover(api, target), Ok(true));
		assert_eq!(targets.cover(api, target), Ok(true));

		ACTION.set(TargetAction::Continue);
		assert_eq!(ask(IS_COMBAT_ITEM_SLOT), (false, vec![false]));
		assert_eq!(ask(IS_PROJECTILE_COLLISION_TARGET_SLOT), (true, vec![true]));

		ACTION.set(TargetAction::Count);
		assert_eq!(ask(IS_COMBAT_ITEM_SLOT), (true, vec![false]));

		ACTION.set(TargetAction::Exclude);
		assert_eq!(
			ask(IS_PROJECTILE_COLLISION_TARGET_SLOT),
			(false, vec![true])
		);
	});
}

/// The callback, which notes the game's answer and decides [`ACTION`].
fn on_target(_server: Server<'_>, _entity: Entity<'_>, counted: bool) -> TargetAction {
	SEEN.with_borrow_mut(|seen| seen.push(counted));
	ACTION.get()
}
