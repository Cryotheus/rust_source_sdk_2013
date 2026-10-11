//! Tests of `crate::hooks::tf2::player`: post hooks of `Spawn`, `ResetScores` and
//! `TakeHealth` on mock player classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use std::cell::RefCell;

thread_local! {
	/// What ran during the calls since the last [`run`] or [`heal`], in order,
	/// with the object each ran for.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };

	/// The health each healed callback was given since the last [`heal`].
	static GAINED: RefCell<Vec<c_int>> = const { RefCell::new(Vec::new()) };
}

/// A player of a C++ class, as far as hooks know it.
#[repr(C)]
struct Player {
	vtable: *mut *mut c_void,
}

impl Player {
	/// A player of a new class, whose vtable holds [`game_method`] at
	/// [`SPAWN_SLOT`] and [`RESET_SCORES_SLOT`], and [`game_take_health`] at
	/// [`TAKE_HEALTH_SLOT`].
	fn of_new_class() -> Box<Self> {
		let slots = Vec::leak(vec![
			game_method as PlayerMethod as *mut c_void;
			SPAWN_SLOT.max(RESET_SCORES_SLOT).max(TAKE_HEALTH_SLOT)
				+ 1
		]);

		slots[TAKE_HEALTH_SLOT] = game_take_health as TakeHealth as *mut c_void;

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
		NonNull::from(self).cast()
	}
}

/// The game's `Spawn` or `ResetScores`, which notes that it ran.
unsafe extern "C" fn game_method(this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr())));
}

/// The game's `TakeHealth`, which notes that it ran, and returns the healing
/// as the health gained.
unsafe extern "C" fn game_take_health(
	this: *mut sys::CBaseEntity,
	amount: f32,
	_damage_type: c_int,
) -> c_int {
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr())));
	amount as c_int
}

/// Heals `player` by `amount` through its hooked `TakeHealth`, and returns
/// what ran, what the call returned, and the health each callback was given.
fn heal(
	harness: &Harness,
	player: &mut Player,
	amount: f32,
) -> (Vec<(&'static str, usize)>, c_int, Vec<c_int>) {
	CALLS.take();
	GAINED.take();

	let returned = harness.call::<TakeHealth>(player.ptr().as_ptr(), TAKE_HEALTH_SLOT, (amount, 0));

	(CALLS.take(), returned, GAINED.take())
}

fn on_healed(_server: Server<'_>, player: Entity<'_>, gained: c_int) {
	CALLS.with_borrow_mut(|calls| calls.push(("healed", player.as_ptr().addr())));
	GAINED.with_borrow_mut(|all| all.push(gained));
}

fn on_scores_reset(_server: Server<'_>, player: Entity<'_>) {
	CALLS.with_borrow_mut(|calls| calls.push(("scores reset", player.as_ptr().addr())));
}

fn on_spawned(_server: Server<'_>, player: Entity<'_>) {
	CALLS.with_borrow_mut(|calls| calls.push(("spawned", player.as_ptr().addr())));
}

/// Calls `player`'s hooked `function`, and returns what ran.
fn run(
	harness: &Harness,
	player: &mut Player,
	function: VirtualFunction<PlayerMethod>,
) -> Vec<(&'static str, usize)> {
	CALLS.take();
	harness.call::<PlayerMethod>(player.ptr().as_ptr(), function.index(), ());
	CALLS.take()
}

#[test]
fn spawns_and_score_resets_reach_each_class_hook_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class();
		let mut bot = Player::of_new_class();
		let (player_address, bot_address) = (player.ptr().addr().get(), bot.ptr().addr().get());

		for object in [player.ptr(), bot.ptr()] {
			// SAFETY: The mock classes have `void ()` methods at the slots, and
			// are leaked.
			unsafe {
				api.install_player_method(
					object,
					tf2_binding(no_interfaces),
					on_spawned,
					SPAWN,
					&SPAWNED_ROUTES,
				)
				.unwrap();
				api.install_player_method(
					object,
					tf2_binding(no_interfaces),
					on_scores_reset,
					RESET_SCORES,
					&SCORES_RESET_ROUTES,
				)
				.unwrap();
			}
		}

		// A second hook of a class is refused, so that each call is reported
		// once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_player_method(
					player.ptr(),
					tf2_binding(no_interfaces),
					on_spawned,
					SPAWN,
					&SPAWNED_ROUTES,
				)
			},
			Err(PlayerHookError::Hook(HookError::AlreadyInstalled))
		));

		assert_eq!(
			run(harness, &mut player, SPAWN),
			[("game", player_address), ("spawned", player_address)]
		);
		assert_eq!(
			run(harness, &mut bot, SPAWN),
			[("game", bot_address), ("spawned", bot_address)]
		);
		assert_eq!(
			run(harness, &mut player, RESET_SCORES),
			[("game", player_address), ("scores reset", player_address)]
		);
		assert_eq!(
			run(harness, &mut bot, RESET_SCORES),
			[("game", bot_address), ("scores reset", bot_address)]
		);
	});
}

