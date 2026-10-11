//! Tests of `crate::hooks::tf2::death`: post hooks of `Event_Killed` on mock player
//! classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, expect, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::tf2::damage::DamageType;
use std::cell::RefCell;
use std::ffi::c_void;

thread_local! {
	/// What ran during the calls since the last [`kill`], in order, with the
	/// object each ran for.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };
}

/// A player of a C++ class, as far as hooks know it.
#[repr(C)]
struct Player {
	vtable: *mut *mut c_void,
}

impl Player {
	/// A player of a new class, whose vtable holds `killed` at
	/// [`EVENT_KILLED_SLOT`].
	fn of_new_class(killed: EventKilled) -> Box<Self> {
		let slots = Vec::leak(vec![killed as *mut c_void; EVENT_KILLED_SLOT + 1]);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
		NonNull::from(self).cast()
	}
}

#[test]
fn deaths_reach_each_class_hook_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class(game_killed);
		let mut bot = Player::of_new_class(game_killed);
		let (player_address, bot_address) = (player.ptr().addr().get(), bot.ptr().addr().get());

		for object in [player.ptr(), bot.ptr()] {
			// SAFETY: The mock classes have `Event_Killed` at the slot, and are
			// leaked.
			unsafe {
				api.install_death(
					object,
					tf2_binding(no_interfaces),
					on_killed,
					HookTiming::Post,
				)
			}
			.unwrap();
		}

		// A second hook of a class is refused, so that each death is reported
		// once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_death(
					player.ptr(),
					tf2_binding(no_interfaces),
					on_killed,
					HookTiming::Post,
				)
			},
			Err(DeathHookError::Hook(HookError::AlreadyInstalled))
		));

		assert_eq!(
			kill(harness, &mut player),
			[("game", player_address), ("killed", player_address)]
		);
		assert_eq!(
			kill(harness, &mut bot),
			[("game", bot_address), ("killed", bot_address)]
		);
	});
}

#[test]
fn dying_hooks_run_before_the_game_and_beside_killed_hooks() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class(game_killed);
		let mut bot = Player::of_new_class(game_killed);
		let (player_address, bot_address) = (player.ptr().addr().get(), bot.ptr().addr().get());

		// SAFETY: The mock classes have `Event_Killed` at the slot, and are
		// leaked.
		unsafe {
			api.install_death(
				player.ptr(),
				tf2_binding(no_interfaces),
				on_dying,
				HookTiming::Pre,
			)
			.unwrap();
			api.install_death(
				player.ptr(),
				tf2_binding(no_interfaces),
				on_killed,
				HookTiming::Post,
			)
			.unwrap();
			api.install_death(
				bot.ptr(),
				tf2_binding(no_interfaces),
				on_dying,
				HookTiming::Pre,
			)
			.unwrap();
		}

		// Each timing refuses a second hook of a class on its own.
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_death(
					player.ptr(),
					tf2_binding(no_interfaces),
					on_dying,
					HookTiming::Pre,
				)
			},
			Err(DeathHookError::Hook(HookError::AlreadyInstalled))
		));

		assert_eq!(
			kill(harness, &mut player),
			[
				("dying", player_address),
				("game", player_address),
				("killed", player_address)
			]
		);
		assert_eq!(
			kill(harness, &mut bot),
			[("dying", bot_address), ("game", bot_address)]
		);
	});
}

/// The game's `Event_Killed`, which notes that it ran.
unsafe extern "C" fn game_killed(this: *mut sys::CBaseEntity, info: *const sys::CTakeDamageInfo) {
	// SAFETY: The tests pass a constructed local damage record.
	let amount = unsafe { (&raw const (*info).m_flDamage).read() };

	expect(amount == 11.0, "the game received another damage record");
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr())));
}

/// Calls `player`'s hooked `Event_Killed` with 11 bullet damage, and returns
/// what ran.
fn kill(harness: &Harness, player: &mut Player) -> Vec<(&'static str, usize)> {
	let info = DamageInfo::new(11.0, DamageType::BULLET);

	CALLS.take();
	harness.call::<EventKilled>(player.ptr().as_ptr(), EVENT_KILLED_SLOT, (info.as_ptr(),));
	assert_eq!(
		info.amount(),
		11.0,
		"a const source record must never be overwritten"
	);
	CALLS.take()
}

/// The callback before the game, which checks the copy it was given and
/// notes the player.
fn on_dying(_server: Server<'_>, victim: Entity<'_>, info: &DamageInfo) {
	expect(
		info.amount() == 11.0,
		"the callback saw another damage record",
	);
	CALLS.with_borrow_mut(|calls| calls.push(("dying", victim.as_ptr().addr())));
}

/// The callback, which checks the copy it was given and notes the player.
fn on_killed(_server: Server<'_>, victim: Entity<'_>, info: &DamageInfo) {
	expect(
		info.amount() == 11.0,
		"the callback saw another damage record",
	);
	expect(
		info.damage_type() == DamageType::BULLET,
		"the callback saw another damage type",
	);
	CALLS.with_borrow_mut(|calls| calls.push(("killed", victim.as_ptr().addr())));
}
