//! Tests of `crate::hook`: hooks through the mock SourceHook and KHook, which
//! follow the libraries' protocols closely enough to run the hooks as they
//! would.
//!
//! Handlers' panics are caught, so handlers note what went wrong instead.

use super::*;
use crate::api::MetamodVersion;
use crate::test_support::foreign::{FOREIGN_KHOOK, ForeignDelegate, khook_superseding};
use crate::test_support::harness::{DEV_BUILD, Harness, expect};
use std::cell::RefCell;
use std::ptr;

const ADD: VirtualFunction<Add> = VirtualFunction::new(1);
const MIXED: VirtualFunction<Mixed> = VirtualFunction::new(2);
const POKE: VirtualFunction<Poke> = VirtualFunction::new(0);

/// The size of every [`Object`]'s vtable: [`poke`], [`add`], then [`mixed`].
const SLOTS: usize = 3;

thread_local! {
	/// How many times handlers noted a call.
	static HANDLER_CALLS: Cell<u32> = const { Cell::new(0) };

	/// How many times the hooked functions' originals ran.
	static ORIGINAL_CALLS: Cell<u32> = const { Cell::new(0) };

	/// What handlers saw the calls return so far.
	static SEEN_RETURNS: RefCell<Vec<Option<i32>>> = const { RefCell::new(Vec::new()) };

	/// What the last handler to note a call saw of `superseded`.
	static SEEN_SUPERSEDED: Cell<Option<Option<bool>>> = const { Cell::new(None) };

	/// The harness and hook a handler removes as it runs.
	static SELF_REMOVAL: Cell<Option<(*const Harness, HookId)>> = const { Cell::new(None) };
}

type Add = unsafe extern "C" fn(*mut Object, i32) -> i32;

type Mixed =
	unsafe extern "C" fn(*mut Object, i8, f32, u64, f64, *const u8, i16, f32, bool, u32) -> f64;

type Poke = unsafe extern "C" fn(*mut Object);

/// An object of a C++ class, as far as hooks know it.
#[repr(C)]
struct Object {
	vtable: *mut *mut c_void,
	base: i32,
}

impl Object {
	fn new(vtable: *mut *mut c_void, base: i32) -> Box<Self> {
		Box::new(Self { vtable, base })
	}

	fn ptr(&mut self) -> NonNull<Self> {
		NonNull::from(self)
	}
}

/// A pointer to send to another thread.
#[derive(Clone, Copy)]
struct SendPtr<T>(*const T);

// SAFETY: The tests' values outlive the threads they send them to, and only one
// thread uses them at a time.
unsafe impl<T> Send for SendPtr<T> {}

unsafe extern "C" fn add(this: *mut Object, amount: i32) -> i32 {
	ORIGINAL_CALLS.set(ORIGINAL_CALLS.get() + 1);

	// SAFETY: The tests call it on their objects.
	unsafe { (*this).base + amount }
}

#[test]
fn calls_pass_every_kind_of_argument() {
	fn check(call: &HookCall<'_, Mixed>) -> HookAction<f64> {
		note_handler(call.superseded());

		expect(
			call.args()
				== (
					-3,
					1.5,
					1 << 40,
					-2.25,
					64 as *const u8,
					-300,
					0.25,
					true,
					7,
				),
			"the arguments changed",
		);

		match call.timing() {
			HookTiming::Pre => HookAction::Ignore,
			HookTiming::Post => HookAction::Override(call.return_value().unwrap_or(f64::NAN) + 0.5),
		}
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);
		let target = HookTarget::class_of(object.ptr());
		let args = (
			-3,
			1.5,
			1 << 40,
			-2.25,
			64 as *const u8,
			-300,
			0.25,
			true,
			7,
		);

		// SAFETY: The class has `Mixed` at the slot.
		unsafe {
			api.add_hook(MIXED, target, HookTiming::Pre, &check)
				.unwrap();
			api.add_hook(MIXED, target, HookTiming::Post, &check)
				.unwrap();
		}

		let expected =
			5.0 - 3.0 + 1.5 + (1u64 << 40) as f64 - 2.25 + 64.0 - 300.0 + 0.25 + 1.0 + 7.0;

		assert_eq!(
			harness.call::<Mixed>(&raw mut *object, MIXED.index(), args),
			expected + 0.5
		);
		assert_eq!((HANDLER_CALLS.get(), ORIGINAL_CALLS.get()), (2, 1));
	});
}

fn class() -> *mut *mut c_void {
	let mut slots = vec![add as Add as *mut c_void; SLOTS];

	slots[POKE.index()] = poke as Poke as *mut c_void;
	slots[MIXED.index()] = mixed as Mixed as *mut c_void;
	Vec::leak(slots).as_mut_ptr()
}

