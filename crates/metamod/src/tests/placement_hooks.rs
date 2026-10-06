//! Tests of `crate::placement_hooks`: hooks of `IsPlacementPosValid` on the
//! vtables of mock building classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use std::cell::RefCell;

thread_local! {
	/// What ran during the calls since the last [`is_valid`], in order.
	static CALLS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// A building of a C++ class, as hooks know it.
#[repr(C)]
struct Building {
	vtable: *mut *mut c_void,
}

impl Building {
	/// A building of a new class, whose vtable holds `check` at
	/// [`IS_PLACEMENT_POS_VALID_SLOT`].
	fn of_new_class(check: IsPlacementPosValid) -> Box<Self> {
		let slots = Vec::leak(vec![check as *mut c_void; IS_PLACEMENT_POS_VALID_SLOT + 1]);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> *mut sys::CBaseEntity {
		(&raw mut *self).cast()
	}

	fn vtable(&self) -> NonNull<*mut c_void> {
		NonNull::new(self.vtable).unwrap()
	}
}

/// The game's check, which notes that it ran, and allows every position.
unsafe extern "C" fn game_check(_this: *mut sys::CBaseEntity) -> bool {
	CALLS.with_borrow_mut(|calls| calls.push("game"));
	true
}

/// Calls `building`'s hooked check, and returns its result and what ran.
fn is_valid(harness: &Harness, building: &mut Building) -> (bool, Vec<&'static str>) {
	CALLS.take();

	let valid =
		harness.call::<IsPlacementPosValid>(building.ptr(), IS_PLACEMENT_POS_VALID_SLOT, ());

	(valid, CALLS.take())
}

/// The callback, which leaves dispensers to the game, allows sentry guns
/// without the game's check, and refuses teleporters where the game's check
/// allows them.
fn on_placement(_server: Server<'_>, placement: Placement<'_>) -> PlacementAction {
	CALLS.with_borrow_mut(|calls| calls.push("callback"));

	match placement.kind() {
		ObjectKind::Dispenser => PlacementAction::Continue,
		ObjectKind::Sentry => PlacementAction::Allow,

		ObjectKind::Teleporter => {
			if placement.check() {
				PlacementAction::Refuse
			} else {
				PlacementAction::Allow
			}
		}
	}
}

#[test]
fn placements_are_checked_by_the_game_or_the_callback() {
	on_both(|harness| {
		let api = harness.api();
		let mut dispenser = Building::of_new_class(game_check);
		let mut sentry = Building::of_new_class(game_check);
		let mut teleporter = Building::of_new_class(game_check);
		let targets = [
			(ObjectKind::Dispenser, dispenser.vtable()),
			(ObjectKind::Sentry, sentry.vtable()),
			(ObjectKind::Teleporter, teleporter.vtable()),
		];

		// SAFETY: The mock classes have the check at the slot, and are leaked.
		let hooks =
			unsafe { api.install_placement(&targets, tf2_binding(no_interfaces), on_placement) }
				.unwrap();

		// The buildings are hooked once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_placement(&targets, tf2_binding(no_interfaces), on_placement) },
			Err(PlacementHookError::Hook(HookError::AlreadyInstalled))
		));

		assert_eq!(
			is_valid(harness, &mut dispenser),
			(true, vec!["callback", "game"])
		);
		assert_eq!(is_valid(harness, &mut sentry), (true, vec!["callback"]));

		// The callback's own check skips the hook, and its decision stands.
		assert_eq!(
			is_valid(harness, &mut teleporter),
			(false, vec!["callback", "game"])
		);

		// Removed hooks leave the check to the game, and can be installed again.
		hooks.remove(api);
		assert_eq!(is_valid(harness, &mut sentry), (true, vec!["game"]));

		// SAFETY: As above.
		unsafe { api.install_placement(&targets, tf2_binding(no_interfaces), on_placement) }
			.unwrap()
			.remove(api);
	});
}
