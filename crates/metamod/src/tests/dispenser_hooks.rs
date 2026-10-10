//! Dispenser supply ABI and lifecycle fixtures through SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, expect, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::{InterfaceFactory, sys};
use std::cell::RefCell;
use std::ptr;

thread_local! {
	static ACTION: Cell<DispenserAmmoAction> = const { Cell::new(DispenserAmmoAction::Continue) };
	static CALLS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
	static SEEN: Cell<Option<(BuildingClass, usize, usize)>> = const { Cell::new(None) };
}

type SetupVirtualHook = unsafe extern "C" fn(
	*mut crate::sys::khook::IKHook,
	*mut *mut c_void,
	std::ffi::c_int,
	*mut c_void,
	*mut c_void,
	*mut c_void,
	*mut c_void,
	*mut c_void,
	*mut c_void,
	std::ffi::c_uint,
	bool,
) -> crate::sys::khook::HookId;

const PLAYER: usize = 0x9a7;

#[repr(C)]
struct Dispenser {
	vtable: *mut *mut c_void,
}

impl Dispenser {
	fn new_class(function: DispenseAmmo) -> NonNull<*mut c_void> {
		let slots = vec![function as *mut c_void; DISPENSE_AMMO_SLOT + 1];
		NonNull::new(Vec::leak(slots).as_mut_ptr()).unwrap()
	}

	fn of_class(vtable: NonNull<*mut c_void>) -> Self {
		Self {
			vtable: vtable.as_ptr(),
		}
	}

	fn ptr(&mut self) -> *mut sys::CBaseEntity {
		(self as *mut Self).cast()
	}
}

#[test]
fn aliased_class_tables_are_rejected_before_registration() {
	on_both(|harness| {
		let mut vtables = classes();
		vtables[3] = vtables[1];
		assert!(matches!(
			install(harness, vtables),
			Err(DispenserHookError::UnexpectedLayout)
		));
		assert!(!dispenser_hooked(harness.api()));
		let _hooks = install(harness, classes()).unwrap();
	});
}

fn classes() -> [NonNull<*mut c_void>; 4] {
	[native_supply as DispenseAmmo; 4].map(Dispenser::new_class)
}

#[test]
fn earlier_supersede_keeps_its_result_and_skips_callback() {
	fn skip(_call: &HookCall<'_, DispenseAmmo>) -> HookAction<bool> {
		HookAction::Supersede(false)
	}
	on_both(|harness| {
		let vtables = classes();
		// SAFETY: As for install; the prior hook has the same typed contract.
		unsafe {
			harness.api().add_hook(
				DISPENSE_AMMO,
				HookTarget::vtable(vtables[1]),
				HookTiming::Pre,
				&skip,
			)
		}
		.unwrap();
		let _hooks = install(harness, vtables).unwrap();
		let mut dispenser = Dispenser::of_class(vtables[1]);
		ACTION.set(DispenserAmmoAction::Supply(true));
		assert_eq!(
			supply(harness, &mut dispenser, ptr::without_provenance_mut(PLAYER)),
			(false, vec![])
		);
		assert_eq!(SEEN.take(), None);
	});
}

#[test]
fn generated_slot_matches_both_platform_contracts() {
	#[cfg(target_os = "windows")]
	assert_eq!(DISPENSE_AMMO_SLOT, 411);
	#[cfg(target_os = "linux")]
	assert_eq!(DISPENSE_AMMO_SLOT, 425);
	assert_eq!(DISPENSER_CLASSES.len(), 4);
	assert!(
		DISPENSER_CLASSES
			.iter()
			.all(|class| class.kind() == source_sdk_2013::tf2::buildings::BuildingKind::Dispenser)
	);
}

fn install(
	harness: &Harness,
	vtables: [NonNull<*mut c_void>; 4],
) -> Result<DispenserHooks, DispenserHookError> {
	// SAFETY: Every leaked mock table has the exact native signature at the
	// slot, and mock objects belong to its corresponding dispenser class.
	unsafe {
		harness
			.api()
			.install_dispenser_ammo(vtables, tf2_binding(no_interfaces), on_supply)
	}
}

#[test]
fn missing_classes_fail_without_live_entities() {
	on_both(|harness| {
		let binding = tf2_binding(no_interfaces);
		let scope = ();
		// SAFETY: The test server module is the executable containing
		// no_interfaces, and contains none of TF2's dispenser classes.
		let server = unsafe { binding.server(&scope) };
		let targets = ClassTargets::load(server).unwrap();
		assert!(matches!(
			unsafe {
				harness
					.api()
					.hook_dispenser_ammo(&targets, binding, on_supply)
			},
			Err(DispenserHookError::Target(BuildingVtableError::NotFound(
				BuildingClass::CartDispenser
			)))
		));
		// SAFETY: A checked non-TF2 binding reaches no engine interface here.
		let other = unsafe {
			ServerBinding::new(
				InterfaceFactory::new(no_interfaces),
				InterfaceFactory::new(no_interfaces),
				Game::SourceSdk2013,
			)
		};
		assert!(matches!(
			unsafe {
				harness
					.api()
					.hook_dispenser_ammo(&targets, other, on_supply)
			},
			Err(DispenserHookError::Target(BuildingVtableError::WrongGame))
		));
		let _hooks = install(harness, classes()).unwrap();
		assert!(matches!(
			unsafe {
				harness
					.api()
					.hook_dispenser_ammo(&targets, binding, on_supply)
			},
			Err(DispenserHookError::Hook(HookError::AlreadyInstalled))
		));
	});
}