#[test]
fn classes_sharing_a_hooked_method_keep_their_hooks_apart() {
	fn count(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Override(-1)
	}

	on_both(|harness| {
		let api = harness.api();
		let (first, second) = (class(), class());
		let mut first_object = Object::new(first, 5);
		let mut second_object = Object::new(second, 5);

		// SAFETY: Both classes have `Add` at the slot.
		let [first_hook, second_hook] = unsafe {
			[
				api.add_hook(
					ADD,
					HookTarget::class_of(first_object.ptr()),
					HookTiming::Pre,
					&count,
				)
				.unwrap(),
				api.add_hook(
					ADD,
					HookTarget::class_of(second_object.ptr()),
					HookTiming::Pre,
					&count,
				)
				.unwrap(),
			]
		};

		assert_eq!(library_hooks(harness, first, ADD.index()), 1);
		assert_eq!(library_hooks(harness, second, ADD.index()), 1);

		// One class's method is emptied; the other's keeps its hook and handler.
		assert!(api.remove_hook(first_hook));
		assert_eq!(library_hooks(harness, first, ADD.index()), 0);
		assert_eq!(library_hooks(harness, second, ADD.index()), 1);
		assert_eq!(
			harness.call::<Add>(&raw mut *first_object, ADD.index(), (1,)),
			6
		);
		assert_eq!(
			harness.call::<Add>(&raw mut *second_object, ADD.index(), (1,)),
			-1
		);
		assert_eq!(HANDLER_CALLS.get(), 1);

		match harness.version {
			MetamodVersion::Stable1226 => {
				let state = harness.sourcehook.state.borrow();

				// The classes share one hook manager, which stays registered for the
				// slot it still patches.
				assert_eq!(state.managers.len(), 1);
				assert!(state.managers[0].registered);

				// SAFETY: The classes have the slot.
				unsafe {
					assert_eq!(first.add(ADD.index()).read(), add as Add as *mut c_void);
					assert_ne!(second.add(ADD.index()).read(), add as Add as *mut c_void);
				}
			}

			MetamodVersion::Dev1469 => {
				assert_eq!(harness.khook.state.borrow().removals.len(), 1);
			}
		}

		// The emptied class is hooked again, through the same hook manager.
		// SAFETY: As above.
		let first_hook = unsafe {
			api.add_hook(
				ADD,
				HookTarget::class_of(first_object.ptr()),
				HookTiming::Pre,
				&count,
			)
		}
		.unwrap();

		assert_eq!(
			harness.call::<Add>(&raw mut *first_object, ADD.index(), (1,)),
			-1
		);
		assert_eq!(HANDLER_CALLS.get(), 2);

		// Once neither class is hooked, SourceHook unregisters the hook manager.
		assert!(api.remove_hook(first_hook));
		assert!(api.remove_hook(second_hook));
		assert_eq!(library_hooks(harness, first, ADD.index()), 0);
		assert_eq!(library_hooks(harness, second, ADD.index()), 0);

		if harness.version == MetamodVersion::Stable1226 {
			let state = harness.sourcehook.state.borrow();

			assert_eq!(state.managers.len(), 1);
			assert!(!state.managers[0].registered);
		}
	});
}

#[test]
fn detoured_functions_run_handlers_on_khook_only() {
	fn double_after(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());

		match call.timing() {
			HookTiming::Pre => HookAction::Ignore,
			HookTiming::Post => HookAction::Override(call.return_value().unwrap_or_default() * 2),
		}
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);
		let address = NonNull::new(add as Add as *mut c_void).unwrap();

		// SAFETY: `add` is a function of signature `Add`.
		let function = unsafe { DetouredFunction::<Add>::new(address) };

		match harness.version {
			MetamodVersion::Stable1226 => {
				assert_eq!(
					api.add_function_hook(function, HookTiming::Post, &double_after),
					Err(HookError::DetourUnsupported)
				);
				assert_eq!(
					harness.call_function::<Add>(address, &raw mut *object, (1,)),
					6
				);
			}

			MetamodVersion::Dev1469 => {
				let id = api
					.add_function_hook(function, HookTiming::Post, &double_after)
					.unwrap();

				assert!(api.has_hook(id));
				assert_eq!(
					harness.call_function::<Add>(address, &raw mut *object, (1,)),
					12
				);
				assert_eq!(
					harness.call_function::<Add>(address, &raw mut *object, (3,)),
					16
				);
				assert_eq!((HANDLER_CALLS.get(), ORIGINAL_CALLS.get()), (2, 2));

				assert!(api.remove_hook(id));
				assert_eq!(
					harness.call_function::<Add>(address, &raw mut *object, (1,)),
					6
				);
			}
		}
	});
}

