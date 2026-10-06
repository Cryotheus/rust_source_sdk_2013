//! Tests of `crate::airblast_hooks`: pre hooks of `DeflectPlayer` on mock
//! weapon classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, expect, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::{InterfaceFactory, sys};
use std::cell::RefCell;

thread_local! {
	/// What ran during the calls since the last [`push`], in order.
	static CALLS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

	/// What the callback decides.
	static ACTION: Cell<AirblastAction> = const { Cell::new(AirblastAction::Allow) };

	/// What the callback was last given: the weapon's kind, and the addresses
	/// of the weapon, the player the airblast reached, and its owner.
	static SEEN: Cell<Option<(AirblastWeapon, usize, usize, usize)>> = const { Cell::new(None) };
}

/// Where each push's owner aims.
const FORWARD: usize = 0xf0e;

/// The owner of each weapon pushing.
const OWNER: usize = 0x0e4;

/// The player each push reaches.
const TARGET: usize = 0x7a6;

/// A weapon of a C++ class, as hooks know it.
#[repr(C)]
struct Weapon {
	vtable: *mut *mut c_void,
}

impl Weapon {
	/// The vtable of a new class, which holds `deflect_projectiles` at
	/// [`DEFLECT_PROJECTILES_SLOT`] and `deflect_player` at
	/// [`DEFLECT_PLAYER_SLOT`].
	fn new_class(
		deflect_projectiles: DeflectProjectiles,
		deflect_player: DeflectPlayer,
	) -> NonNull<*mut c_void> {
		let mut slots =
			vec![unexpected as DeflectProjectiles as *mut c_void; DEFLECT_PLAYER_SLOT + 1];

		slots[DEFLECT_PROJECTILES_SLOT] = deflect_projectiles as *mut c_void;
		slots[DEFLECT_PLAYER_SLOT] = deflect_player as *mut c_void;

		NonNull::new(Vec::leak(slots).as_mut_ptr()).unwrap()
	}

	/// A weapon of the class whose vtable is `vtable`.
	fn of_class(vtable: NonNull<*mut c_void>) -> Box<Self> {
		Box::new(Self {
			vtable: vtable.as_ptr(),
		})
	}

	fn ptr(&mut self) -> *mut sys::CBaseEntity {
		(&raw mut *self).cast()
	}
}

/// `CTFWeaponBase::DeflectProjectiles`, which no test runs.
unsafe extern "C" fn deflect_projectiles(_this: *mut sys::CBaseEntity) -> bool {
	expect(false, "an airblast ran");
	false
}

/// `CTFWeaponBase::DeflectPlayer`, which notes that it ran.
unsafe extern "C" fn ignore_player(
	_this: *mut sys::CBaseEntity,
	_target: *mut sys::CBaseEntity,
	_owner: *mut sys::CBaseEntity,
	_forward: *mut sys::Vector,
) -> bool {
	CALLS.with_borrow_mut(|calls| calls.push("ignore"));
	true
}

/// Installs the hooks on `vtables` and `reference`, with [`on_airblast`].
fn install(
	harness: &Harness,
	(vtables, reference): ([NonNull<*mut c_void>; 2], NonNull<*mut c_void>),
) -> Result<AirblastHooks, AirblastHookError> {
	// SAFETY: The mock classes have `DeflectProjectiles` and `DeflectPlayer`
	// at their slots, and are leaked.
	unsafe {
		harness
			.api()
			.install_airblasts(vtables, reference, tf2_binding(no_interfaces), on_airblast)
	}
}

/// The vtables of a new flame thrower class, a new Dragon's Fury class, and a
/// new rocket launcher class, as the game lays them out.
fn new_classes() -> ([NonNull<*mut c_void>; 2], NonNull<*mut c_void>) {
	(
		[
			Weapon::new_class(deflect_projectiles, push_player),
			Weapon::new_class(deflect_projectiles, push_player),
		],
		Weapon::new_class(deflect_projectiles, ignore_player),
	)
}

/// The callback, which notes that it ran and what it was given, and decides
/// as [`ACTION`] says.
fn on_airblast(_server: Server<'_>, push: AirblastPush<'_>) -> AirblastAction {
	CALLS.with_borrow_mut(|calls| calls.push("callback"));

	SEEN.set(Some((
		push.kind,
		push.weapon.as_ptr().addr(),
		push.target.as_ptr().addr(),
		push.owner.as_ptr().addr(),
	)));

	ACTION.get()
}

