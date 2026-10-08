//! Tests of `crate::building_hooks`: hooks of the methods of mock building
//! classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, expect, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::tf2::class_targets::ClassTargets;
use source_sdk_2013::tf2::damage::DamageType;
use source_sdk_2013::{InterfaceFactory, sys};
use std::cell::RefCell;
use std::ptr;

thread_local! {
	/// What the wrench hit callback decides.
	static ACTION: Cell<WrenchHitAction> = const { Cell::new(WrenchHitAction::Continue) };

	/// What ran during the calls since the last [`run`], in order.
	static CALLS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

	/// The class and the address of the building the last callback was given.
	static SEEN: Cell<Option<(BuildingClass, usize)>> = const { Cell::new(None) };
}

/// The methods every class keeps unless it overrides them.
const BASE: Methods = Methods {
	finished: base_finished,
	killed: base_killed,
	upgrading: base_upgrading,
	wrench_hit: base_wrench_hit,
};

/// The engineer whose wrench hits.
const PLAYER: usize = 0x9a7;

/// Where each hit lands.
const POSITION: [f32; 3] = [1.5, -2.25, 64.0];

/// The engineer's wrench.
const WRENCH: usize = 0x3e1;

/// A building of a C++ class, as hooks know it.
#[repr(C)]
struct Building {
	vtable: *mut *mut c_void,
}

impl Building {
	/// The vtable of a new class, which holds `methods` at their slots.
	fn new_class(methods: Methods) -> NonNull<*mut c_void> {
		let length = [
			FINISHED_BUILDING_SLOT,
			INPUT_WRENCH_HIT_SLOT,
			KILLED_SLOT,
			START_UPGRADING_SLOT,
		]
		.into_iter()
		.max()
		.unwrap();
		let mut slots = vec![unexpected as FinishedBuilding as *mut c_void; length + 1];

		slots[FINISHED_BUILDING_SLOT] = methods.finished as *mut c_void;
		slots[INPUT_WRENCH_HIT_SLOT] = methods.wrench_hit as *mut c_void;
		slots[KILLED_SLOT] = methods.killed as *mut c_void;
		slots[START_UPGRADING_SLOT] = methods.upgrading as *mut c_void;

		NonNull::new(Vec::leak(slots).as_mut_ptr()).unwrap()
	}

	/// A building of the class whose vtable is `vtable`.
	fn of_class(vtable: NonNull<*mut c_void>) -> Box<Self> {
		Box::new(Self {
			vtable: vtable.as_ptr(),
		})
	}

	fn ptr(&mut self) -> *mut sys::CBaseEntity {
		(&raw mut *self).cast()
	}
}

/// A class's building methods.
#[derive(Clone, Copy)]
struct Methods {
	finished: FinishedBuilding,
	killed: Killed,
	upgrading: StartUpgrading,
	wrench_hit: InputWrenchHit,
}

/// `CBaseObject::FinishedBuilding`, which notes that it ran.
unsafe extern "C" fn base_finished(_this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push("finished"));
}

/// `CBaseObject::Killed`, which checks the damage and notes that it ran.
unsafe extern "C" fn base_killed(_this: *mut sys::CBaseEntity, info: *const sys::CTakeDamageInfo) {
	// SAFETY: The tests pass a constructed local damage record.
	let amount = unsafe { (&raw const (*info).m_flDamage).read() };

	expect(
		amount == 11.0,
		"the building received another damage record",
	);
	CALLS.with_borrow_mut(|calls| calls.push("killed"));
}

/// `CBaseObject::StartUpgrading`, which notes that it ran.
unsafe extern "C" fn base_upgrading(_this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push("upgrading"));
}

/// `CBaseObject::InputWrenchHit`, which checks the hit, notes that it ran, and
/// does something.
unsafe extern "C" fn base_wrench_hit(
	_this: *mut sys::CBaseEntity,
	player: *mut sys::CBaseEntity,
	wrench: *mut sys::CBaseEntity,
	position: sys::Vector,
) -> bool {
	check_hit(player, wrench, position);
	CALLS.with_borrow_mut(|calls| calls.push("wrench hit"));
	true
}