#[test]
fn emptied_slots_lose_the_library_hook_until_a_handler_returns() {
	fn count(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Override(-1)
	}

	on_both(|harness| {
		let api = harness.api();
		let vtable = class();
		let mut object = Object::new(vtable, 5);
		let target = HookTarget::class_of(object.ptr());

		// Hooked, emptied and hooked again, any number of times.
		for round in 1..=3 {
			// SAFETY: The class has `Add` at the slot.
			let hook = unsafe { api.add_hook(ADD, target, HookTiming::Pre, &count) }.unwrap();

			assert_eq!(library_hooks(harness, vtable, ADD.index()), 1);
			assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), -1);
			assert_eq!(HANDLER_CALLS.get(), round);

			assert!(api.remove_hook(hook));
			assert_eq!(library_hooks(harness, vtable, ADD.index()), 0);
			assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
			assert_eq!(HANDLER_CALLS.get(), round);
		}

		match harness.version {
			MetamodVersion::Stable1226 => {
				let state = harness.sourcehook.state.borrow();

				// The slot holds its function again, and one hook manager served
				// every round.
				// SAFETY: The class has the slot.
				assert_eq!(
					unsafe { vtable.add(ADD.index()).read() },
					add as Add as *mut c_void
				);
				assert_eq!(state.managers.len(), 1);
			}

			MetamodVersion::Dev1469 => {
				let state = harness.khook.state.borrow();

				// Removed from KHook's own thread, with nothing to call back, as the
				// library may be gone by the time the removal completes.
				assert_eq!(state.removals.len(), 3);
				assert!(
					state
						.removals
						.iter()
						.all(|&(_, is_async, calls_back)| is_async && !calls_back)
				);
			}
		}
	});
}

#[test]
fn functions_returning_nothing_can_be_superseded() {
	fn supersede(call: &HookCall<'_, Poke>) -> HookAction<()> {
		note_handler(call.superseded());
		expect(
			call.return_value().is_none(),
			"a value was returned before the function ran",
		);

		HookAction::Supersede(())
	}

	fn after(call: &HookCall<'_, Poke>) -> HookAction<()> {
		expect(
			call.return_value() == Some(()),
			"the call returned no value",
		);
		HookAction::Ignore
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);
		let target = HookTarget::class_of(object.ptr());

		// SAFETY: The class has `Poke` at the slot.
		unsafe {
			api.add_hook(POKE, target, HookTiming::Pre, &supersede)
				.unwrap();
			api.add_hook(POKE, target, HookTiming::Post, &after)
				.unwrap();
		}

		harness.call::<Poke>(&raw mut *object, POKE.index(), ());
		assert_eq!((HANDLER_CALLS.get(), ORIGINAL_CALLS.get()), (1, 0));
	});
}

#[test]
fn handlers_decide_calls_before_and_after_the_function() {
	fn supersede_one(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());

		match call.args() {
			(1,) => HookAction::Supersede(10),
			_ => HookAction::Ignore,
		}
	}

	fn override_two(call: &HookCall<'_, Add>) -> HookAction<i32> {
		expect(
			call.superseded().is_none(),
			"a post hook was told whether it was superseded",
		);
		SEEN_RETURNS.with_borrow_mut(|seen| seen.push(call.return_value()));

		match call.args() {
			(2,) => HookAction::Override(call.return_value().unwrap_or_default() + 100),
			_ => HookAction::Handled,
		}
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);
		let target = HookTarget::class_of(object.ptr());

		// SAFETY: The class has `Add` at the slot.
		let pre = unsafe { api.add_hook(ADD, target, HookTiming::Pre, &supersede_one) }.unwrap();

		// SAFETY: As above.
		let post = unsafe { api.add_hook(ADD, target, HookTiming::Post, &override_two) }.unwrap();

		assert!(api.has_hook(pre) && api.has_hook(post));
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 10);
		assert_eq!(ORIGINAL_CALLS.get(), 0);
		assert_eq!(
			harness.call::<Add>(&raw mut *object, ADD.index(), (2,)),
			107
		);
		assert_eq!(ORIGINAL_CALLS.get(), 1);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (3,)), 8);
		assert_eq!(HANDLER_CALLS.get(), 3);

		// SAFETY: As above.
		let original = unsafe { api.original_function(ADD, target) }.unwrap();

		assert_eq!(original as usize, add as Add as usize);
		assert!(api.remove_hook(pre));
		assert!(!api.remove_hook(pre));
		assert!(!api.has_hook(pre) && api.has_hook(post));
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);

		// After the superseding value, the function's own values.
		assert_eq!(SEEN_RETURNS.take(), [Some(10), Some(7), Some(8), Some(6)]);
	});
}

#[test]
fn handlers_ignore_other_threads() {
	fn supersede(_call: &HookCall<'_, Add>) -> HookAction<i32> {
		HookAction::Supersede(0)
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);

		// SAFETY: The class has `Add` at the slot.
		unsafe {
			api.add_hook(
				ADD,
				HookTarget::class_of(object.ptr()),
				HookTiming::Pre,
				&supersede,
			)
			.unwrap();
		}

		let object = SendPtr(ptr::from_mut(&mut *object).cast_const());
		let shared = SendPtr(ptr::from_ref(harness));

		let returned = std::thread::spawn(move || {
			let (object, shared) = (object, shared);

			// SAFETY: The harness outlives the thread, which this thread awaits.
			let harness = unsafe { &*shared.0 };

			FOREIGN_KHOOK.set(harness.khook_ptr());
			harness.call::<Add>(object.0.cast_mut(), ADD.index(), (1,))
		})
		.join()
		.unwrap();

		assert_eq!(returned, 6);
	});
}

