//! Tests of `crate::touch_hooks`: hooks of `Touch` before and after the
//! game's, on mock entity classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use std::cell::RefCell;

thread_local! {
	/// What ran during the calls since the last [`touch`], in order, with the
	/// touched entity and the one touching it.
	static CALLS: RefCell<Vec<(&'static str, usize, usize)>> = const { RefCell::new(Vec::new()) };

	/// The address of the entity whose touches [`on_touch`] blocks.
	static BLOCKED: Cell<usize> = const { Cell::new(0) };
}

/// An entity of a C++ class, as far as hooks know it.
#[repr(C)]
struct Mock {
	vtable: *mut *mut c_void,
}

impl Mock {
	/// An entity of a new class, whose vtable holds `touch` at [`TOUCH_SLOT`].
	fn of_new_class(touch: Touch) -> Box<Self> {
		let slots = Vec::leak(vec![touch as *mut c_void; TOUCH_SLOT + 1]);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
		NonNull::from(self).cast()
	}

	fn vtable(&self) -> NonNull<*mut c_void> {
		NonNull::new(self.vtable).unwrap()
	}
}

/// Another callback, which only notes that it ran.
fn also_on_touch(
	_server: Server<'_>,
	stage: TouchStage,
	entity: Entity<'_>,
	other: Entity<'_>,
) -> TouchAction {
	if stage == TouchStage::Before {
		note("also", entity.as_ptr().addr(), other.as_ptr().addr());
	}

	TouchAction::Block
}

/// The game's `Touch`, which notes that it ran.
unsafe extern "C" fn game_touch(this: *mut sys::CBaseEntity, other: *mut sys::CBaseEntity) {
	note("game", this.addr(), other.addr());
}

/// Notes a call of `name` for the touched entity and the one touching it.
fn note(name: &'static str, entity: usize, other: usize) {
	CALLS.with_borrow_mut(|calls| calls.push((name, entity, other)));
}

/// The callback, which notes the stage, and blocks the touches of [`BLOCKED`].
fn on_touch(
	_server: Server<'_>,
	stage: TouchStage,
	entity: Entity<'_>,
	other: Entity<'_>,
) -> TouchAction {
	let entity = entity.as_ptr().addr();

	note(
		match stage {
			TouchStage::Before => "before",
			TouchStage::After => "after",
		},
		entity,
		other.as_ptr().addr(),
	);

	if entity == BLOCKED.get() {
		TouchAction::Block
	} else {
		TouchAction::Allow
	}
}

/// Calls `entity`'s hooked `Touch` with `other`, and returns what ran.
fn touch(
	harness: &Harness,
	entity: &mut Mock,
	other: &mut Mock,
) -> Vec<(&'static str, usize, usize)> {
	CALLS.take();
	harness.call::<Touch>(entity.ptr().as_ptr(), TOUCH_SLOT, (other.ptr().as_ptr(),));
	CALLS.take()
}

#[test]
fn touches_run_between_the_hooks_unless_blocked() {
	on_both(|harness| {
		let api = harness.api();
		let mut item = Mock::of_new_class(game_touch);
		let mut zone = Mock::of_new_class(game_touch);
		let mut player = Mock::of_new_class(game_touch);
		let (item_address, zone_address, player_address) = (
			item.ptr().addr().get(),
			zone.ptr().addr().get(),
			player.ptr().addr().get(),
		);

		for mock in [&item, &zone] {
			// SAFETY: The mock classes have `Touch` at the slot, and are leaked.
			let _ =
				unsafe { api.install_touches(mock.vtable(), tf2_binding(no_interfaces), on_touch) }
					.unwrap();
		}

		// A class is hooked once per callback, so that each touch reaches it
		// once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_touches(item.vtable(), tf2_binding(no_interfaces), on_touch) },
			Err(HookError::AlreadyInstalled)
		));

		BLOCKED.set(zone_address);
		assert_eq!(
			touch(harness, &mut item, &mut player),
			[
				("before", item_address, player_address),
				("game", item_address, player_address),
				("after", item_address, player_address),
			]
		);

		// A blocked touch skips the game's, but not the hook after it.
		assert_eq!(
			touch(harness, &mut zone, &mut player),
			[
				("before", zone_address, player_address),
				("after", zone_address, player_address)
			]
		);

		// Another callback hooks the class too, after the first, and no longer
		// runs once removed.
		// SAFETY: As above.
		let also = unsafe {
			api.install_touches(item.vtable(), tf2_binding(no_interfaces), also_on_touch)
		}
		.unwrap();

		assert_eq!(
			touch(harness, &mut item, &mut player),
			[
				("before", item_address, player_address),
				("also", item_address, player_address),
				("after", item_address, player_address),
			]
		);

		also.remove(api);
		assert_eq!(
			touch(harness, &mut item, &mut player),
			[
				("before", item_address, player_address),
				("game", item_address, player_address),
				("after", item_address, player_address),
			]
		);
	});
}