#[test]
fn buildings_are_searched_for_only_to_hook_them() {
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
			unsafe { api.hook_buildings(other_server, other, callbacks()) },
			Err(BuildingHookError::Target(BuildingVtableError::WrongGame))
		));

		let binding = tf2_binding(no_interfaces);
		// SAFETY: The server's game module is this test's executable, which
		// `no_interfaces` is in, and which has no building classes.
		let server = unsafe { binding.server(&scope) };

		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.hook_buildings(server, binding, callbacks()) },
			Err(BuildingHookError::Target(BuildingVtableError::NotFound(
				BuildingClass::CartDispenser
			)))
		));

		// Nor in a snapshot other hooks share.
		let targets = ClassTargets::load(server).unwrap();

		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.hook_building_classes(&targets, binding, callbacks()) },
			Err(BuildingHookError::Target(BuildingVtableError::NotFound(
				BuildingClass::CartDispenser
			)))
		));

		// Without callbacks, there is nothing to search for.
		// SAFETY: As above.
		let none = unsafe { api.hook_buildings(server, binding, BuildingCallbacks::default()) };
		assert!(none.unwrap().hooks.is_empty());

		let _hooks = install(harness, new_classes(), callbacks()).unwrap();

		// Installed hooks are reported without searching the module again.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.hook_buildings(server, binding, callbacks()) },
			Err(BuildingHookError::Hook(HookError::AlreadyInstalled))
		));
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.hook_building_classes(&targets, binding, callbacks()) },
			Err(BuildingHookError::Hook(HookError::AlreadyInstalled))
		));
	});
}

/// Every callback.
fn callbacks() -> BuildingCallbacks {
	BuildingCallbacks {
		finished: Some(on_finished),
		killed: Some(on_killed),
		upgrading: Some(on_upgrading),
		wrench_hit: Some(on_wrench_hit),
	}
}

/// Checks that a building's `InputWrenchHit` was given [`PLAYER`]'s
/// [`WRENCH`]'s hit at [`POSITION`].
fn check_hit(player: *mut sys::CBaseEntity, wrench: *mut sys::CBaseEntity, position: sys::Vector) {
	expect(
		player.addr() == PLAYER && wrench.addr() == WRENCH,
		"the building was hit by another wrench",
	);
	expect(
		[position.x, position.y, position.z] == POSITION,
		"the building was hit elsewhere",
	);
}

/// The dispensers' `StartUpgrading`.
unsafe extern "C" fn dispenser_upgrading(_this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push("dispenser upgrading"));
}

/// Runs `building`'s hooked `FinishedBuilding`, and returns what ran.
fn finish(harness: &Harness, building: &mut Building) -> Vec<&'static str> {
	run(|| harness.call::<FinishedBuilding>(building.ptr(), FINISHED_BUILDING_SLOT, ()))
}

/// Runs `building`'s hooked `InputWrenchHit` by [`PLAYER`]'s [`WRENCH`] at
/// [`POSITION`], and returns its result and what ran.
fn hit(harness: &Harness, building: &mut Building) -> (bool, Vec<&'static str>) {
	let [x, y, z] = POSITION;
	let args = (
		ptr::without_provenance_mut(PLAYER),
		ptr::without_provenance_mut(WRENCH),
		sys::Vector { x, y, z },
	);
	let mut result = false;
	let calls = run(|| {
		result = harness.call::<InputWrenchHit>(building.ptr(), INPUT_WRENCH_HIT_SLOT, args);
	});

	(result, calls)
}

#[test]
fn hooked_methods_run_their_callbacks_around_the_game() {
	on_both(|harness| {
		let classes = new_classes();
		let _hooks = install(harness, classes, callbacks()).unwrap();

		ACTION.set(WrenchHitAction::Continue);

		for (class, vtable) in BuildingClass::ALL.into_iter().zip(classes) {
			let mut building = Building::of_class(vtable);
			let seen = Some((class, building.ptr().addr()));

			let killed = match class {
				BuildingClass::Sapper => "sapper killed",
				BuildingClass::Sentry => "sentry killed",
				_ => "killed",
			};

			assert_eq!(kill(harness, &mut building), ["on killed", killed]);
			assert_eq!(SEEN.take(), seen);

			let finished = match class {
				BuildingClass::Sapper => "sapper finished",
				BuildingClass::Teleporter => "teleporter finished",
				_ => "finished",
			};

			assert_eq!(finish(harness, &mut building), [finished, "on finished"]);
			assert_eq!(SEEN.take(), seen);

			let upgrading = match class {
				BuildingClass::Sapper => "upgrading",
				BuildingClass::Sentry => "sentry upgrading",
				BuildingClass::Teleporter => "teleporter upgrading",
				_ => "dispenser upgrading",
			};

			assert_eq!(upgrade(harness, &mut building), [upgrading, "on upgrading"]);
			assert_eq!(SEEN.take(), seen);

			let wrench_hit = match class {
				BuildingClass::Teleporter => "teleporter wrench hit",
				_ => "wrench hit",
			};

			assert_eq!(
				hit(harness, &mut building),
				(true, vec!["on wrench hit", wrench_hit])
			);
			assert_eq!(SEEN.take(), seen);
		}
	});
}