#[test]
fn handlers_removing_their_own_hook_leave_the_library_hook_until_later() {
	fn remove_self(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());

		if let Some((harness, hook)) = SELF_REMOVAL.take() {
			// SAFETY: The test keeps the harness for the call.
			let harness = unsafe { &*harness };

			expect(harness.api().remove_hook(hook), "the hook was not removed");
		}

		HookAction::Ignore
	}

	fn other(_call: &HookCall<'_, Poke>) -> HookAction<()> {
		HookAction::Ignore
	}

	on_both(|harness| {
		let api = harness.api();
		let vtable = class();
		let mut object = Object::new(vtable, 5);
		let target = HookTarget::class_of(object.ptr());

		for timing in [HookTiming::Pre, HookTiming::Post] {
			// SAFETY: The class has `Add` at the slot.
			let hook = unsafe { api.add_hook(ADD, target, timing, &remove_self) }.unwrap();

			SELF_REMOVAL.set(Some((ptr::from_ref(harness), hook)));
			assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
			assert!(!api.has_hook(hook));

			// The handler is gone, but the library's hook stays while it ran.
			assert_eq!(library_hooks(harness, vtable, ADD.index()), 1);
			assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);

			// Until the plugin next adds or removes a hook outside the handlers.
			// SAFETY: The class has `Poke` at the slot.
			let poke = unsafe { api.add_hook(POKE, target, HookTiming::Pre, &other) }.unwrap();

			assert_eq!(library_hooks(harness, vtable, ADD.index()), 0);
			assert!(api.remove_hook(poke));
		}

		assert_eq!(HANDLER_CALLS.get(), 2);
		assert_eq!(ORIGINAL_CALLS.get(), 4);
	});
}

#[test]
fn handlers_see_earlier_hooks_only_through_sourcehook() {
	fn look(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Ignore
	}

	on_both(|harness| {
		let api = harness.api();
		let vtable = class();
		let mut object = Object::new(vtable, 5);

		// SAFETY: The class has `Add` at the slot.
		unsafe {
			api.add_hook(
				ADD,
				HookTarget::class_of(object.ptr()),
				HookTiming::Pre,
				&look,
			)
			.unwrap();
		}

		let foreign = ForeignDelegate::new::<i32>(harness.sourcehook_ptr(), MetaRes::SUPERCEDE, 77);
		let superseding = 77;
		let foreign_hook = khook_superseding::<Object, i32, i32>(&superseding);

		match harness.version {
			MetamodVersion::Stable1226 => harness.sourcehook.add_foreign(
				// SAFETY: The class has the slot.
				unsafe { vtable.add(ADD.index()) },
				foreign.ptr(),
			),

			// Another plugin's hook, added after this one's, which KHook runs first,
			// using its `make_return` and `make_call_original` for the call.
			MetamodVersion::Dev1469 => harness.khook.add_foreign(vtable, ADD.index(), foreign_hook),
		}

		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 77);
		assert_eq!(ORIGINAL_CALLS.get(), 0);

		let expected = match harness.version {
			MetamodVersion::Stable1226 => Some(true),
			MetamodVersion::Dev1469 => None,
		};

		assert_eq!(SEEN_SUPERSEDED.get(), Some(expected));

		// Then the other plugin lets calls through, which this plugin's
		// `make_call_original` and `make_return` complete under KHook.
		foreign.result.set(MetaRes::IGNORED);

		for slot in &mut harness.khook.state.borrow_mut().slots {
			slot.hooks
				.retain(|hook| hook.context != foreign_hook.context);
		}

		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
	});
}

#[test]
fn hooks_apply_to_their_targets() {
	fn count(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Override(-1)
	}

	on_both(|harness| {
		let api = harness.api();
		let vtable = class();
		let mut first = Object::new(vtable, 1);
		let mut second = Object::new(vtable, 2);

		// SAFETY: The class has `Add` at the slot.
		unsafe {
			api.add_hook(
				ADD,
				HookTarget::instance(first.ptr()),
				HookTiming::Pre,
				&count,
			)
			.unwrap();
		}

		assert_eq!(harness.call::<Add>(&raw mut *first, ADD.index(), (1,)), -1);
		assert_eq!(harness.call::<Add>(&raw mut *second, ADD.index(), (1,)), 3);
		assert_eq!((HANDLER_CALLS.get(), ORIGINAL_CALLS.get()), (1, 2));

		let vtable = HookTarget::vtable(NonNull::new(vtable).unwrap());

		// SAFETY: As above.
		unsafe { api.add_hook(ADD, vtable, HookTiming::Pre, &count) }.unwrap();

		assert_eq!(harness.call::<Add>(&raw mut *second, ADD.index(), (1,)), -1);
		assert_eq!(HANDLER_CALLS.get(), 2);
	});
}

