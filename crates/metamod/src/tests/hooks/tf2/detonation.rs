//! Tests of `crate::hooks::tf2::detonation`: hooks keeping buildings from
//! detonating, on a mock building class, through the mock SourceHook and
//! KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};

thread_local! {
	/// What [`on_detonation`] decides.
	static ACTION: Cell<DetonationAction> = const { Cell::new(DetonationAction::Continue) };

	/// How many times the game's `DetonateObject` ran since the last check.
	static DETONATED: Cell<u32> = const { Cell::new(0) };

	/// How many times [`on_detonation`] was asked since the last check.
	static ASKED: Cell<u32> = const { Cell::new(0) };
}

/// The game's `DetonateObject`, which counts itself.
unsafe extern "C" fn game_detonate(_: *mut sys::CBaseEntity) {
	DETONATED.set(DETONATED.get() + 1);
}

#[test]
fn kept_buildings_skip_the_games_detonation() {
	/// A building of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_detonate as EntityFn as *mut c_void;
			DETONATE_OBJECT_SLOT + 1
		]);
		let mut building = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut building).cast::<sys::CBaseEntity>();
		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature.
		let target =
			unsafe { ClassTarget::<BaseObject>::from_raw(NonNull::new(building.vtable).unwrap()) };
		let hooks = api.hook_building_detonations(tf2_binding(no_interfaces), on_detonation);
		let detonate = || {
			harness.call::<EntityFn>(this, DETONATE_OBJECT_SLOT, ());
			(ASKED.take(), DETONATED.take())
		};

		assert_eq!(hooks.cover(api, target), Ok(true));

		ACTION.set(DetonationAction::Continue);
		assert_eq!(detonate(), (1, 1));

		ACTION.set(DetonationAction::Keep);
		assert_eq!(detonate(), (1, 0));

		ACTION.set(DetonationAction::Continue);
		assert_eq!(detonate(), (1, 1));
	});
}

/// The callback, which counts itself and decides [`ACTION`].
fn on_detonation(_server: Server<'_>, _building: Entity<'_>) -> DetonationAction {
	ASKED.set(ASKED.get() + 1);
	ACTION.get()
}