/// Installs the hooks of `callbacks` on `vtables`.
fn install(
	harness: &Harness,
	vtables: [NonNull<*mut c_void>; 7],
	callbacks: BuildingCallbacks,
) -> Result<BuildingHooks, BuildingHookError> {
	// SAFETY: The mock classes have the building methods at their slots, and
	// are leaked.
	unsafe {
		harness
			.api()
			.install_buildings(vtables, tf2_binding(no_interfaces), callbacks)
	}
}

/// Runs `building`'s hooked `Killed` with 11 bullet damage, and returns what
/// ran.
fn kill(harness: &Harness, building: &mut Building) -> Vec<&'static str> {
	let info = DamageInfo::new(11.0, DamageType::BULLET);
	let calls = run(|| harness.call::<Killed>(building.ptr(), KILLED_SLOT, (info.as_ptr(),)));

	assert_eq!(
		info.amount(),
		11.0,
		"a const source record must never be overwritten"
	);
	calls
}

#[test]
fn methods_an_earlier_hook_skipped_run_no_callback() {
	on_both(|harness| {
		let classes = new_classes();
		let mut building = Building::of_class(classes[1]);

		fn skip_hit(_call: &HookCall<'_, InputWrenchHit>) -> HookAction<bool> {
			HookAction::Supersede(true)
		}

		fn skip_death(_call: &HookCall<'_, Killed>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: As for `install`.
		unsafe {
			let api = harness.api();
			let target = HookTarget::vtable(classes[1]);

			api.add_hook(INPUT_WRENCH_HIT, target, HookTiming::Pre, &skip_hit)
				.unwrap();
			api.add_hook(KILLED, target, HookTiming::Pre, &skip_death)
				.unwrap();
		}

		ACTION.set(WrenchHitAction::Refuse);
		let _hooks = install(harness, classes, callbacks()).unwrap();

		assert_eq!(hit(harness, &mut building), (true, vec![]));
		assert_eq!(kill(harness, &mut building), Vec::<&str>::new());
		assert_eq!(SEEN.take(), None);
	});
}

/// The vtables of a new class of each of [`BuildingClass::ALL`], as the game
/// lays them out.
fn new_classes() -> [NonNull<*mut c_void>; 7] {
	let dispenser = Methods {
		upgrading: dispenser_upgrading,
		..BASE
	};
	let sapper = Methods {
		finished: sapper_finished,
		killed: sapper_killed,
		..BASE
	};
	let sentry = Methods {
		killed: sentry_killed,
		upgrading: sentry_upgrading,
		..BASE
	};
	let teleporter = Methods {
		finished: teleporter_finished,
		upgrading: teleporter_upgrading,
		wrench_hit: teleporter_wrench_hit,
		..BASE
	};

	[
		dispenser, dispenser, dispenser, dispenser, sapper, sentry, teleporter,
	]
	.map(Building::new_class)
}

/// The finished callback, which notes that it ran and what it was given.
fn on_finished(_server: Server<'_>, event: BuildingEvent<'_>) {
	CALLS.with_borrow_mut(|calls| calls.push("on finished"));
	SEEN.set(Some((event.class, event.building.as_ptr().addr())));
}

/// The killed callback, which checks the copy of the damage it was given, and
/// notes that it ran and the building.
fn on_killed(_server: Server<'_>, event: BuildingEvent<'_>, info: &DamageInfo) {
	expect(
		info.amount() == 11.0 && info.damage_type() == DamageType::BULLET,
		"the callback saw another damage record",
	);
	CALLS.with_borrow_mut(|calls| calls.push("on killed"));
	SEEN.set(Some((event.class, event.building.as_ptr().addr())));
}

/// The upgrading callback, which notes that it ran and what it was given.
fn on_upgrading(_server: Server<'_>, event: BuildingEvent<'_>) {
	CALLS.with_borrow_mut(|calls| calls.push("on upgrading"));
	SEEN.set(Some((event.class, event.building.as_ptr().addr())));
}

/// The wrench hit callback, which checks the hit, notes that it ran and the
/// building, and decides as [`ACTION`] says.
fn on_wrench_hit(_server: Server<'_>, hit: WrenchHit<'_>) -> WrenchHitAction {
	expect(
		hit.player.as_ptr().addr() == PLAYER && hit.wrench.as_ptr().addr() == WRENCH,
		"the callback saw another wrench",
	);
	expect(
		hit.position.to_array() == POSITION,
		"the callback saw the hit elsewhere",
	);
	CALLS.with_borrow_mut(|calls| calls.push("on wrench hit"));
	SEEN.set(Some((hit.class, hit.building.as_ptr().addr())));
	ACTION.get()
}

#[test]
fn only_methods_with_a_callback_are_hooked() {
	on_both(|harness| {
		let classes = new_classes();
		let mut building = Building::of_class(classes[5]);
		let killed = BuildingCallbacks {
			killed: Some(on_killed),
			..BuildingCallbacks::default()
		};
		let _hooks = install(harness, classes, killed).unwrap();

		assert_eq!(finish(harness, &mut building), ["finished"]);
		assert_eq!(upgrade(harness, &mut building), ["sentry upgrading"]);
		assert_eq!(hit(harness, &mut building), (true, vec!["wrench hit"]));
		assert_eq!(kill(harness, &mut building), ["on killed", "sentry killed"]);

		// Any of the hooks keeps the others from being installed apart.
		let finished = BuildingCallbacks {
			finished: Some(on_finished),
			..BuildingCallbacks::default()
		};

		assert!(matches!(
			install(harness, classes, finished),
			Err(BuildingHookError::Hook(HookError::AlreadyInstalled))
		));
	});
}

#[test]
fn pauses_and_reloads_let_the_buildings_act() {
	on_both(|harness| {
		let api = harness.api();
		let classes = new_classes();
		let mut building = Building::of_class(classes[6]);

		ACTION.set(WrenchHitAction::Refuse);

		let hooks = install(harness, classes, callbacks()).unwrap();

		harness.set_status(true, true, harness.generation);
		assert_eq!(
			hit(harness, &mut building),
			(true, vec!["teleporter wrench hit"])
		);

		// A later load, which has not installed its own hooks yet. Its
		// generation is one no later harness takes, as it installs hooks.
		harness.set_status(true, false, harness.generation | 1 << 63);
		assert_eq!(
			hit(harness, &mut building),
			(true, vec!["teleporter wrench hit"])
		);
		assert!(!hooks.hooks.iter().any(|&hook| api.has_hook(hook)));

		let _hooks = install(harness, classes, callbacks()).unwrap();
		assert_eq!(hit(harness, &mut building), (false, vec!["on wrench hit"]));
	});
}

#[test]
fn refused_wrench_hits_skip_the_buildings_function() {
	on_both(|harness| {
		let classes = new_classes();
		let _hooks = install(harness, classes, callbacks()).unwrap();

		ACTION.set(WrenchHitAction::Refuse);

		for vtable in classes {
			let mut building = Building::of_class(vtable);

			assert_eq!(hit(harness, &mut building), (false, vec!["on wrench hit"]));
		}
	});
}

/// Runs `call`, and returns what ran during it.
fn run(call: impl FnOnce()) -> Vec<&'static str> {
	CALLS.take();
	call();
	CALLS.take()
}