#[test]
fn inactive_plugins_run_no_handlers() {
	fn supersede(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Supersede(0)
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);
		let target = HookTarget::class_of(object.ptr());

		// SAFETY: The class has `Add` at the slot.
		let hook = unsafe { api.add_hook(ADD, target, HookTiming::Pre, &supersede) }.unwrap();

		harness.set_status(true, true, harness.generation);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);

		harness.set_status(false, false, harness.generation);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
		assert!(!api.has_hook(hook));

		// A later load of the library, which still has the hooks of this one.
		harness.set_status(true, false, harness.generation + 1);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
		assert!(!api.has_hook(hook));

		harness.set_status(true, false, harness.generation);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 0);
		assert_eq!(HANDLER_CALLS.get(), 1);
	});
}

#[test]
fn khook_hooks_awaiting_insertion_are_removed_at_once() {
	fn look(_call: &HookCall<'_, Add>) -> HookAction<i32> {
		HookAction::Ignore
	}

	let harness = Harness::new(MetamodVersion::Dev1469);
	let api = harness.api();
	let vtable = class();
	let object = Object::new(vtable, 5);
	let target = HookTarget::vtable(NonNull::new(object.vtable).unwrap());

	harness.khook.state.borrow_mut().defer_virtual_hooks = true;

	// SAFETY: The class has `Add` at the slot.
	let hook = unsafe { api.add_hook(ADD, target, HookTiming::Pre, &look) }.unwrap();

	assert!(api.remove_hook(hook));

	// KHook took it off its worker's queue before inserting it.
	harness.khook.activate_pending_hooks();
	assert_eq!(library_hooks(&harness, vtable, ADD.index()), 0);
}

#[test]
fn khook_hooks_being_removed_run_no_handlers() {
	fn count(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Override(call.return_value().unwrap_or_default() + 1)
	}

	let harness = Harness::new(MetamodVersion::Dev1469);
	let api = harness.api();
	let vtable = class();
	let mut object = Object::new(vtable, 5);
	let target = HookTarget::class_of(object.ptr());

	HANDLER_CALLS.set(0);
	harness.khook.state.borrow_mut().defer_removals = true;

	// SAFETY: The class has `Add` at the slot.
	let first = unsafe { api.add_hook(ADD, target, HookTiming::Post, &count) }.unwrap();

	assert!(api.remove_hook(first));

	// KHook has yet to remove it, and still calls it, with no handler.
	assert_eq!(library_hooks(&harness, vtable, ADD.index()), 1);
	assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);

	// SAFETY: As above.
	unsafe { api.add_hook(ADD, target, HookTiming::Post, &count) }.unwrap();

	// Both hooks run, but only the newer one runs the handler.
	assert_eq!(library_hooks(&harness, vtable, ADD.index()), 2);
	assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 7);
	assert_eq!(HANDLER_CALLS.get(), 1);

	harness.khook.finish_removals();

	assert_eq!(library_hooks(&harness, vtable, ADD.index()), 1);
	assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 7);
	assert_eq!(HANDLER_CALLS.get(), 2);
}

#[test]
fn khook_hooks_stay_installed_with_their_stack_sizes() {
	fn handler(_call: &HookCall<'_, Mixed>) -> HookAction<f64> {
		HookAction::Ignore
	}

	fn other(_call: &HookCall<'_, Poke>) -> HookAction<()> {
		HookAction::Ignore
	}

	let harness = Harness::new(MetamodVersion::Dev1469);

	// A build that keeps hooks on their slots until the plugin unloads.
	harness.set_metamod_build(Some(1489));

	let api = harness.api();
	let mut object = Object::new(class(), 5);
	let target = HookTarget::class_of(object.ptr());

	// SAFETY: The class has these functions at their slots.
	let hooks = unsafe {
		[
			api.add_hook(MIXED, target, HookTiming::Pre, &handler)
				.unwrap(),
			api.add_hook(MIXED, target, HookTiming::Pre, &handler)
				.unwrap(),
			api.add_hook(POKE, target, HookTiming::Post, &other)
				.unwrap(),
		]
	};

	for hook in hooks {
		assert!(api.remove_hook(hook));
	}

	let state = harness.khook.state.borrow();

	let stack_sizes = state
		.slots
		.iter()
		.flat_map(|slot| slot.hooks.iter().map(|hook| hook.stack_size))
		.collect::<Vec<_>>();

	// For `this`, nine parameters and the returned value, then just `this`.
	let expected = match cfg!(windows) {
		true => [32 + 8 * 7, 32],
		false => [8 + 8 + 9 * 8, 8],
	};

	assert_eq!(stack_sizes, expected);
	assert_eq!(state.removed, 0);
}

#[test]
fn khook_keeps_hooks_on_builds_that_do_not_cope() {
	fn count(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Ignore
	}

	// Just below and just above the build known to cope, and a Metamod whose
	// build the plugin cannot read.
	for build in [Some(DEV_BUILD - 1), Some(DEV_BUILD + 1), None] {
		let harness = Harness::new(MetamodVersion::Dev1469);

		harness.set_metamod_build(build);
		HANDLER_CALLS.set(0);

		let api = harness.api();
		let vtable = class();
		let mut object = Object::new(vtable, 5);
		let target = HookTarget::class_of(object.ptr());

		for _ in 0..2 {
			// SAFETY: The class has `Add` at the slot.
			let hook = unsafe { api.add_hook(ADD, target, HookTiming::Pre, &count) }.unwrap();

			assert!(api.remove_hook(hook));
		}

		// One hook of the library's, never removed, and reused.
		assert_eq!(library_hooks(&harness, vtable, ADD.index()), 1);
		assert_eq!(harness.khook.state.borrow().removed, 0);

		// SAFETY: As above.
		unsafe { api.add_hook(ADD, target, HookTiming::Pre, &count) }.unwrap();

		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
		assert_eq!(library_hooks(&harness, vtable, ADD.index()), 1);
		assert_eq!(HANDLER_CALLS.get(), 1, "with {build:?}");
	}
}

