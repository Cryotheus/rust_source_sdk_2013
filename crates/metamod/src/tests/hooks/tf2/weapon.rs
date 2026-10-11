//! Tests of `crate::hooks::tf2::weapon`: hooks of weapons' and players' weapon
//! methods, on mock classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::tf2::class_targets::{ClassKind, ClassTarget};
use std::cell::{Cell, RefCell};
use std::ffi::{c_int, c_void};
use std::ptr;

thread_local! {
	/// What [`on_switch`] and [`on_reload`] decide.
	static ACTION: Cell<WeaponAction> = const { Cell::new(WeaponAction::Continue) };

	/// What [`on_smack`] decides.
	static SMACK: Cell<SmackAction> = const { Cell::new(SmackAction::Continue) };

	/// What ran during the calls since the last check, in order, with the
	/// address of the weapon it was given, if any.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };
}

/// An entity of a C++ class, as far as hooks know it.
#[repr(C)]
struct Mock {
	vtable: *mut *mut c_void,
}

impl Mock {
	/// An entity of a new class, whose vtable holds `methods` at their slots,
	/// and [`unexpected`] in the others.
	fn of_new_class(methods: &[(usize, *mut c_void)]) -> Box<Self> {
		let last = methods.iter().map(|&(slot, _)| slot).max().unwrap();
		let slots = Vec::leak(vec![unexpected as EntityFn as *mut c_void; last + 1]);

		for &(slot, method) in methods {
			slots[slot] = method;
		}

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn address(&mut self) -> usize {
		self.ptr().addr()
	}

	fn ptr(&mut self) -> *mut sys::CBaseEntity {
		ptr::from_mut(self).cast()
	}

	/// The mock's class, as a class of the kind `K`.
	fn target<K: ClassKind>(&self) -> ClassTarget<'static, K> {
		// SAFETY: The tests only call the hooked slots, which hold a method of
		// their signature.
		unsafe { ClassTarget::from_raw(NonNull::new(self.vtable).unwrap()) }
	}
}

#[test]
fn equips_are_seen_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = player();
		let mut weapon = weapon();
		let address = weapon.address();
		let hooks = api.hook_weapon_equips(tf2_binding(no_interfaces), on_equip);

		assert_eq!(hooks.cover(api, player.target::<TfPlayer>()), Ok(true));

		CALLS.take();
		harness.call::<WeaponFn>(player.ptr(), WEAPON_EQUIP_SLOT, (weapon.ptr().cast(),));
		assert_eq!(CALLS.take(), [("game", address), ("equipped", address)]);

		// Without a weapon, the callback does not run.
		harness.call::<WeaponFn>(player.ptr(), WEAPON_EQUIP_SLOT, (ptr::null_mut(),));
		assert_eq!(CALLS.take(), [("game", 0)]);
	});
}

/// The game's `Weapon_CanSwitchTo`, which notes that it ran, and allows the
/// weapon.
unsafe extern "C" fn game_can_switch_to(
	_: *mut sys::CBaseEntity,
	weapon: *mut sys::CBaseCombatWeapon,
) -> bool {
	note("game", weapon.addr());
	true
}

/// The game's `Weapon_Equip`, which notes that it ran.
unsafe extern "C" fn game_equip(_: *mut sys::CBaseEntity, weapon: *mut sys::CBaseCombatWeapon) {
	note("game", weapon.addr());
}

/// The game's `ItemPostFrame`, which notes that it ran.
unsafe extern "C" fn game_frame(_: *mut sys::CBaseEntity) {
	note("game", 0);
}

/// The game's `Reload`, which notes that it ran, and reloads.
unsafe extern "C" fn game_reload(_: *mut sys::CBaseEntity) -> bool {
	note("game", 0);
	true
}

/// The game's `GetSmackTime`, which notes that it ran with the weapon's mode,
/// and times the smack at 10.2.
unsafe extern "C" fn game_smack_time(_: *mut sys::CBaseEntity, mode: c_int) -> f32 {
	note("game", usize::try_from(mode).unwrap());
	10.2
}