/// The sapper's `FinishedBuilding`.
unsafe extern "C" fn sapper_finished(_this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push("sapper finished"));
}

/// The sapper's `Killed`.
unsafe extern "C" fn sapper_killed(
	_this: *mut sys::CBaseEntity,
	_info: *const sys::CTakeDamageInfo,
) {
	CALLS.with_borrow_mut(|calls| calls.push("sapper killed"));
}

/// The sentry gun's `Killed`.
unsafe extern "C" fn sentry_killed(
	_this: *mut sys::CBaseEntity,
	_info: *const sys::CTakeDamageInfo,
) {
	CALLS.with_borrow_mut(|calls| calls.push("sentry killed"));
}

/// The sentry gun's `StartUpgrading`.
unsafe extern "C" fn sentry_upgrading(_this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push("sentry upgrading"));
}

/// The teleporter's `FinishedBuilding`.
unsafe extern "C" fn teleporter_finished(_this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push("teleporter finished"));
}

/// The teleporter's `StartUpgrading`.
unsafe extern "C" fn teleporter_upgrading(_this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push("teleporter upgrading"));
}

/// The teleporter's `InputWrenchHit`, which checks the hit, notes that it ran,
/// and does something.
unsafe extern "C" fn teleporter_wrench_hit(
	_this: *mut sys::CBaseEntity,
	player: *mut sys::CBaseEntity,
	wrench: *mut sys::CBaseEntity,
	position: sys::Vector,
) -> bool {
	check_hit(player, wrench, position);
	CALLS.with_borrow_mut(|calls| calls.push("teleporter wrench hit"));
	true
}