#[test]
fn later_loads_leave_the_hooks_of_earlier_ones_to_metamod() {
	fn count(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Override(-1)
	}

	on_both(|harness| {
		let api = harness.api();
		let vtable = class();
		let mut object = Object::new(vtable, 5);
		let target = HookTarget::class_of(object.ptr());

		// SAFETY: The class has `Add` at the slot.
		let earlier = unsafe { api.add_hook(ADD, target, HookTiming::Pre, &count) }.unwrap();

		// A later load. Metamod 1.12 removed the earlier load's hooks before it.
		// Under KHook, Metamod has yet to remove them, from a library it did not
		// unload.
		harness.load_again();

		let left = match harness.version {
			MetamodVersion::Stable1226 => 0,
			MetamodVersion::Dev1469 => 1,
		};

		assert!(!api.remove_hook(earlier));
		assert_eq!(library_hooks(harness, vtable, ADD.index()), left);

		// SAFETY: As above.
		let later = unsafe { api.add_hook(ADD, target, HookTiming::Pre, &count) }.unwrap();

		assert_eq!(library_hooks(harness, vtable, ADD.index()), left + 1);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), -1);
		assert_eq!(HANDLER_CALLS.get(), 1);

		// Only the later load's own hook leaves.
		assert!(api.remove_hook(later));
		assert_eq!(library_hooks(harness, vtable, ADD.index()), left);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
		assert_eq!(HANDLER_CALLS.get(), 1);
	});
}

/// How many hooks of any plugin's the hooking library has on a vtable slot.
fn library_hooks(harness: &Harness, vtable: *mut *mut c_void, index: usize) -> usize {
	match harness.version {
		// SAFETY: The tests' vtables have the slot.
		MetamodVersion::Stable1226 => harness.sourcehook.hooks_on(unsafe { vtable.add(index) }),

		MetamodVersion::Dev1469 => harness.khook.hook_ids(vtable, index).len(),
	}
}

#[test]
fn metamod_builds_are_read_from_its_version_string() {
	assert_eq!(khook::build_of(c"2.0.0-dev+1490"), Some(1490));
	assert_eq!(khook::build_of(c"2.0.1-dev+1503"), Some(1503));

	// The stable branch, a release without a build, and malformed strings.
	for version in [
		c"1.12.0-dev+1229",
		c"2.0.0",
		c"2.0.0-dev+",
		c"2.0.0-dev+1490V",
		c"2.0.x-dev+1490",
		c"2.0.0-dev+99999999999",
	] {
		assert_eq!(khook::build_of(version), None, "{version:?}");
	}
}

#[allow(
	clippy::too_many_arguments,
	reason = "It stands for a C++ method taking them"
)]
unsafe extern "C" fn mixed(
	this: *mut Object,
	a: i8,
	b: f32,
	c: u64,
	d: f64,
	e: *const u8,
	f: i16,
	g: f32,
	h: bool,
	i: u32,
) -> f64 {
	ORIGINAL_CALLS.set(ORIGINAL_CALLS.get() + 1);

	// SAFETY: The tests call it on their objects.
	let base = unsafe { (*this).base };

	f64::from(base)
		+ f64::from(a)
		+ f64::from(b)
		+ c as f64
		+ d
		+ e as usize as f64
		+ f64::from(f)
		+ f64::from(g)
		+ f64::from(u8::from(h))
		+ f64::from(i)
}

/// Notes a handler's call, with what it saw of `superseded`.
fn note_handler(superseded: Option<bool>) {
	HANDLER_CALLS.set(HANDLER_CALLS.get() + 1);
	SEEN_SUPERSEDED.set(Some(superseded));
}

/// Runs `test` on a Metamod of each version, as
/// [`crate::test_support::harness::on_both`] does, with nothing noted yet.
fn on_both(test: impl Fn(&Harness)) {
	crate::test_support::harness::on_both(|harness| {
		HANDLER_CALLS.set(0);
		ORIGINAL_CALLS.set(0);
		SEEN_RETURNS.take();
		SEEN_SUPERSEDED.set(None);
		test(harness);
	});
}

