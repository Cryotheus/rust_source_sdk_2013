//! Tests of `crate::hooks::tf2::damage_effect`: pre hooks of `DamageEffect` on mock
//! player classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::raw::tf2::damage::{DMG_DROWN, DMG_PREVENT_PHYSICS_FORCE};
use std::cell::RefCell;
use std::ffi::{c_int, c_void};

thread_local! {
	/// What ran during the calls since the last [`hit`], in order, with the
	/// object each ran for, and the damage and its types each saw.
	static CALLS: RefCell<Vec<(&'static str, usize, f32, c_int)>> = const { RefCell::new(Vec::new()) };

	/// The address of the player whose hits' effects the callback skips.
	static SKIPPED: Cell<usize> = const { Cell::new(0) };
}

/// A player of a C++ class, as far as hooks know it.
#[repr(C)]
struct Player {
	vtable: *mut *mut c_void,
}

impl Player {
	/// A player of a new class, whose vtable holds `effect` at
	/// [`DAMAGE_EFFECT_SLOT`].
	fn of_new_class(effect: DamageEffect) -> Box<Self> {
		let slots = Vec::leak(vec![effect as *mut c_void; DAMAGE_EFFECT_SLOT + 1]);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
		NonNull::from(self).cast()
	}
}

/// The game's `DamageEffect`, which notes that it ran.
unsafe extern "C" fn game_effect(this: *mut sys::CBaseEntity, damage: f32, damage_type: c_int) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr(), damage, damage_type)));
}

/// The callback, which notes the player and the hit, and skips the effects of
/// [`SKIPPED`]'s.
fn on_effect(
	_server: Server<'_>,
	player: Entity<'_>,
	damage: f32,
	damage_type: DamageType,
) -> DamageEffectAction {
	let address = player.as_ptr().addr();

	CALLS.with_borrow_mut(|calls| {
		calls.push(("callback", address, damage, damage_type.bits() as c_int))
	});

	if address == SKIPPED.get() {
		DamageEffectAction::Skip
	} else {
		DamageEffectAction::Show
	}
}

/// Calls `player`'s hooked `DamageEffect` for a hit of `damage` drowning
/// damage, and returns what ran.
fn hit(
	harness: &Harness,
	player: &mut Player,
	damage: f32,
) -> Vec<(&'static str, usize, f32, c_int)> {
	CALLS.take();
	harness.call::<DamageEffect>(
		player.ptr().as_ptr(),
		DAMAGE_EFFECT_SLOT,
		(damage, DMG_DROWN | DMG_PREVENT_PHYSICS_FORCE),
	);
	CALLS.take()
}

#[test]
fn effects_show_unless_a_hook_skips_them() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class(game_effect);
		let mut bot = Player::of_new_class(game_effect);
		let mut blocked = Player::of_new_class(game_effect);
		let (player_address, bot_address) = (player.ptr().addr().get(), bot.ptr().addr().get());
		let drowning = DMG_DROWN | DMG_PREVENT_PHYSICS_FORCE;

		for object in [player.ptr(), bot.ptr()] {
			// SAFETY: The mock classes have `DamageEffect` at the slot, and are
			// leaked.
			unsafe { api.install_damage_effect(object, tf2_binding(no_interfaces), on_effect) }
				.unwrap();
		}

		// A second hook of a class is refused, so that each hit's effects are
		// decided once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_damage_effect(player.ptr(), tf2_binding(no_interfaces), on_effect)
			},
			Err(DamageEffectHookError::Hook(HookError::AlreadyInstalled))
		));

		// The callback sees the hit as the game would, and the game shows its
		// effects unless the callback skips them.
		SKIPPED.set(bot_address);
		assert_eq!(
			hit(harness, &mut player, 7.5),
			[
				("callback", player_address, 7.5, drowning),
				("game", player_address, 7.5, drowning)
			]
		);
		assert_eq!(
			hit(harness, &mut bot, 7.5),
			[("callback", bot_address, 7.5, drowning)]
		);

		// Effects an earlier hook skipped reach neither the callback nor the
		// game.
		fn skip(_call: &HookCall<'_, DamageEffect>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: As above.
		unsafe {
			api.add_hook(
				DAMAGE_EFFECT,
				HookTarget::class_of(blocked.ptr()),
				HookTiming::Pre,
				&skip,
			)
			.unwrap();
			api.install_damage_effect(blocked.ptr(), tf2_binding(no_interfaces), on_effect)
				.unwrap();
		}

		assert_eq!(hit(harness, &mut blocked, 7.5), []);
	});
}