/// Another weapon class's `DeflectPlayer`.
unsafe extern "C" fn other_push(
	_this: *mut sys::CBaseEntity,
	_target: *mut sys::CBaseEntity,
	_owner: *mut sys::CBaseEntity,
	_forward: *mut sys::Vector,
) -> bool {
	CALLS.with_borrow_mut(|calls| calls.push("other"));
	true
}

#[test]
fn pauses_and_reloads_let_the_weapons_push() {
	on_both(|harness| {
		let api = harness.api();
		let classes = new_classes();
		let mut weapon = Weapon::of_class(classes.0[0]);

		ACTION.set(AirblastAction::Refuse);

		let hooks = install(harness, classes).unwrap();

		harness.set_status(true, true, harness.generation);
		assert_eq!(push(harness, &mut weapon), (true, vec!["push"]));

		// A later load, which has not installed its own hooks yet. Its
		// generation is one no later harness takes, as it installs hooks.
		harness.set_status(true, false, harness.generation | 1 << 63);
		assert_eq!(push(harness, &mut weapon), (true, vec!["push"]));
		assert!(!hooks.hooks.iter().any(|&hook| api.has_hook(hook)));

		let _hooks = install(harness, classes).unwrap();
		assert_eq!(push(harness, &mut weapon), (false, vec!["callback"]));
	});
}

/// Runs `weapon`'s hooked `DeflectPlayer` on [`TARGET`] for [`OWNER`], and
/// returns its result and what ran.
fn push(harness: &Harness, weapon: &mut Weapon) -> (bool, Vec<&'static str>) {
	CALLS.take();

	let args = (
		ptr::without_provenance_mut(TARGET),
		ptr::without_provenance_mut(OWNER),
		ptr::without_provenance_mut(FORWARD),
	);
	let pushed = harness.call::<DeflectPlayer>(weapon.ptr(), DEFLECT_PLAYER_SLOT, args);

	(pushed, CALLS.take())
}

/// The flame throwers' `DeflectPlayer`, which notes that it ran, and pushes.
unsafe extern "C" fn push_player(
	_this: *mut sys::CBaseEntity,
	target: *mut sys::CBaseEntity,
	owner: *mut sys::CBaseEntity,
	forward: *mut sys::Vector,
) -> bool {
	expect(
		target.addr() == TARGET && owner.addr() == OWNER && forward.addr() == FORWARD,
		"the weapon was given other players",
	);
	CALLS.with_borrow_mut(|calls| calls.push("push"));
	true
}

#[test]
fn pushes_an_earlier_hook_skipped_run_no_callback() {
	on_both(|harness| {
		let classes = new_classes();
		let mut weapon = Weapon::of_class(classes.0[0]);

		fn skip(_call: &HookCall<'_, DeflectPlayer>) -> HookAction<bool> {
			HookAction::Supersede(true)
		}

		// SAFETY: As for `install`.
		unsafe {
			harness.api().add_hook(
				DEFLECT_PLAYER,
				HookTarget::vtable(classes.0[0]),
				HookTiming::Pre,
				&skip,
			)
		}
		.unwrap();

		ACTION.set(AirblastAction::Refuse);
		let _hooks = install(harness, classes).unwrap();
		assert_eq!(push(harness, &mut weapon), (true, vec![]));
	});
}

#[test]
fn refused_pushes_skip_the_weapons_function() {
	on_both(|harness| {
		let classes = new_classes();
		let mut flame_thrower = Weapon::of_class(classes.0[0]);
		let mut dragons_fury = Weapon::of_class(classes.0[1]);
		let mut rocket_launcher = Weapon::of_class(classes.1);
		let _hooks = install(harness, classes).unwrap();

		ACTION.set(AirblastAction::Allow);
		assert_eq!(
			push(harness, &mut flame_thrower),
			(true, vec!["callback", "push"])
		);
		assert_eq!(
			SEEN.take(),
			Some((
				AirblastWeapon::FlameThrower,
				flame_thrower.ptr().addr(),
				TARGET,
				OWNER
			))
		);

		// The weapon's function does not run, and the player counts as not
		// deflected.
		ACTION.set(AirblastAction::Refuse);
		assert_eq!(push(harness, &mut flame_thrower), (false, vec!["callback"]));
		assert_eq!(push(harness, &mut dragons_fury), (false, vec!["callback"]));
		assert_eq!(
			SEEN.take(),
			Some((
				AirblastWeapon::DragonsFury,
				dragons_fury.ptr().addr(),
				TARGET,
				OWNER
			))
		);

		// Weapons without airblast are not hooked.
		assert_eq!(push(harness, &mut rocket_launcher), (true, vec!["ignore"]));
		assert_eq!(SEEN.take(), None);
	});
}

