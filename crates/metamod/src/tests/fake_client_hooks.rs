//! Tests of `crate::fake_client_hooks`: pre hooks of `CreateFakeClient` and
//! `CreateFakeClientEx` on a mock engine, through the mock SourceHook and
//! KHook.

use super::*;
use crate::test_support::harness::on_both;
use std::cell::RefCell;
use std::ffi::{CString, c_void};

thread_local! {
	/// The engine's functions that ran since the last [`take`], with the name
	/// and whether the client is reported.
	static CALLS: RefCell<Vec<(&'static str, CString, bool)>> = const { RefCell::new(Vec::new()) };
}

/// What the mock engine's functions return, as the edict they created.
const EDICT: *mut sys::edict_t = 0x1000 as *mut sys::edict_t;

/// The engine's interface object, as far as hooks know it.
#[repr(C)]
struct Engine {
	vtable: *mut *mut c_void,
}

impl Engine {
	fn new() -> Box<Self> {
		let slots = Vec::leak(vec![std::ptr::null_mut(); CREATE_FAKE_CLIENT_EX_SLOT + 1]);

		slots[CREATE_FAKE_CLIENT_SLOT] = engine_create as CreateFakeClient as *mut c_void;
		slots[CREATE_FAKE_CLIENT_EX_SLOT] = engine_create_ex as CreateFakeClientEx as *mut c_void;

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<sys::IVEngineServer> {
		NonNull::from(self).cast()
	}
}

/// Notes that one of the engine's functions ran.
///
/// # Safety
///
/// `name` must be a string.
unsafe fn note(function: &'static str, name: *const c_char, report: bool) {
	// SAFETY: As the caller promises.
	let name = unsafe { CStr::from_ptr(name) }.to_owned();

	CALLS.with_borrow_mut(|calls| calls.push((function, name, report)));
}

/// The engine's `CreateFakeClient`, which reports the client.
unsafe extern "C" fn engine_create(
	_this: *mut sys::IVEngineServer,
	name: *const c_char,
) -> *mut sys::edict_t {
	// SAFETY: The tests pass strings.
	unsafe { note("create", name, true) };
	EDICT
}

/// The engine's `CreateFakeClientEx`.
unsafe extern "C" fn engine_create_ex(
	_this: *mut sys::IVEngineServer,
	name: *const c_char,
	report: bool,
) -> *mut sys::edict_t {
	// SAFETY: The tests pass strings.
	unsafe { note("create_ex", name, report) };
	EDICT
}

/// Reports what its creator asks to, unless its name starts with `hidden`.
fn report(name: &CStr, requested: bool) -> bool {
	requested && !name.to_bytes().starts_with(b"hidden")
}

fn take() -> Vec<(&'static str, CString, bool)> {
	CALLS.take()
}

fn call(function: &'static str, name: &CStr, report: bool) -> (&'static str, CString, bool) {
	(function, name.to_owned(), report)
}

#[test]
fn chosen_clients_are_created_unreported_and_the_rest_as_asked() {
	on_both(|harness| {
		let api = harness.api();
		let mut engine = Engine::new();
		let this = engine.ptr().as_ptr();

		// SAFETY: The mock engine has both functions at their slots, and is
		// leaked for the test's duration.
		let hooks = unsafe { api.install_fake_clients(engine.ptr(), report) }.unwrap();

		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_fake_clients(engine.ptr(), report) },
			Err(HookError::AlreadyInstalled)
		));

		let create = |name: &CStr| {
			harness.call::<CreateFakeClient>(this, CREATE_FAKE_CLIENT_SLOT, (name.as_ptr(),))
		};

		let create_ex = |name: &CStr, report: bool| {
			harness.call::<CreateFakeClientEx>(
				this,
				CREATE_FAKE_CLIENT_EX_SLOT,
				(name.as_ptr(), report),
			)
		};

		assert_eq!(create(c"Scout"), EDICT);
		assert_eq!(take(), [call("create", c"Scout", true)]);

		// The engine's own `CreateFakeClientEx` creates the client in its place,
		// once.
		assert_eq!(create(c"hidden Spy"), EDICT);
		assert_eq!(take(), [call("create_ex", c"hidden Spy", false)]);

		assert_eq!(create_ex(c"hidden Heavy", true), EDICT);
		assert_eq!(take(), [call("create_ex", c"hidden Heavy", false)]);

		// A client its creator asks not to report stays unreported.
		assert_eq!(create_ex(c"Robot", false), EDICT);
		assert_eq!(take(), [call("create_ex", c"Robot", false)]);

		assert_eq!(create_ex(c"Medic", true), EDICT);
		assert_eq!(take(), [call("create_ex", c"Medic", true)]);

		hooks.remove(api);

		assert_eq!(create(c"hidden Pyro"), EDICT);
		assert_eq!(take(), [call("create", c"hidden Pyro", true)]);

		// Removed hooks can be replaced.
		// SAFETY: As above.
		let hooks = unsafe { api.install_fake_clients(engine.ptr(), report) }.unwrap();

		assert_eq!(create(c"hidden Pyro"), EDICT);
		assert_eq!(take(), [call("create_ex", c"hidden Pyro", false)]);
		hooks.remove(api);
	});
}