#[test]
fn spawn_hooks_run_after_superseded_spawns() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class();
		let address = player.ptr().addr().get();

		// Another hook, which skips every spawn.
		fn skip(_call: &HookCall<'_, PlayerMethod>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: The mock class has `Spawn` at the slot, and is leaked.
		unsafe {
			api.add_hook(
				SPAWN,
				HookTarget::class_of(player.ptr()),
				HookTiming::Pre,
				&skip,
			)
			.unwrap();
			api.install_player_method(
				player.ptr(),
				tf2_binding(no_interfaces),
				on_spawned,
				SPAWN,
				&SPAWNED_ROUTES,
			)
			.unwrap();
		}

		assert_eq!(run(harness, &mut player, SPAWN), [("spawned", address)]);
	});
}

#[test]
fn heals_reach_each_class_hook_after_the_game_with_the_health_gained() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class();
		let mut bot = Player::of_new_class();
		let (player_address, bot_address) = (player.ptr().addr().get(), bot.ptr().addr().get());

		for object in [player.ptr(), bot.ptr()] {
			// SAFETY: The mock classes have `int (float, int)` methods at the
			// slot, and are leaked.
			unsafe { api.install_player_healed(object, tf2_binding(no_interfaces), on_healed) }
				.unwrap();
		}

		// A second hook of a class is refused, so that each heal is reported
		// once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_player_healed(player.ptr(), tf2_binding(no_interfaces), on_healed)
			},
			Err(PlayerHookError::Hook(HookError::AlreadyInstalled))
		));

		assert_eq!(
			heal(harness, &mut player, 7.5),
			(
				vec![("game", player_address), ("healed", player_address)],
				7,
				vec![7]
			)
		);
		assert_eq!(
			heal(harness, &mut bot, 0.0),
			(
				vec![("game", bot_address), ("healed", bot_address)],
				0,
				vec![0]
			)
		);
	});
}

#[test]
fn heal_hooks_are_given_what_an_earlier_hook_returned() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class();
		let address = player.ptr().addr().get();

		// Another hook, which skips every heal and returns 3 instead.
		fn skip(_call: &HookCall<'_, TakeHealth>) -> HookAction<c_int> {
			HookAction::Supersede(3)
		}

		// SAFETY: The mock class has `TakeHealth` at the slot, and is leaked.
		unsafe {
			api.add_hook(
				TAKE_HEALTH,
				HookTarget::class_of(player.ptr()),
				HookTiming::Pre,
				&skip,
			)
			.unwrap();
			api.install_player_healed(player.ptr(), tf2_binding(no_interfaces), on_healed)
				.unwrap();
		}

		assert_eq!(
			heal(harness, &mut player, 10.0),
			(vec![("healed", address)], 3, vec![3])
		);
	});
}