#[test]
fn the_layout_is_checked_on_the_functions_before_any_hook() {
	on_both(|harness| {
		let classes = new_classes();
		let mut weapon = Weapon::of_class(classes.0[1]);

		// A hook of the Dragon's Fury's class alone, as another plugin's could
		// be, which SourceHook patches into the class's vtable.
		fn ignore(_call: &HookCall<'_, DeflectPlayer>) -> HookAction<bool> {
			HookAction::Ignore
		}

		// SAFETY: As for `install`.
		unsafe {
			harness.api().add_hook(
				DEFLECT_PLAYER,
				HookTarget::vtable(classes.0[1]),
				HookTiming::Pre,
				&ignore,
			)
		}
		.unwrap();

		let _hooks = install(harness, classes).unwrap();
		ACTION.set(AirblastAction::Refuse);
		assert_eq!(push(harness, &mut weapon), (false, vec!["callback"]));
	});
}

#[test]
fn the_weapons_are_hooked_once_until_removed() {
	on_both(|harness| {
		let api = harness.api();
		let classes = new_classes();
		let mut weapon = Weapon::of_class(classes.0[0]);
		let hooks = install(harness, classes).unwrap();

		assert!(matches!(
			install(harness, new_classes()),
			Err(AirblastHookError::Hook(HookError::AlreadyInstalled))
		));

		// Removed hooks leave the pushes to the game, and can be installed again.
		ACTION.set(AirblastAction::Refuse);
		hooks.remove(api);
		assert_eq!(push(harness, &mut weapon), (true, vec!["push"]));

		install(harness, classes).unwrap().remove(api);
	});
}

#[test]
fn the_weapons_are_searched_for_unless_already_hooked() {
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
			// SAFETY: The search fails before any class is hooked.
			unsafe { api.hook_airblasts(other_server, other, on_airblast) },
			Err(AirblastHookError::Target(AirblastVtableError::WrongGame))
		));

		let binding = tf2_binding(no_interfaces);
		// SAFETY: The server's game module is this test's executable, which
		// `no_interfaces` is in, and which has no weapon classes.
		let server = unsafe { binding.server(&scope) };

		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.hook_airblasts(server, binding, on_airblast) },
			Err(AirblastHookError::Target(AirblastVtableError::NotFound(
				"CTFFlameThrower"
			)))
		));

		let _hooks = install(harness, new_classes()).unwrap();

		// Installed hooks are reported without searching the module again.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.hook_airblasts(server, binding, on_airblast) },
			Err(AirblastHookError::Hook(HookError::AlreadyInstalled))
		));
	});
}

/// A slot of a mock vtable that no test calls.
unsafe extern "C" fn unexpected(_this: *mut sys::CBaseEntity) -> bool {
	expect(false, "an unhooked slot was called");
	false
}

#[test]
fn vtables_not_laid_out_as_the_weapons_are_not_hooked() {
	on_both(|harness| {
		let ([flame_thrower, dragons_fury], reference) = new_classes();
		let own_push = Weapon::new_class(deflect_projectiles, other_push);
		let no_push = Weapon::new_class(deflect_projectiles, ignore_player);
		let own_airblast = Weapon::new_class(unexpected, push_player);

		// Both weapons keep the reference's `DeflectProjectiles`, and share a
		// `DeflectPlayer` of their own.
		for classes in [
			([flame_thrower, own_push], reference),
			([no_push, dragons_fury], reference),
			([flame_thrower, own_airblast], reference),
			([flame_thrower, dragons_fury], flame_thrower),
		] {
			assert!(matches!(
				install(harness, classes),
				Err(AirblastHookError::UnexpectedLayout)
			));
		}

		ACTION.set(AirblastAction::Refuse);
		assert_eq!(
			push(harness, &mut Weapon::of_class(flame_thrower)),
			(true, vec!["push"])
		);
	});
}
