//! Tests of `crate::observer_hooks`: post hooks of `SetObserverMode` on mock
//! player classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};

use source_sdk_2013::raw::tf2::observer::{
	GET_OBSERVER_MODE_SLOT, GetObserverModeFn, NUM_OBSERVER_MODES, OBS_MODE_CHASE,
};

use std::cell::RefCell;
use std::ffi::c_int;
use std::ptr::null_mut;

thread_local! {
	/// The players the callback ran for since the last [`set_mode`].
	static CALLBACKS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// A player of a C++ class, as far as hooks know it, and the state the mock
/// game's observer methods read and write.
#[repr(C)]
struct Player {
	vtable: *mut *mut c_void,
	mode: c_int,
	/// Whether the player is on a team, which the game does not let roam.
	on_team: bool,
	/// Whether the match summary is showing, during which the game refuses
	/// every change.
	summary: bool,
}

impl Player {
	/// A player of a new class, whose vtable holds the mock game's
	/// `SetObserverMode` and `GetObserverMode`.
	fn of_new_class(on_team: bool) -> Box<Self> {
		let slots = Vec::leak(vec![
			null_mut::<c_void>();
			SET_OBSERVER_MODE_SLOT.max(GET_OBSERVER_MODE_SLOT) + 1
		]);

		slots[SET_OBSERVER_MODE_SLOT] = game_set_mode as SetObserverMode as *mut c_void;
		slots[GET_OBSERVER_MODE_SLOT] = game_get_mode as GetObserverModeFn as *mut c_void;

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
			mode: OBS_MODE_CHASE,
			on_team,
			summary: false,
		})
	}

	fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
		NonNull::from(self).cast()
	}
}

/// The game's `GetObserverMode`.
unsafe extern "C" fn game_get_mode(this: *mut sys::CBaseEntity) -> c_int {
	// SAFETY: Only mock players have this method in their vtables.
	unsafe { (*this.cast::<Player>()).mode }
}

/// The game's `SetObserverMode` outside PASS Time, as far as the hook relies
/// on it.
unsafe extern "C" fn game_set_mode(this: *mut sys::CBaseEntity, mut mode: c_int) -> bool {
	// SAFETY: As for `game_get_mode`.
	let player = unsafe { &mut *this.cast::<Player>() };

	if !(0..NUM_OBSERVER_MODES).contains(&mode) || player.summary {
		return false;
	}

	if mode == OBS_MODE_POI {
		mode = OBS_MODE_ROAMING;
	}

	if player.on_team && mode == OBS_MODE_ROAMING {
		mode = OBS_MODE_IN_EYE;
	}

	player.mode = mode;
	true
}

/// The callback, which notes the player and lets them roam, as
/// `PlayerObserver::roam` would.
fn on_refused(_server: Server<'_>, player: Entity<'_>) {
	let player = player.as_ptr().cast::<Player>();

	CALLBACKS.with_borrow_mut(|callbacks| callbacks.push(player.addr()));

	// SAFETY: The hook passes the live mock player whose method just ran.
	unsafe { (*player).mode = OBS_MODE_ROAMING };
}

#[test]
fn only_roaming_the_game_turned_into_first_person_reaches_the_callback() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class(true);
		let mut spectator = Player::of_new_class(false);
		let address = player.ptr().addr().get();

		for object in [player.ptr(), spectator.ptr()] {
			// SAFETY: The mock classes have both methods at their slots, and are
			// leaked.
			unsafe { api.install_roaming(object, tf2_binding(no_interfaces), on_refused) }.unwrap();
		}

		// A second hook of a class is refused, so that each change is seen
		// once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_roaming(player.ptr(), tf2_binding(no_interfaces), on_refused) },
			Err(ObserverHookError::Hook(HookError::AlreadyInstalled))
		));

		// Roaming, and the point of interest outside PASS Time, turn into first
		// person for a player on a team, which the callback undoes.
		for mode in [OBS_MODE_ROAMING, OBS_MODE_POI] {
			assert_eq!(
				set_mode(harness, &mut player, mode),
				(true, OBS_MODE_ROAMING, vec![address])
			);
		}

		// Other modes, and spectators, are left to the game.
		assert_eq!(
			set_mode(harness, &mut player, OBS_MODE_IN_EYE),
			(true, OBS_MODE_IN_EYE, vec![])
		);
		assert_eq!(
			set_mode(harness, &mut player, OBS_MODE_CHASE),
			(true, OBS_MODE_CHASE, vec![])
		);
		assert_eq!(
			set_mode(harness, &mut spectator, OBS_MODE_ROAMING),
			(true, OBS_MODE_ROAMING, vec![])
		);

		// So is a change the game refused, even of a player following in first
		// person.
		player.mode = OBS_MODE_IN_EYE;
		player.summary = true;
		assert_eq!(
			set_mode(harness, &mut player, OBS_MODE_ROAMING),
			(false, OBS_MODE_IN_EYE, vec![])
		);
	});
}

/// Calls `player`'s hooked `SetObserverMode` with `mode`, and returns what it
/// returned, the player's mode after it, and whom the callback ran for.
fn set_mode(harness: &Harness, player: &mut Player, mode: c_int) -> (bool, c_int, Vec<usize>) {
	CALLBACKS.take();

	let accepted =
		harness.call::<SetObserverMode>(player.ptr().as_ptr(), SET_OBSERVER_MODE_SLOT, (mode,));

	(accepted, player.mode, CALLBACKS.take())
}
