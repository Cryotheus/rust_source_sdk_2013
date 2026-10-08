//! Tests of `crate::weapon_hooks`: hooks of weapons' and players' weapon
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

/// The game's `Weapon_Switch`, which notes that it ran, and switches.
unsafe extern "C" fn game_switch(
	_: *mut sys::CBaseEntity,
	weapon: *mut sys::CBaseCombatWeapon,
	_: c_int,
) -> bool {
	note("game", weapon.addr());
	true
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
