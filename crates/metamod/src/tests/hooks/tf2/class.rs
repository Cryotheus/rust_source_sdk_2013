//! Tests of `crate::hooks::tf2::class`: covering mock entity classes one by one,
//! through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::raw::tf2::virtuals::EntityFn;
use source_sdk_2013::tf2::class_targets::BaseEntity;
use std::ffi::c_int;
use std::ptr;

thread_local! {
	/// What ran during the calls since the last [`run`], in order, with the
	/// entity it ran for.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };
}

/// The callback of the tests' hooks.
type Callback = for<'s> fn(Server<'s>, HookTiming, Entity<'s>);

/// The hooked method of the mock classes.
const FUNCTION: VirtualFunction<EntityFn> = VirtualFunction::new(SLOT);

/// The slot of the hooked method.
const SLOT: usize = 3;

/// An entity of a C++ class, as far as hooks know it.
#[repr(C)]
struct Mock {
	vtable: *mut *mut c_void,
}

impl Mock {
	/// An entity of a new class, whose vtable holds `method` up to [`SLOT`].
	fn of_new_class(method: EntityFn) -> Box<Self> {
		let slots = Vec::leak(vec![method as *mut c_void; SLOT + 1]);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn address(&self) -> usize {
		ptr::from_ref(self).addr()
	}

	fn ptr(&mut self) -> *mut sys::CBaseEntity {
		ptr::from_mut(self).cast()
	}

	/// The mock's class, as an entity class.
	fn target(&self) -> ClassTarget<'static, BaseEntity> {
		// SAFETY: The tests' hooks only call the method at the slot, which the
		// leaked vtable holds.
		unsafe { ClassTarget::from_raw(NonNull::new(self.vtable).unwrap()) }
	}
}

#[test]
fn a_class_hooked_with_another_signature_stays_uncovered() {
	/// The method with another signature.
	type Other = unsafe extern "C" fn(*mut sys::CBaseEntity, c_int);

	fn ignore(_: &HookCall<'_, Other>) -> HookAction<()> {
		HookAction::Ignore
	}

	on_both(|harness| {
		let api = harness.api();
		let hooks = hooks();
		let entity = Mock::of_new_class(game_method);

		// SAFETY: The hook never runs, as the test never calls the slot.
		unsafe {
			api.add_hook(
				VirtualFunction::<Other>::new(SLOT),
				HookTarget::vtable(NonNull::new(entity.vtable).unwrap()),
				HookTiming::Pre,
				&ignore,
			)
		}
		.unwrap();

		assert_eq!(
			hooks.cover(api, entity.target()),
			Err(HookError::SignatureMismatch)
		);
		assert!(hooks.is_empty());
	});
}

#[test]
fn covered_classes_are_found_by_their_entities() {
	on_both(|harness| {
		let api = harness.api();
		let hooks = hooks();
		let mut covered = Mock::of_new_class(game_method);
		let mut other = Mock::of_new_class(game_method);
		let scope = ();
		// SAFETY: The test reaches no interface through the server.
		let server = unsafe { tf2_binding(no_interfaces).server(&scope) };
		// SAFETY: The mocks outlive the server's scope, and only their vtables
		// are read.
		let (covered_entity, other_entity) = unsafe {
			(
				Entity::from_live(server, NonNull::new(covered.ptr()).unwrap()),
				Entity::from_live(server, NonNull::new(other.ptr()).unwrap()),
			)
		};

		hooks.cover(api, covered.target()).unwrap();
		assert!(hooks.covers(covered_entity));
		assert!(!hooks.covers(other_entity));

		// A covered entity's class is found by its vtable, without its data
		// description maps, which the mock has none of.
		assert_eq!(hooks.cover_entity(api, server, covered_entity), Ok(false));
	});
}

#[test]
fn covered_classes_run_the_callback_around_the_method() {
	on_both(|harness| {
		let api = harness.api();
		let hooks = hooks();
		let mut first = Mock::of_new_class(game_method);
		let mut second = Mock::of_new_class(game_method);
		let (a, b) = (first.address(), second.address());

		assert!(hooks.is_empty());
		assert_eq!(run(harness, &mut first), [("game", a)]);

		// A class is covered once.
		assert_eq!(hooks.cover(api, first.target()), Ok(true));
		assert_eq!(hooks.cover(api, first.target()), Ok(false));
		assert_eq!(hooks.len(), 1);
		assert_eq!(
			run(harness, &mut first),
			[("before", a), ("game", a), ("after", a)]
		);

		// Another class only runs the callback once it is covered too.
		assert_eq!(run(harness, &mut second), [("game", b)]);
		assert_eq!(hooks.cover(api, second.target()), Ok(true));
		assert_eq!(
			run(harness, &mut second),
			[("before", b), ("game", b), ("after", b)]
		);

		hooks.remove(api);
		assert_eq!(run(harness, &mut first), [("game", a)]);
		assert_eq!(run(harness, &mut second), [("game", b)]);
	});
}

/// Tells the callback of the call, which goes on.
fn dispatch<'s>(
	server: Server<'s>,
	callback: Callback,
	entity: Entity<'s>,
	call: &HookCall<'_, EntityFn>,
) -> HookAction<()> {
	callback(server, call.timing(), entity);
	HookAction::Ignore
}

/// The game's method, which notes that it ran.
unsafe extern "C" fn game_method(this: *mut sys::CBaseEntity) {
	note("game", this.addr());
}

/// Hooks [`FUNCTION`] around the method with [`on_call`], on no class yet.
fn hooks() -> ClassHooks<BaseEntity> {
	ClassHooks::new(
		tf2_binding(no_interfaces),
		on_call as Callback,
		FUNCTION,
		&[HookTiming::Post, HookTiming::Pre],
		dispatch,
	)
}

fn note(name: &'static str, entity: usize) {
	CALLS.with_borrow_mut(|calls| calls.push((name, entity)));
}

/// The callback, which notes the timing.
fn on_call(_server: Server<'_>, timing: HookTiming, entity: Entity<'_>) {
	note(
		match timing {
			HookTiming::Pre => "before",
			HookTiming::Post => "after",
		},
		entity.as_ptr().addr(),
	);
}

/// Calls `entity`'s method, as the engine would, and returns what ran.
fn run(harness: &Harness, entity: &mut Mock) -> Vec<(&'static str, usize)> {
	CALLS.take();
	harness.call::<EntityFn>(entity.ptr(), SLOT, ());
	CALLS.take()
}