/// The game's `Weapon_Switch`, which notes that it ran, and switches.
unsafe extern "C" fn game_switch(
	_: *mut sys::CBaseEntity,
	weapon: *mut sys::CBaseCombatWeapon,
	_: c_int,
) -> bool {
	note("game", weapon.addr());
	true
}

#[test]
fn melee_swings_can_miss() {
	on_both(|harness| {
		let api = harness.api();
		let mut melee = Mock::of_new_class(&[(
			GET_SMACK_TIME_SLOT,
			game_smack_time as SmackTimeFn as *mut c_void,
		)]);
		let hooks = api.hook_melee_smacks(tf2_binding(no_interfaces), on_smack);

		assert_eq!(hooks.cover(api, melee.target::<TfMeleeWeapon>()), Ok(true));

		let mut swing = || {
			CALLS.take();
			let time = harness.call::<SmackTimeFn>(melee.ptr(), GET_SMACK_TIME_SLOT, (1,));
			(time, CALLS.take())
		};

		SMACK.set(SmackAction::Continue);
		assert_eq!(swing(), (10.2, vec![("smack", 0), ("game", 1)]));

		// The weapon never smacks, as the time is not positive.
		const { assert!(NO_SMACK <= 0.0) };
		SMACK.set(SmackAction::Miss);
		assert_eq!(swing(), (NO_SMACK, vec![("smack", 0)]));
	});
}

/// Notes that `name` ran, given the weapon at `weapon`.
fn note(name: &'static str, weapon: usize) {
	CALLS.with_borrow_mut(|calls| calls.push((name, weapon)));
}

/// The equip callback, which notes the weapon.
fn on_equip(_server: Server<'_>, _player: Entity<'_>, weapon: Entity<'_>) {
	note("equipped", weapon.as_ptr().addr());
}

/// The frame callback, which notes the timing.
fn on_frame(_server: Server<'_>, timing: HookTiming, _weapon: Entity<'_>) {
	note(
		match timing {
			HookTiming::Pre => "before",
			HookTiming::Post => "after",
		},
		0,
	);
}

/// The reload callback, which notes the call and decides [`ACTION`].
fn on_reload(_server: Server<'_>, _weapon: Entity<'_>) -> WeaponAction {
	note("reload", 0);
	ACTION.get()
}

/// The smack callback, which notes the call and decides [`SMACK`].
fn on_smack(_server: Server<'_>, _weapon: Entity<'_>) -> SmackAction {
	note("smack", 0);
	SMACK.get()
}

/// The switch callback, which notes the weapon and decides [`ACTION`].
fn on_switch(_server: Server<'_>, _player: Entity<'_>, weapon: Entity<'_>) -> WeaponAction {
	note("switch", weapon.as_ptr().addr());
	ACTION.get()
}

/// A player of a new class, with the game's methods of weapons.
fn player() -> Box<Mock> {
	Mock::of_new_class(&[
		(
			WEAPON_SWITCH_SLOT,
			game_switch as WeaponSwitchFn as *mut c_void,
		),
		(
			WEAPON_CAN_SWITCH_TO_SLOT,
			game_can_switch_to as WeaponPredicateFn as *mut c_void,
		),
		(WEAPON_EQUIP_SLOT, game_equip as WeaponFn as *mut c_void),
	])
}

#[test]
fn reloads_can_be_refused() {
	on_both(|harness| {
		let api = harness.api();
		let mut weapon = weapon();
		let hooks = api.hook_weapon_reloads(tf2_binding(no_interfaces), on_reload);

		assert_eq!(hooks.cover(api, weapon.target::<CombatWeapon>()), Ok(true));

		let mut reload = || {
			CALLS.take();
			let reloads = harness.call::<PredicateFn>(weapon.ptr(), RELOAD_SLOT, ());
			(reloads, CALLS.take())
		};

		ACTION.set(WeaponAction::Continue);
		assert_eq!(reload(), (true, vec![("reload", 0), ("game", 0)]));

		ACTION.set(WeaponAction::Refuse);
		assert_eq!(reload(), (false, vec![("reload", 0)]));
	});
}

