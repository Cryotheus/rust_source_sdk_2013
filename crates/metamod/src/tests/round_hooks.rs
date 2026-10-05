//! Tests of `crate::round_hooks`: pre hooks of `CleanUpMap` on a mock
//! game rules class, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::InterfaceFactory;
use std::cell::RefCell;

thread_local! {
	/// What ran during the calls since the last [`clean_up`], in order, with
	/// the object each ran for.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };

	/// Whether the callback skips the cleanup.
	static SKIP: Cell<bool> = const { Cell::new(false) };
}

/// A game rules object of a C++ class, as far as hooks know it.
#[repr(C)]
struct GameRules {
	vtable: *mut *mut c_void,
}

impl GameRules {
	/// The vtable of a new class, which holds `clean_up` at
	/// [`CLEAN_UP_MAP_SLOT`].
	fn new_class(clean_up: CleanUpMap) -> NonNull<*mut c_void> {
		let slots = Vec::leak(vec![clean_up as *mut c_void; CLEAN_UP_MAP_SLOT + 1]);

		NonNull::new(slots.as_mut_ptr()).unwrap()
	}

	/// An object of the class whose vtable is `vtable`, as each level makes.
	fn of_class(vtable: NonNull<*mut c_void>) -> Box<Self> {
		Box::new(Self {
			vtable: vtable.as_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<c_void> {
		NonNull::from(self).cast()
	}
}

/// The game's `CleanUpMap`, which notes that it ran.
unsafe extern "C" fn game_clean_up(this: *mut c_void) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr())));
}

/// The callback, which notes that it ran, and skips the cleanup if [`SKIP`].
fn on_clean_up(_server: Server<'_>) -> MapCleanupAction {
	CALLS.with_borrow_mut(|calls| calls.push(("callback", 0)));

	match SKIP.get() {
		true => MapCleanupAction::Skip,
		false => MapCleanupAction::Allow,
	}
}

/// Calls `rules`'s hooked `CleanUpMap`, and returns what ran.
fn clean_up(harness: &Harness, rules: &mut GameRules) -> Vec<(&'static str, usize)> {
	CALLS.take();
	harness.call::<CleanUpMap>(rules.ptr().as_ptr(), CLEAN_UP_MAP_SLOT, ());
	CALLS.take()
}

#[test]
fn cleanups_run_unless_the_hook_skips_them() {
	on_both(|harness| {
		let api = harness.api();
		let class = GameRules::new_class(game_clean_up);
		let mut rules = GameRules::of_class(class);
		let address = rules.ptr().addr().get();

		// SAFETY: The mock class has `CleanUpMap` at the slot, and is leaked.
		unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) }.unwrap();

		// A second hook of the class is refused, so that each cleanup is decided
		// once, and so is a hook of another class.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) },
			Err(HookError::AlreadyInstalled)
		));
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_map_cleanup(
					GameRules::new_class(game_clean_up),
					tf2_binding(no_interfaces),
					on_clean_up,
				)
			},
			Err(HookError::TooManyFunctions)
		));

		SKIP.set(false);
		assert_eq!(
			clean_up(harness, &mut rules),
			[("callback", 0), ("game", address)]
		);

		SKIP.set(true);
		assert_eq!(clean_up(harness, &mut rules), [("callback", 0)]);
	});
}

#[test]
fn one_hook_covers_the_game_rules_of_every_level() {
	on_both(|harness| {
		let api = harness.api();
		let class = GameRules::new_class(game_clean_up);

		// SAFETY: As above.
		unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) }.unwrap();
		SKIP.set(true);

		// The game rules of three levels, the first gone before the next.
		for _level in 0..3 {
			let mut rules = GameRules::of_class(class);

			assert_eq!(clean_up(harness, &mut rules), [("callback", 0)]);
			drop(rules);
		}
	});
}

#[test]
fn pauses_and_reloads_let_the_game_clean_up() {
	on_both(|harness| {
		let api = harness.api();
		let class = GameRules::new_class(game_clean_up);
		let mut rules = GameRules::of_class(class);
		let address = rules.ptr().addr().get();

		// SAFETY: As above.
		let hook =
			unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) }
				.unwrap();
		SKIP.set(true);

		harness.set_status(true, true, harness.generation);
		assert_eq!(clean_up(harness, &mut rules), [("game", address)]);

		// A later load, which has not installed its own hook yet. Its
		// generation is one no later harness takes, as it installs hooks.
		harness.set_status(true, false, harness.generation | 1 << 63);
		assert_eq!(clean_up(harness, &mut rules), [("game", address)]);
		assert!(!api.has_hook(hook));

		// SAFETY: As above.
		unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) }.unwrap();
		assert_eq!(clean_up(harness, &mut rules), [("callback", 0)]);
	});
}

#[test]
fn removed_or_superseded_hooks_run_no_callback() {
	on_both(|harness| {
		let api = harness.api();
		let class = GameRules::new_class(game_clean_up);
		let mut rules = GameRules::of_class(class);
		let address = rules.ptr().addr().get();

		// SAFETY: As above.
		let hook =
			unsafe { api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up) }
				.unwrap();
		SKIP.set(true);

		assert!(api.remove_hook(hook));
		assert_eq!(clean_up(harness, &mut rules), [("game", address)]);

		// A cleanup an earlier hook skipped reaches neither the callback nor the
		// game.
		fn skip(_call: &HookCall<'_, CleanUpMap>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: As above.
		unsafe {
			api.add_hook(
				CLEAN_UP_MAP,
				HookTarget::vtable(class),
				HookTiming::Pre,
				&skip,
			)
			.unwrap();
			api.install_map_cleanup(class, tf2_binding(no_interfaces), on_clean_up)
				.unwrap();
		}

		assert_eq!(clean_up(harness, &mut rules), []);
	});
}

#[test]
fn the_game_rules_class_is_searched_for_unless_already_hooked() {
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
			api.hook_map_cleanup(other_server, other, on_clean_up),
			Err(MapCleanupHookError::Target(GameRulesVtableError::WrongGame))
		));

		let binding = tf2_binding(no_interfaces);
		// SAFETY: The server's game module is this test's executable, which
		// `no_interfaces` is in, and which has no game rules class.
		let server = unsafe { binding.server(&scope) };

		assert!(matches!(
			api.hook_map_cleanup(server, binding, on_clean_up),
			Err(MapCleanupHookError::Target(GameRulesVtableError::NotFound))
		));

		// SAFETY: As above.
		unsafe {
			api.install_map_cleanup(GameRules::new_class(game_clean_up), binding, on_clean_up)
		}
		.unwrap();

		// An installed hook is reported without searching the module again.
		assert!(matches!(
			api.hook_map_cleanup(server, binding, on_clean_up),
			Err(MapCleanupHookError::Hook(HookError::AlreadyInstalled))
		));
	});
}
