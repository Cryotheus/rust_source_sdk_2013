//! Tests of `crate::hooks::tf2::scoreboard`: pre hooks of `Think` on a mock player
//! resource class, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::InterfaceFactory;
use std::cell::RefCell;

thread_local! {
	/// What ran during the calls since the last [`think`], in order, with the
	/// object each ran for.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };
}

/// A player resource of a C++ class, as far as hooks know it.
#[repr(C)]
struct Resource {
	vtable: *mut *mut c_void,
}

impl Resource {
	/// The vtable of a new class, which holds [`game_think`] at
	/// [`THINK_SLOT`].
	fn new_class() -> NonNull<*mut c_void> {
		let slots = Vec::leak(vec![game_think as Think as *mut c_void; THINK_SLOT + 1]);

		NonNull::new(slots.as_mut_ptr()).unwrap()
	}

	/// A resource of the class whose vtable is `vtable`, as each level makes.
	fn of_class(vtable: NonNull<*mut c_void>) -> Box<Self> {
		Box::new(Self {
			vtable: vtable.as_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
		NonNull::from(self).cast()
	}
}

/// The game's `Think`, which notes that it ran.
unsafe extern "C" fn game_think(this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr())));
}

/// The callback, which notes that it ran for the resource.
fn on_update(_server: Server<'_>, resource: Entity<'_>) {
	CALLS.with_borrow_mut(|calls| calls.push(("callback", resource.as_ptr().addr())));
}

/// Runs `resource`'s hooked think, and returns what ran.
fn think(harness: &Harness, resource: &mut Resource) -> Vec<(&'static str, usize)> {
	CALLS.take();
	harness.call::<Think>(resource.ptr().as_ptr(), THINK_SLOT, ());
	CALLS.take()
}

#[test]
fn one_hook_covers_the_resource_of_every_level() {
	on_both(|harness| {
		let api = harness.api();
		let class = Resource::new_class();
		let mut first = Resource::of_class(class);

		// SAFETY: The mock class has `Think` at the slot, and is leaked.
		unsafe {
			api.install_scoreboard_updates(first.ptr(), tf2_binding(no_interfaces), on_update)
		}
		.unwrap();
		drop(first);

		// The resources of three levels, the first gone before the next.
		for _level in 0..3 {
			let mut resource = Resource::of_class(class);
			let address = resource.ptr().addr().get();

			assert_eq!(
				think(harness, &mut resource),
				[("callback", address), ("game", address)]
			);
		}
	});
}

#[test]
fn pauses_and_reloads_let_the_game_update() {
	on_both(|harness| {
		let api = harness.api();
		let mut resource = Resource::of_class(Resource::new_class());
		let address = resource.ptr().addr().get();

		// SAFETY: As above.
		let hook = unsafe {
			api.install_scoreboard_updates(resource.ptr(), tf2_binding(no_interfaces), on_update)
		}
		.unwrap();

		harness.set_status(true, true, harness.generation);
		assert_eq!(think(harness, &mut resource), [("game", address)]);

		// A later load, which has not installed its own hook yet. Its
		// generation is one no later harness takes, as it installs hooks.
		harness.set_status(true, false, harness.generation | 1 << 63);
		assert_eq!(think(harness, &mut resource), [("game", address)]);
		assert!(!api.has_hook(hook));

		// SAFETY: As above.
		unsafe {
			api.install_scoreboard_updates(resource.ptr(), tf2_binding(no_interfaces), on_update)
		}
		.unwrap();
		assert_eq!(
			think(harness, &mut resource),
			[("callback", address), ("game", address)]
		);
	});
}

#[test]
fn removed_or_superseded_hooks_run_no_callback() {
	on_both(|harness| {
		let api = harness.api();
		let mut resource = Resource::of_class(Resource::new_class());
		let address = resource.ptr().addr().get();

		// SAFETY: As above.
		let hook = unsafe {
			api.install_scoreboard_updates(resource.ptr(), tf2_binding(no_interfaces), on_update)
		}
		.unwrap();

		assert!(api.remove_hook(hook));
		assert_eq!(think(harness, &mut resource), [("game", address)]);

		// A think an earlier hook skipped reaches neither the callback nor the
		// game.
		fn skip(_call: &HookCall<'_, Think>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: As above.
		unsafe {
			api.add_hook(
				THINK,
				HookTarget::class_of(resource.ptr()),
				HookTiming::Pre,
				&skip,
			)
			.unwrap();
			api.install_scoreboard_updates(resource.ptr(), tf2_binding(no_interfaces), on_update)
				.unwrap();
		}

		assert_eq!(think(harness, &mut resource), []);
	});
}

#[test]
fn the_resource_is_looked_for_unless_already_hooked() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();

		// SAFETY: As for `tf2_binding`, but for another game, whose servers reach
		// no interface before the game is checked.
		let other = unsafe {
			ServerBinding::new(
				InterfaceFactory::new(no_interfaces),
				InterfaceFactory::new(no_interfaces),
				Game::SourceSdk2013,
			)
		};
		// SAFETY: As above.
		let other_server = unsafe { other.server(&scope) };

		assert!(matches!(
			api.hook_scoreboard_updates(other_server, other, on_update),
			Err(ScoreboardHookError::NotTf2)
		));

		let binding = tf2_binding(no_interfaces);
		// SAFETY: As for `tf2_binding`. The game server exports no interface, so
		// the search for the resource finds no server tools to search with.
		let server = unsafe { binding.server(&scope) };

		assert!(matches!(
			api.hook_scoreboard_updates(server, binding, on_update),
			Err(ScoreboardHookError::Interface(_))
		));

		let mut resource = Resource::of_class(Resource::new_class());

		// SAFETY: The mock class has `Think` at the slot, and is leaked.
		unsafe { api.install_scoreboard_updates(resource.ptr(), binding, on_update) }.unwrap();

		// An installed hook is reported without looking for the resource.
		assert!(matches!(
			api.hook_scoreboard_updates(server, binding, on_update),
			Err(ScoreboardHookError::Hook(HookError::AlreadyInstalled))
		));
	});
}

#[test]
fn updates_run_after_the_callback() {
	on_both(|harness| {
		let api = harness.api();
		let class = Resource::new_class();
		let mut resource = Resource::of_class(class);
		let address = resource.ptr().addr().get();

		// SAFETY: The mock class has `Think` at the slot, and is leaked.
		unsafe {
			api.install_scoreboard_updates(resource.ptr(), tf2_binding(no_interfaces), on_update)
		}
		.unwrap();

		// A second hook of the class is refused, so that each update is
		// prepared once, and so is a hook of another class.
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_scoreboard_updates(
					resource.ptr(),
					tf2_binding(no_interfaces),
					on_update,
				)
			},
			Err(HookError::AlreadyInstalled)
		));
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_scoreboard_updates(
					Resource::of_class(Resource::new_class()).ptr(),
					tf2_binding(no_interfaces),
					on_update,
				)
			},
			Err(HookError::TooManyFunctions)
		));

		assert_eq!(
			think(harness, &mut resource),
			[("callback", address), ("game", address)]
		);
	});
}