/// Calls the method at `slot` of `player`, given `weapon`, and returns its
/// result with what ran.
fn switch(
	harness: &Harness,
	player: &mut Mock,
	slot: usize,
	weapon: *mut sys::CBaseCombatWeapon,
) -> (bool, Vec<(&'static str, usize)>) {
	CALLS.take();

	let result = if slot == WEAPON_SWITCH_SLOT {
		harness.call::<WeaponSwitchFn>(player.ptr(), slot, (weapon, 0))
	} else {
		harness.call::<WeaponPredicateFn>(player.ptr(), slot, (weapon,))
	};

	(result, CALLS.take())
}

#[test]
fn switches_can_be_refused() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = player();
		let mut weapon = weapon();
		let address = weapon.address();
		let binding = tf2_binding(no_interfaces);
		let switches = api.hook_weapon_switches(binding, on_switch);
		let checks = api.hook_weapon_can_switch_to(binding, on_switch);

		for hooks in [&switches, &checks] {
			assert_eq!(hooks.cover(api, player.target::<TfPlayer>()), Ok(true));
		}

		for slot in [WEAPON_SWITCH_SLOT, WEAPON_CAN_SWITCH_TO_SLOT] {
			ACTION.set(WeaponAction::Continue);
			assert_eq!(
				switch(harness, &mut player, slot, weapon.ptr().cast()),
				(true, vec![("switch", address), ("game", address)])
			);

			ACTION.set(WeaponAction::Refuse);
			assert_eq!(
				switch(harness, &mut player, slot, weapon.ptr().cast()),
				(false, vec![("switch", address)])
			);

			// Without a weapon, the game decides alone.
			assert_eq!(
				switch(harness, &mut player, slot, ptr::null_mut()),
				(true, vec![("game", 0)])
			);
		}
	});
}

/// A method no test calls.
unsafe extern "C" fn unexpected(_: *mut sys::CBaseEntity) {
	unreachable!("no test calls this slot");
}

/// A weapon of a new class, with the game's frames and reloads.
fn weapon() -> Box<Mock> {
	Mock::of_new_class(&[
		(ITEM_POST_FRAME_SLOT, game_frame as EntityFn as *mut c_void),
		(RELOAD_SLOT, game_reload as PredicateFn as *mut c_void),
	])
}

#[test]
fn weapon_frames_run_between_the_hooks() {
	on_both(|harness| {
		let api = harness.api();
		let mut weapon = weapon();
		let hooks = api.hook_weapon_frames(tf2_binding(no_interfaces), on_frame);

		assert_eq!(hooks.cover(api, weapon.target::<CombatWeapon>()), Ok(true));

		CALLS.take();
		harness.call::<EntityFn>(weapon.ptr(), ITEM_POST_FRAME_SLOT, ());
		assert_eq!(CALLS.take(), [("before", 0), ("game", 0), ("after", 0)]);
	});
}

/// Exercises the passive source hook and existing impact scheduler together.
/// This models the pinned native ordering; it does not render a client animation.
mod secondary {
	use super::*;
	use std::marker::PhantomData;

	thread_local! {
		static ACTIVE: Cell<*const Harness> = const { Cell::new(ptr::null()) };
		static OWNED: Cell<bool> = const { Cell::new(false) };
		static SOURCES: RefCell<Vec<(usize, bool)>> = const { RefCell::new(Vec::new()) };
		static TIME: Cell<f32> = const { Cell::new(0.0) };
		static MODE: Cell<c_int> = const { Cell::new(1) };
		static PANIC_NEXT: Cell<bool> = const { Cell::new(false) };
	}