#[test]
fn other_plugins_keep_their_hooks_on_slots_this_one_leaves() {
	fn count(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Ignore
	}

	on_both(|harness| {
		let api = harness.api();
		let vtable = class();
		let mut object = Object::new(vtable, 5);
		let target = HookTarget::class_of(object.ptr());

		// SAFETY: The class has `Add` at the slot.
		let hook = unsafe { api.add_hook(ADD, target, HookTiming::Pre, &count) }.unwrap();

		// Another plugin's hook on the same slot, superseding every call.
		let foreign = ForeignDelegate::new::<i32>(harness.sourcehook_ptr(), MetaRes::SUPERCEDE, 77);
		let superseding = 77;
		let foreign_hook = khook_superseding::<Object, i32, i32>(&superseding);

		match harness.version {
			MetamodVersion::Stable1226 => harness.sourcehook.add_foreign(
				// SAFETY: The class has the slot.
				unsafe { vtable.add(ADD.index()) },
				foreign.ptr(),
			),

			MetamodVersion::Dev1469 => harness.khook.add_foreign(vtable, ADD.index(), foreign_hook),
		}

		assert_eq!(library_hooks(harness, vtable, ADD.index()), 2);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 77);
		assert_eq!(HANDLER_CALLS.get(), 1);

		// This plugin's hook leaves, and the other plugin's stays and still
		// decides the call.
		assert!(api.remove_hook(hook));
		assert_eq!(library_hooks(harness, vtable, ADD.index()), 1);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 77);
		assert_eq!((HANDLER_CALLS.get(), ORIGINAL_CALLS.get()), (1, 0));

		match harness.version {
			// SourceHook keeps the slot patched for the hook left, through the hook
			// manager that patched it, which stays registered.
			MetamodVersion::Stable1226 => {
				// SAFETY: The class has the slot.
				assert_ne!(
					unsafe { vtable.add(ADD.index()).read() },
					add as Add as *mut c_void
				);
				assert!(harness.sourcehook.state.borrow().managers[0].registered);
			}

			// Only this plugin's own hook was removed.
			MetamodVersion::Dev1469 => {
				let state = harness.khook.state.borrow();

				assert_eq!(state.removals.len(), 1);
				assert_ne!(state.removals[0].0, foreign_hook.id);
			}
		}

		// A handler added again runs beside the other plugin's hook.
		// SAFETY: As above.
		unsafe { api.add_hook(ADD, target, HookTiming::Pre, &count) }.unwrap();

		assert_eq!(library_hooks(harness, vtable, ADD.index()), 2);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 77);
		assert_eq!((HANDLER_CALLS.get(), ORIGINAL_CALLS.get()), (2, 0));
	});
}

#[test]
fn panicking_handlers_are_ignored() {
	fn panic(_call: &HookCall<'_, Add>) -> HookAction<i32> {
		panic!("a handler panicked, as this test means it to");
	}

	on_both(|harness| {
		let api = harness.api();
		let mut object = Object::new(class(), 5);

		// SAFETY: The class has `Add` at the slot.
		unsafe {
			api.add_hook(
				ADD,
				HookTarget::class_of(object.ptr()),
				HookTiming::Pre,
				&panic,
			)
			.unwrap();
		}

		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
	});
}

unsafe extern "C" fn poke(_this: *mut Object) {
	ORIGINAL_CALLS.set(ORIGINAL_CALLS.get() + 1);
}

#[test]
fn slots_keep_the_library_hook_while_a_handler_of_its_timing_is_left() {
	fn look(call: &HookCall<'_, Add>) -> HookAction<i32> {
		note_handler(call.superseded());
		HookAction::Ignore
	}

	on_both(|harness| {
		let api = harness.api();
		let vtable = class();
		let mut object = Object::new(vtable, 5);
		let target = HookTarget::class_of(object.ptr());

		// SAFETY: The class has `Add` at the slot.
		let [first, second, after] = unsafe {
			[
				api.add_hook(ADD, target, HookTiming::Pre, &look).unwrap(),
				api.add_hook(ADD, target, HookTiming::Pre, &look).unwrap(),
				api.add_hook(ADD, target, HookTiming::Post, &look).unwrap(),
			]
		};

		assert_eq!(library_hooks(harness, vtable, ADD.index()), 2);

		assert!(api.remove_hook(first));
		assert_eq!(library_hooks(harness, vtable, ADD.index()), 2);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
		assert_eq!(HANDLER_CALLS.get(), 2);

		// The hook before the function leaves; the one after stays.
		assert!(api.remove_hook(second));
		assert_eq!(library_hooks(harness, vtable, ADD.index()), 1);
		assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 6);
		assert_eq!(HANDLER_CALLS.get(), 3);

		assert!(api.remove_hook(after));
		assert_eq!(library_hooks(harness, vtable, ADD.index()), 0);
		assert_eq!(ORIGINAL_CALLS.get(), 2);
	});
}