unsafe extern "C" fn native_supply(
	_this: *mut sys::CBaseEntity,
	player: *mut sys::CBaseEntity,
) -> bool {
	expect(
		player.is_null() || player.addr() == PLAYER,
		"native supply received another recipient",
	);
	CALLS.with_borrow_mut(|calls| calls.push("stock supply"));
	!player.is_null()
}

#[test]
fn null_recipient_does_not_enter_callback() {
	on_both(|harness| {
		let vtables = classes();
		let _hooks = install(harness, vtables).unwrap();
		let mut dispenser = Dispenser::of_class(vtables[1]);
		ACTION.set(DispenserAmmoAction::Supply(true));
		assert_eq!(
			supply(harness, &mut dispenser, ptr::null_mut()),
			(false, vec!["stock supply"])
		);
		assert_eq!(SEEN.take(), None);
	});
}

fn on_supply(_server: Server<'_>, event: DispenserAmmoEvent<'_>) -> DispenserAmmoAction {
	expect(
		event.player.as_ptr().addr() == PLAYER,
		"callback received another recipient",
	);
	CALLS.with_borrow_mut(|calls| calls.push("on supply"));
	SEEN.set(Some((
		event.class,
		event.building.as_ptr().addr(),
		event.player.as_ptr().addr(),
	)));
	ACTION.get()
}

#[test]
fn pause_unload_and_later_generation_disable_old_routes() {
	on_both(|harness| {
		let api = harness.api();
		let vtables = classes();
		let hooks = install(harness, vtables).unwrap();
		let mut dispenser = Dispenser::of_class(vtables[1]);
		ACTION.set(DispenserAmmoAction::Supply(false));
		harness.set_status(true, true, harness.generation);
		assert_eq!(
			supply(harness, &mut dispenser, ptr::without_provenance_mut(PLAYER)),
			(true, vec!["stock supply"])
		);
		harness.set_status(false, false, harness.generation);
		assert_eq!(
			supply(harness, &mut dispenser, ptr::without_provenance_mut(PLAYER)),
			(true, vec!["stock supply"])
		);
		harness.set_status(true, false, harness.generation | 1 << 63);
		assert_eq!(
			supply(harness, &mut dispenser, ptr::without_provenance_mut(PLAYER)),
			(true, vec!["stock supply"])
		);
		assert!(!hooks.hooks.iter().any(|&hook| api.has_hook(hook)));
		let _new = install(harness, vtables).unwrap();
		hooks.remove(api);
		assert_eq!(
			supply(harness, &mut dispenser, ptr::without_provenance_mut(PLAYER)),
			(false, vec!["on supply"])
		);
	});
}

#[test]
fn removal_and_reinstallation_do_not_leave_callbacks() {
	on_both(|harness| {
		let api = harness.api();
		let vtables = classes();
		let hooks = install(harness, vtables).unwrap();
		assert!(matches!(
			install(harness, vtables),
			Err(DispenserHookError::Hook(HookError::AlreadyInstalled))
		));
		let mut dispenser = Dispenser::of_class(vtables[1]);
		hooks.remove(api);
		ACTION.set(DispenserAmmoAction::Supply(false));
		assert_eq!(
			supply(harness, &mut dispenser, ptr::without_provenance_mut(PLAYER)),
			(true, vec!["stock supply"])
		);
		let _hooks = install(harness, vtables).unwrap();
		assert_eq!(
			supply(harness, &mut dispenser, ptr::without_provenance_mut(PLAYER)),
			(false, vec!["on supply"])
		);
	});
}

#[test]
fn replacement_preserves_supplied_and_empty_results_without_native_grants() {
	on_both(|harness| {
		let vtables = classes();
		let _hooks = install(harness, vtables).unwrap();
		let mut dispenser = Dispenser::of_class(vtables[1]);
		for supplied in [false, true] {
			ACTION.set(DispenserAmmoAction::Supply(supplied));
			assert_eq!(
				supply(harness, &mut dispenser, ptr::without_provenance_mut(PLAYER)),
				(supplied, vec!["on supply"])
			);
		}
		ACTION.set(DispenserAmmoAction::Continue);
		assert_eq!(
			supply(harness, &mut dispenser, ptr::without_provenance_mut(PLAYER)),
			(true, vec!["on supply", "stock supply"])
		);
	});
}