	/// The original mock attack calls the mock hook backend again, like native
	/// Swing's virtual GetSmackTime call. The borrowed harness outlives the guard.
	struct Calls<'a> {
		previous: *const Harness,
		_scope: PhantomData<&'a Harness>,
	}

	impl<'a> Calls<'a> {
		fn new(harness: &'a Harness) -> Self {
			SOURCES.with_borrow_mut(|sources| sources.clear());
			Self {
				previous: ACTIVE.replace(harness),
				_scope: PhantomData,
			}
		}
	}

	impl Drop for Calls<'_> {
		fn drop(&mut self) {
			ACTIVE.set(self.previous);
			SOURCES.with_borrow_mut(|sources| sources.clear());
			PANIC_NEXT.set(false);
		}
	}

	#[test]
	fn a_panicking_source_observer_cannot_refuse_the_original_attack() {
		on_both(|harness| {
			let _calls = Calls::new(harness);
			let mut weapon = fists();
			let _hooks = install(harness, &weapon);
			OWNED.set(true);
			MODE.set(1);
			PANIC_NEXT.set(true);
			let seen = secondary(harness, &mut weapon);
			assert!(seen.contains(&("animation", 1)));
			assert!(seen.contains(&("punch-bookkeeping", 0)));
			assert!(seen.contains(&("authoritative-trace", 0)));
			assert!(SOURCES.with_borrow(|sources| sources.is_empty()));
			assert_eq!(TIME.get(), 10.2);
			assert_eq!(
				secondary(harness, &mut weapon).last(),
				Some(&("secondary-post", 0))
			);
			assert_eq!(TIME.get(), NO_SMACK);
		});
	}

	#[test]
	fn an_owned_secondary_miss_preserves_the_entire_attack_and_primary_schedule() {
		on_both(|harness| {
			let _calls = Calls::new(harness);
			let mut weapon = fists();
			let _hooks = install(harness, &weapon);
			OWNED.set(true);
			// Fists swaps its internal weapon mode when the viewmodel is flipped.
			for mode in [0, 1] {
				MODE.set(mode);
				assert_eq!(
					secondary(harness, &mut weapon),
					[
						("secondary-pre", 0),
						("animation", mode as usize),
						("swing-sound", 0),
						("attack-deadlines", 0),
						("schedule", 0),
						("punch-bookkeeping", 0),
						("secondary-post", 0),
					]
				);
				assert_eq!(TIME.get(), NO_SMACK);
				assert!(SOURCES.with_borrow(|sources| sources.is_empty()));
			}
			CALLS.take();
			let primary = harness.call::<SmackTimeFn>(weapon.ptr(), GET_SMACK_TIME_SLOT, (1,));
			assert_eq!(primary, 10.2);
			assert_eq!(CALLS.take(), [("schedule", 0), ("game", 1)]);
		});
	}

	fn fists() -> Box<Mock> {
		Mock::of_new_class(&[
			(
				SECONDARY_ATTACK_SLOT,
				game_secondary as EntityFn as *mut c_void,
			),
			(
				GET_SMACK_TIME_SLOT,
				game_smack_time as SmackTimeFn as *mut c_void,
			),
		])
	}

	unsafe extern "C" fn game_secondary(weapon: *mut sys::CBaseEntity) {
		note("animation", MODE.get() as usize);
		note("swing-sound", 0);
		note("attack-deadlines", 0);
		let at = ACTIVE.with(|active| {
			// SAFETY: Calls retains the shared harness borrow through this attack.
			let harness = unsafe { active.get().as_ref() }.unwrap();
			harness.call::<SmackTimeFn>(weapon, GET_SMACK_TIME_SLOT, (MODE.get(),))
		});
		TIME.set(at);
		note("punch-bookkeeping", 0);
	}

	fn install(
		harness: &Harness,
		weapon: &Mock,
	) -> (ClassHooks<CombatWeapon>, ClassHooks<TfMeleeWeapon>) {
		let api = harness.api();
		let source = api.hook_weapon_secondary_attacks(tf2_binding(no_interfaces), on_source);
		let schedule = api.hook_melee_smacks(tf2_binding(no_interfaces), on_schedule);
		assert_eq!(source.cover(api, weapon.target::<CombatWeapon>()), Ok(true));
		assert_eq!(
			schedule.cover(api, weapon.target::<TfMeleeWeapon>()),
			Ok(true)
		);
		(source, schedule)
	}

	/// Mirrors ItemPostFrame's positive absolute-time gate, with the trace
	/// represented by a recorded event instead of an invented entity field.
	fn later_frame() {
		if TIME.get() > 0.0 && 10.3 > TIME.get() {
			note("authoritative-trace", 0);
		}
	}

	fn on_schedule(_server: Server<'_>, weapon: Entity<'_>) -> SmackAction {
		note("schedule", 0);
		if SOURCES.with_borrow(|sources| {
			sources
				.last()
				.is_some_and(|&(source, owned)| source == weapon.as_ptr() as usize && owned)
		}) {
			SmackAction::Miss
		} else {
			SmackAction::Continue
		}
	}

	fn on_source(_server: Server<'_>, timing: HookTiming, weapon: Entity<'_>) {
		let address = weapon.as_ptr() as usize;
		match timing {
			HookTiming::Pre => {
				note("secondary-pre", 0);
				if PANIC_NEXT.replace(false) {
					panic!("observer failed");
				}
				SOURCES.with_borrow_mut(|sources| sources.push((address, OWNED.get())));
			}

			HookTiming::Post => {
				note("secondary-post", 0);
				SOURCES.with_borrow_mut(|sources| sources.pop());
			}
		}
	}

	#[test]
	fn ordinary_back_punches_and_uncovered_weapons_keep_native_impacts() {
		on_both(|harness| {
			let _calls = Calls::new(harness);
			let mut weapon = fists();
			let mut other = fists();
			let _hooks = install(harness, &weapon);
			OWNED.set(false);
			MODE.set(1);
			let ordinary = secondary(harness, &mut weapon);
			assert!(ordinary.contains(&("authoritative-trace", 0)));
			assert_eq!(TIME.get(), 10.2);
			OWNED.set(true);
			let uncovered = secondary(harness, &mut other);
			assert!(!uncovered.contains(&("secondary-pre", 0)));
			assert!(!uncovered.contains(&("schedule", 0)));
			assert!(uncovered.contains(&("animation", 1)));
			assert!(uncovered.contains(&("punch-bookkeeping", 0)));
			assert!(uncovered.contains(&("authoritative-trace", 0)));
		});
	}

	#[test]
	fn pause_removal_and_unload_keep_native_attack_flow() {
		on_both(|harness| {
			let _calls = Calls::new(harness);
			let mut weapon = fists();
			let (source, schedule) = install(harness, &weapon);
			OWNED.set(true);
			MODE.set(1);
			for (loaded, paused) in [(true, true), (false, false)] {
				harness.set_status(loaded, paused, harness.generation);
				let seen = secondary(harness, &mut weapon);
				assert!(seen.contains(&("animation", 1)));
				assert!(seen.contains(&("punch-bookkeeping", 0)));
				assert!(seen.contains(&("authoritative-trace", 0)));
				assert!(!seen.contains(&("secondary-pre", 0)));
			}
			harness.set_status(true, false, harness.generation);
			source.remove(harness.api());
			schedule.remove(harness.api());
			let seen = secondary(harness, &mut weapon);
			assert!(seen.contains(&("animation", 1)));
			assert!(seen.contains(&("authoritative-trace", 0)));
			assert!(!seen.contains(&("schedule", 0)));
		});
	}

	fn secondary(harness: &Harness, weapon: &mut Mock) -> Vec<(&'static str, usize)> {
		CALLS.take();
		harness.call::<EntityFn>(weapon.ptr(), SECONDARY_ATTACK_SLOT, ());
		later_frame();
		CALLS.take()
	}
}