#[test]
fn the_buildings_are_hooked_once_until_removed() {
	on_both(|harness| {
		let api = harness.api();
		let classes = new_classes();
		let mut building = Building::of_class(classes[0]);
		let hooks = install(harness, classes, callbacks()).unwrap();

		assert!(matches!(
			install(harness, new_classes(), callbacks()),
			Err(BuildingHookError::Hook(HookError::AlreadyInstalled))
		));

		// Removed hooks leave the methods to the game, and can be installed
		// again.
		ACTION.set(WrenchHitAction::Refuse);
		hooks.remove(api);
		assert_eq!(hit(harness, &mut building), (true, vec!["wrench hit"]));
		assert_eq!(kill(harness, &mut building), ["killed"]);
		assert_eq!(SEEN.take(), None);

		install(harness, classes, callbacks()).unwrap().remove(api);
	});
}

/// A slot of a mock vtable that no test calls.
unsafe extern "C" fn unexpected(_this: *mut sys::CBaseEntity) {
	expect(false, "an unhooked slot was called");
}

/// Runs `building`'s hooked `StartUpgrading`, and returns what ran.
fn upgrade(harness: &Harness, building: &mut Building) -> Vec<&'static str> {
	run(|| harness.call::<StartUpgrading>(building.ptr(), START_UPGRADING_SLOT, ()))
}

#[test]
fn vtables_not_laid_out_as_the_buildings_are_not_hooked() {
	on_both(|harness| {
		let classes = new_classes();
		let [cart, .., sapper, sentry, teleporter] = classes;

		// The sapper keeps `CBaseObject`'s `Killed`.
		let base_sapper = Methods {
			finished: sapper_finished,
			..BASE
		};
		// The teleporter keeps `CBaseObject`'s `InputWrenchHit`.
		let base_teleporter = Methods {
			finished: teleporter_finished,
			upgrading: teleporter_upgrading,
			..BASE
		};
		// The cart's dispenser has the sentry gun's `StartUpgrading`.
		let own_cart = Methods {
			upgrading: sentry_upgrading,
			..BASE
		};
		// The sapper's `FinishedBuilding` is its `StartUpgrading` too.
		let one_sapper = Methods {
			finished: base_upgrading,
			killed: sapper_killed,
			..BASE
		};

		for (index, methods) in [
			(4, base_sapper),
			(6, base_teleporter),
			(0, own_cart),
			(4, one_sapper),
		] {
			let mut vtables = classes;

			vtables[index] = Building::new_class(methods);
			assert!(matches!(
				install(harness, vtables, callbacks()),
				Err(BuildingHookError::UnexpectedLayout)
			));
		}

		// The classes in another order.
		assert!(matches!(
			install(
				harness,
				[sentry, cart, cart, cart, cart, sapper, teleporter],
				callbacks()
			),
			Err(BuildingHookError::UnexpectedLayout)
		));

		ACTION.set(WrenchHitAction::Refuse);
		assert_eq!(
			hit(harness, &mut Building::of_class(teleporter)),
			(true, vec!["teleporter wrench hit"])
		);
		assert_eq!(SEEN.take(), None);
	});
}
