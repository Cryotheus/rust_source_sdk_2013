//! Tests of `crate::hooks::tf2::spawn`: hooks after `Spawn`, on mock entity
//! classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use std::cell::RefCell;

thread_local! {
	/// What ran during the calls since the last [`spawn`], in order, with the
	/// entity each ran for.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };
}

/// An entity of a C++ class, as far as hooks know it.
#[repr(C)]
struct Mock {
	vtable: *mut *mut c_void,
}

impl Mock {
	/// An entity of a new class, whose vtable holds [`game_spawn`] at
	/// [`SPAWN_SLOT`].
	fn of_new_class() -> Box<Self> {
		let slots = Vec::leak(vec![game_spawn as Spawn as *mut c_void; SPAWN_SLOT + 1]);

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
fn also_on_spawned(_server: Server<'_>, entity: Entity<'_>) {
	note("also", entity.as_ptr().addr());
}

/// The game's `Spawn`, which notes that it ran.
unsafe extern "C" fn game_spawn(this: *mut sys::CBaseEntity) {
	note("game", this.addr());
}

/// Notes a call of `name` for the entity.
fn note(name: &'static str, entity: usize) {
	CALLS.with_borrow_mut(|calls| calls.push((name, entity)));
}

/// The callback, which notes that it ran.
fn on_spawned(_server: Server<'_>, entity: Entity<'_>) {
	note("spawned", entity.as_ptr().addr());
}

/// Calls `entity`'s hooked `Spawn`, and returns what ran.
fn spawn(harness: &Harness, entity: &mut Mock) -> Vec<(&'static str, usize)> {
	CALLS.take();
	harness.call::<Spawn>(entity.ptr().as_ptr(), SPAWN_SLOT, ());
	CALLS.take()
}

#[test]
fn spawns_reach_each_class_hook_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let mut flames = Mock::of_new_class();
		let mut fireball = Mock::of_new_class();
		let mut rocket = Mock::of_new_class();
		let (flames_address, fireball_address, rocket_address) = (
			flames.ptr().addr().get(),
			fireball.ptr().addr().get(),
			rocket.ptr().addr().get(),
		);

		for mock in [&flames, &fireball] {
			// SAFETY: The mock classes have `Spawn` at the slot, and are leaked.
			unsafe { api.install_spawns(mock.vtable(), tf2_binding(no_interfaces), on_spawned) }
				.unwrap();
		}

		// A class is hooked once per callback, so that each spawn reaches it
		// once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_spawns(flames.vtable(), tf2_binding(no_interfaces), on_spawned) },
			Err(HookError::AlreadyInstalled)
		));

		assert_eq!(
			spawn(harness, &mut flames),
			[("game", flames_address), ("spawned", flames_address)]
		);
		assert_eq!(
			spawn(harness, &mut fireball),
			[("game", fireball_address), ("spawned", fireball_address)]
		);

		// Classes not hooked spawn as they do.
		assert_eq!(spawn(harness, &mut rocket), [("game", rocket_address)]);

		// Another callback hooks the class too, after the first, and no longer
		// runs once removed.
		// SAFETY: As above.
		let also = unsafe {
			api.install_spawns(flames.vtable(), tf2_binding(no_interfaces), also_on_spawned)
		}
		.unwrap();

		assert_eq!(
			spawn(harness, &mut flames),
			[
				("game", flames_address),
				("spawned", flames_address),
				("also", flames_address),
			]
		);

		assert!(api.remove_hook(also));
		assert_eq!(
			spawn(harness, &mut flames),
			[("game", flames_address), ("spawned", flames_address)]
		);
	});
}

#[test]
fn spawn_hooks_run_after_superseded_spawns() {
	on_both(|harness| {
		let api = harness.api();
		let mut flames = Mock::of_new_class();
		let address = flames.ptr().addr().get();

		// Another hook, which skips every spawn.
		fn skip(_call: &HookCall<'_, Spawn>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: The mock class has `Spawn` at the slot, and is leaked.
		unsafe {
			api.add_hook(
				SPAWN,
				HookTarget::vtable(flames.vtable()),
				HookTiming::Pre,
				&skip,
			)
			.unwrap();
			api.install_spawns(flames.vtable(), tf2_binding(no_interfaces), on_spawned)
				.unwrap();
		}

		assert_eq!(spawn(harness, &mut flames), [("spawned", address)]);
	});
}