#[test]
fn shared_native_method_routes_each_dispenser_class_once() {
	on_both(|harness| {
		let vtables = classes();
		let _hooks = install(harness, vtables).unwrap();
		ACTION.set(DispenserAmmoAction::Continue);
		for (class, vtable) in DISPENSER_CLASSES.into_iter().zip(vtables) {
			let mut dispenser = Dispenser::of_class(vtable);
			let address = dispenser.ptr().addr();
			assert_eq!(
				supply(harness, &mut dispenser, ptr::without_provenance_mut(PLAYER)),
				(true, vec!["on supply", "stock supply"])
			);
			assert_eq!(SEEN.take(), Some((class, address, PLAYER)));
		}
	});
}

fn supply(
	harness: &Harness,
	dispenser: &mut Dispenser,
	player: *mut sys::CBaseEntity,
) -> (bool, Vec<&'static str>) {
	CALLS.take();
	SEEN.take();
	let result = harness.call::<DispenseAmmo>(dispenser.ptr(), DISPENSE_AMMO_SLOT, (player,));
	(result, CALLS.take())
}

unsafe extern "C" fn unexpected_supply(
	_this: *mut sys::CBaseEntity,
	_player: *mut sys::CBaseEntity,
) -> bool {
	CALLS.with_borrow_mut(|calls| calls.push("unexpected supply"));
	false
}

#[test]
fn wrong_class_layout_installs_nothing_and_allows_retry() {
	on_both(|harness| {
		let mut vtables = classes();
		vtables[2] = Dispenser::new_class(unexpected_supply);
		assert!(matches!(
			install(harness, vtables),
			Err(DispenserHookError::UnexpectedLayout)
		));
		let mut dispenser = Dispenser::of_class(vtables[0]);
		assert_eq!(
			supply(harness, &mut dispenser, ptr::without_provenance_mut(PLAYER)),
			(true, vec!["stock supply"])
		);
		assert!(!dispenser_hooked(harness.api()));
		let _hooks = install(harness, classes()).unwrap();
	});
}

thread_local! {
	static ORIGINAL_SETUP: Cell<Option<SetupVirtualHook>> = const { Cell::new(None) };
	static SETUP_CALLS: Cell<usize> = const { Cell::new(0) };
}

#[test]
fn native_partial_refusal_rolls_back_routes_before_retry() {
	let harness = Harness::new(crate::MetamodVersion::Dev1469);
	let vtables = classes();
	let native = harness.khook_ptr();
	// SAFETY: Harness owns this live mock object and its static vtable.
	let original = unsafe { (*native).vtable };
	// SAFETY: A vtable contains only function pointers and has no owned data.
	let mut refused = unsafe { ptr::read(original) };
	ORIGINAL_SETUP.set(Some(refused.setup_virtual_hook));
	SETUP_CALLS.set(0);
	refused.setup_virtual_hook = refuse_second_registration;
	let refused = Box::leak(Box::new(refused));
	// SAFETY: The replacement preserves every native slot and is leaked.
	unsafe {
		(*native).vtable = refused;
	}
	let failed = install(&harness, vtables);
	// SAFETY: Restore the original static table before further operations.
	unsafe {
		(*native).vtable = original;
	}
	assert!(matches!(failed, Err(DispenserHookError::Hook(_))));
	assert_eq!(SETUP_CALLS.get(), 2);
	assert!(!dispenser_hooked(harness.api()));
	ACTION.set(DispenserAmmoAction::Supply(false));
	for vtable in vtables {
		let mut dispenser = Dispenser::of_class(vtable);
		assert_eq!(
			supply(
				&harness,
				&mut dispenser,
				ptr::without_provenance_mut(PLAYER)
			),
			(true, vec!["stock supply"])
		);
	}
	let _hooks = install(&harness, vtables).unwrap();
	for vtable in vtables {
		let mut dispenser = Dispenser::of_class(vtable);
		assert_eq!(
			supply(
				&harness,
				&mut dispenser,
				ptr::without_provenance_mut(PLAYER)
			),
			(false, vec!["on supply"])
		);
	}
}

/// A native backend refusing the second class after accepting the first.
unsafe extern "C" fn refuse_second_registration(
	this: *mut crate::sys::khook::IKHook,
	vtable: *mut *mut c_void,
	index: std::ffi::c_int,
	context: *mut c_void,
	removed: *mut c_void,
	pre: *mut c_void,
	post: *mut c_void,
	make_return: *mut c_void,
	call_original: *mut c_void,
	stack_size: std::ffi::c_uint,
	asynchronous: bool,
) -> crate::sys::khook::HookId {
	let number = SETUP_CALLS.replace(SETUP_CALLS.get() + 1);
	if number == 1 {
		return crate::sys::khook::INVALID_HOOK;
	}
	let original = ORIGINAL_SETUP.get().unwrap();
	// SAFETY: Forward the mock's unchanged native registration contract.
	unsafe {
		original(
			this,
			vtable,
			index,
			context,
			removed,
			pre,
			post,
			make_return,
			call_original,
			stack_size,
			asynchronous,
		)
	}
}
