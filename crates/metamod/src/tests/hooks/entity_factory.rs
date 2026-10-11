//! Tests of `crate::hooks::entity_factory`: pre hooks of `Create` on mock
//! entity factories, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use std::cell::RefCell;
use std::ffi::{CString, c_char, c_void};

thread_local! {
	/// What ran during the calls since the last [`create`], in order, with
	/// the class name each ran for.
	static CALLS: RefCell<Vec<(&'static str, CString)>> = const { RefCell::new(Vec::new()) };
}

/// An entity factory of a C++ class, as far as hooks know it.
#[repr(C)]
struct Factory {
	vtable: *mut *mut c_void,
}

impl Factory {
	/// A factory of a new class, whose vtable holds `create` at
	/// [`CREATE_SLOT`].
	fn of_new_class(create: CreateFn) -> Box<Self> {
		let slots = Vec::leak(vec![create as *mut c_void; CREATE_SLOT + 1]);

		Self::with_vtable(slots.as_mut_ptr())
	}

	fn with_vtable(vtable: *mut *mut c_void) -> Box<Self> {
		Box::new(Self { vtable })
	}

	/// A factory sharing `self`'s vtable, as the factories of two class names
	/// linked to one entity class do.
	fn of_same_class(&self) -> Box<Self> {
		Self::with_vtable(self.vtable)
	}

	fn ptr(&mut self) -> NonNull<sys::IEntityFactory> {
		NonNull::from(self).cast()
	}
}

/// Creates an entity of `class_name` through `factory`'s hooked `Create`, and
/// returns what it returned and what ran.
fn create(
	harness: &Harness,
	factory: &mut Factory,
	class_name: &CStr,
) -> (
	*mut sys::IServerNetworkable,
	Vec<(&'static str, &'static str)>,
) {
	CALLS.take();

	let created =
		harness.call::<CreateFn>(factory.ptr().as_ptr(), CREATE_SLOT, (class_name.as_ptr(),));
	let calls = CALLS
		.take()
		.into_iter()
		.map(|(what, class_name)| (what, &*class_name.into_string().unwrap().leak()))
		.collect();

	(created, calls)
}

#[test]
fn creations_run_unless_a_hook_refuses_them() {
	on_both(|harness| {
		let api = harness.api();
		let mut souls = Factory::of_new_class(game_create);
		let mut alias = souls.of_same_class();
		let mut gift = Factory::of_new_class(game_create);
		let mut blocked = Factory::of_new_class(game_create);
		let souls_entity = souls.ptr().as_ptr().cast();
		let gift_entity = gift.ptr().as_ptr().cast();
		let alias_entity = alias.ptr().as_ptr().cast();

		for object in [souls.ptr(), gift.ptr()] {
			// SAFETY: The mock classes have `Create` at the slot, are leaked,
			// and their callers here cope with null.
			unsafe { api.install_factory(object, tf2_binding(no_interfaces), on_create) }.unwrap();
		}

		// A second hook of a factory is refused, so that each creation is
		// decided once.
		assert_eq!(
			// SAFETY: As above.
			unsafe { api.install_factory(souls.ptr(), tf2_binding(no_interfaces), on_create) },
			Err(HookError::AlreadyInstalled)
		);

		assert_eq!(
			create(harness, &mut souls, c"souls_pack"),
			(ptr::null_mut(), vec![("callback", "souls_pack")])
		);
		assert_eq!(
			create(harness, &mut gift, c"gift"),
			(gift_entity, vec![("callback", "gift"), ("game", "gift")])
		);

		// Another factory of the hooked class is not hooked.
		assert_eq!(
			create(harness, &mut alias, c"souls_pack"),
			(alias_entity, vec![("game", "souls_pack")])
		);

		// A removed hook lets the factory create again, and can be replaced.
		let hook = ROUTES
			.iter()
			.find_map(|route| {
				route
					.state
					.get()
					.filter(|state| state.factory == souls.ptr().addr().get())
			})
			.unwrap()
			.hook;

		assert!(api.remove_hook(hook));
		assert_eq!(
			create(harness, &mut souls, c"souls_pack"),
			(souls_entity, vec![("game", "souls_pack")])
		);

		// SAFETY: As above.
		unsafe { api.install_factory(souls.ptr(), tf2_binding(no_interfaces), on_create) }.unwrap();
		assert_eq!(
			create(harness, &mut souls, c"souls_pack"),
			(ptr::null_mut(), vec![("callback", "souls_pack")])
		);

		// A creation an earlier hook superseded reaches neither the callback
		// nor the game.
		fn supersede(_call: &HookCall<'_, CreateFn>) -> HookAction<*mut sys::IServerNetworkable> {
			HookAction::Supersede(ptr::dangling_mut())
		}

		// SAFETY: As above.
		unsafe {
			api.add_hook(
				CREATE,
				HookTarget::instance(blocked.ptr()),
				HookTiming::Pre,
				&supersede,
			)
			.unwrap();
			api.install_factory(blocked.ptr(), tf2_binding(no_interfaces), on_create)
				.unwrap();
		}

		assert_eq!(
			create(harness, &mut blocked, c"souls_pack"),
			(ptr::dangling_mut(), vec![])
		);
	});
}

/// The game's `Create`, which notes that it ran, and returns the factory as
/// the entity it created.
unsafe extern "C" fn game_create(
	this: *mut sys::IEntityFactory,
	class_name: *const c_char,
) -> *mut sys::IServerNetworkable {
	// SAFETY: The tests pass NUL-terminated class names.
	let class_name = unsafe { CStr::from_ptr(class_name) }.to_owned();

	CALLS.with_borrow_mut(|calls| calls.push(("game", class_name)));
	this.cast()
}

/// The callback, which notes the class name, and refuses `souls_pack`.
fn on_create(_server: Server<'_>, class_name: &CStr) -> CreateAction {
	CALLS.with_borrow_mut(|calls| calls.push(("callback", class_name.to_owned())));

	match class_name == c"souls_pack" {
		true => CreateAction::Refuse,
		false => CreateAction::Allow,
	}
}