#[test]
fn sourcehook_hook_managers_describe_their_functions() {
	fn handler(_call: &HookCall<'_, Mixed>) -> HookAction<f64> {
		HookAction::Ignore
	}

	let harness = Harness::new(MetamodVersion::Stable1226);
	let api = harness.api();
	let mut object = Object::new(class(), 5);

	// SAFETY: The class has `Mixed` at the slot.
	unsafe {
		api.add_hook(
			MIXED,
			HookTarget::class_of(object.ptr()),
			HookTiming::Post,
			&handler,
		)
		.unwrap();
	}

	let state = harness.sourcehook.state.borrow();
	let info = &state.managers[0].info;

	assert_eq!(info.index.get(), MIXED.index() as c_int);

	// SAFETY: Hook managers' prototypes are leaked.
	let proto = unsafe { &*info.proto.get() };

	// SAFETY: As above, with an entry before the parameters'.
	let parameters = unsafe { std::slice::from_raw_parts(proto.params_pass_info, 10) };

	assert_eq!(proto.num_of_params, 9);
	assert_eq!(proto.ret_pass_info.size, size_of::<f64>());
	assert_eq!(parameters[0].size, 1);

	assert_eq!(
		parameters[1..]
			.iter()
			.map(|parameter| parameter.size)
			.collect::<Vec<_>>(),
		[1, 4, 8, 8, 8, 2, 4, 1, 4]
	);

	assert!(state.hooks[0].post);
}

#[test]
fn sourcehook_keeps_late_player_hooks_after_many_functions() {
	fn handler(call: &HookCall<'_, Add>) -> HookAction<i32> {
		expect(
			call.return_value() == Some(5 + call.args().0),
			"the original returned another slot's value",
		);
		HookAction::Override(1000 + call.args().0)
	}

	let harness = Harness::new(MetamodVersion::Stable1226);
	let api = harness.api();
	let count = 96;
	let table = Vec::leak(vec![add as Add as *mut c_void; count]).as_mut_ptr();
	let mut object = Object::new(table, 5);
	let target = HookTarget::class_of(object.ptr());
	let mut hooks = Vec::new();

	for index in 0..count {
		// SAFETY: Every slot of this class holds Add.
		let hook = unsafe {
			api.add_hook(
				VirtualFunction::<Add>::new(index),
				target,
				HookTiming::Post,
				&handler,
			)
		}
		.expect("a late hook could not be registered");

		hooks.push(hook);
	}

	for index in 0..count {
		assert_eq!(
			harness.call::<Add>(&raw mut *object, index, (index as i32,)),
			1000 + index as i32
		);
	}

	for hook in hooks {
		assert!(api.remove_hook(hook));
	}

	for index in 0..count {
		assert_eq!(
			harness.call::<Add>(&raw mut *object, index, (index as i32,)),
			5 + index as i32
		);
	}
}

#[test]
fn sourcehook_loops_stay_on_the_main_thread() {
	fn supersede(_call: &HookCall<'_, Add>) -> HookAction<i32> {
		HookAction::Supersede(0)
	}

	let harness = Harness::new(MetamodVersion::Stable1226);
	let api = harness.api();
	let mut object = Object::new(class(), 5);

	// SAFETY: The class has `Add` at the slot.
	unsafe {
		api.add_hook(
			ADD,
			HookTarget::class_of(object.ptr()),
			HookTiming::Pre,
			&supersede,
		)
		.unwrap();
	}

	let pointer = SendPtr(ptr::from_mut(&mut *object).cast_const());
	let shared = SendPtr(ptr::from_ref(&harness));

	let returned = std::thread::spawn(move || {
		let (pointer, shared) = (pointer, shared);

		// SAFETY: The harness outlives the thread, which this thread awaits.
		let harness = unsafe { &*shared.0 };

		harness.call::<Add>(pointer.0.cast_mut(), ADD.index(), (1,))
	})
	.join()
	.unwrap();

	assert_eq!(returned, 6);
	assert_eq!(harness.sourcehook.state.borrow().loops, 0);
	assert_eq!(harness.call::<Add>(&raw mut *object, ADD.index(), (1,)), 0);
	assert_eq!(harness.sourcehook.state.borrow().loops, 1);
}

#[test]
fn the_detours_of_functions_are_removed_as_the_plugin_unloads() {
	fn double_after(call: &HookCall<'_, Add>) -> HookAction<i32> {
		match call.timing() {
			HookTiming::Pre => HookAction::Ignore,
			HookTiming::Post => HookAction::Override(call.return_value().unwrap_or_default() * 2),
		}
	}

	on_both(|harness| {
		let api = harness.api();
		let address = NonNull::new(add as Add as *mut c_void).unwrap();

		// SAFETY: `add` is a function of signature `Add`.
		let function = unsafe { DetouredFunction::<Add>::new(address) };

		// Nothing is installed yet, or ever, on SourceHook.
		api.remove_function_hooks();
		assert_eq!(harness.khook.state.borrow().removed, 0);

		if harness.version != MetamodVersion::Dev1469 {
			return;
		}

		let first = api
			.add_function_hook(function, HookTiming::Post, &double_after)
			.unwrap();

		api.remove_function_hooks();
		assert!(!api.has_hook(first));
		assert_eq!(harness.khook.state.borrow().removed, 1);

		// The plugin can hook the function again, which detours it anew.
		let second = api
			.add_function_hook(function, HookTiming::Post, &double_after)
			.unwrap();

		assert!(api.has_hook(second));

		api.remove_function_hooks();
		assert_eq!(harness.khook.state.borrow().removed, 2);
	});
}
