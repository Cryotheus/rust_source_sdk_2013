//! Tests of `crate::fake_client_hooks`: hooks of `CreateFakeClient` and
//! `CreateFakeClientEx` on a mock engine that creates clients as TF2's does,
//! through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_void};

thread_local! {
	/// What the callback was asked since the last [`take`]: each client's name,
	/// and whether its creator asked for it to be reported.
	static ASKED: RefCell<Vec<(CString, bool)>> = const { RefCell::new(Vec::new()) };

	/// The clients the engine created since the last [`take`], with whether it
	/// reports each.
	static CREATED: RefCell<Vec<(CString, bool)>> = const { RefCell::new(Vec::new()) };

	/// The harness the engine's `CreateFakeClientEx` calls `CreateFakeClient`
	/// through, as the engine calls it through its hooked vtable.
	static HARNESS: Cell<*const Harness> = const { Cell::new(std::ptr::null()) };

	/// The engine's choice for the fake clients it creates.
	static REPORT: Cell<bool> = const { Cell::new(true) };
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

/// The client with `name` and the engine's choice.
fn client(name: &CStr, report: bool) -> (CString, bool) {
	(name.to_owned(), report)
}

/// The engine's `CreateFakeClient`, which reports the client as the engine
/// chose.
unsafe extern "C" fn engine_create(
	_this: *mut sys::IVEngineServer,
	name: *const c_char,
) -> *mut sys::edict_t {
	// SAFETY: The tests pass strings.
	let name = unsafe { CStr::from_ptr(name) };

	CREATED.with_borrow_mut(|created| created.push(client(name, REPORT.get())));
	EDICT
}

/// The engine's `CreateFakeClientEx`, which calls `CreateFakeClient` through
/// its vtable with its choice, as TF2's does.
unsafe extern "C" fn engine_create_ex(
	this: *mut sys::IVEngineServer,
	name: *const c_char,
	report: bool,
) -> *mut sys::edict_t {
	// SAFETY: The test running set the harness, which outlives its calls.
	let harness = unsafe { &*HARNESS.get() };

	REPORT.set(report);

	let edict = harness.call::<CreateFakeClient>(this, CREATE_FAKE_CLIENT_SLOT, (name,));

	REPORT.set(true);
	edict
}

/// Hides the clients whose names start with `hidden`, reports those whose
/// names start with `shown`, and the rest as their creators ask.
fn report(name: &CStr, requested: bool) -> bool {
	ASKED.with_borrow_mut(|asked| asked.push(client(name, requested)));

	match name.to_bytes() {
		hidden if hidden.starts_with(b"hidden") => false,
		shown if shown.starts_with(b"shown") => true,
		_ => requested,
	}
}

/// Clients' names, each with whether it was asked for or created reported.
type Clients = Vec<(CString, bool)>;

/// What the callback was asked, and the clients the engine created, since the
/// last call.
fn take() -> (Clients, Clients) {
	assert!(REPORT.get(), "the engine's choice is left at reporting");
	(ASKED.take(), CREATED.take())
}

#[test]
fn the_callback_chooses_once_for_each_client_how_it_is_created() {
	on_both(|harness| {
		HARNESS.set(harness);

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

		// `CreateFakeClient` reports clients.
		assert_eq!(create(c"Scout"), EDICT);
		assert_eq!(
			take(),
			(vec![client(c"Scout", true)], vec![client(c"Scout", true)])
		);

		assert_eq!(create(c"hidden Spy"), EDICT);
		assert_eq!(
			take(),
			(
				vec![client(c"hidden Spy", true)],
				vec![client(c"hidden Spy", false)]
			)
		);

		// `CreateFakeClientEx` reports them as its creator asks, through
		// `CreateFakeClient`.
		assert_eq!(create_ex(c"Medic", true), EDICT);
		assert_eq!(
			take(),
			(vec![client(c"Medic", true)], vec![client(c"Medic", true)])
		);

		assert_eq!(create_ex(c"Robot", false), EDICT);
		assert_eq!(
			take(),
			(vec![client(c"Robot", false)], vec![client(c"Robot", false)])
		);

		assert_eq!(create_ex(c"hidden Heavy", true), EDICT);
		assert_eq!(
			take(),
			(
				vec![client(c"hidden Heavy", true)],
				vec![client(c"hidden Heavy", false)]
			)
		);

		assert_eq!(create_ex(c"shown Robot", false), EDICT);
		assert_eq!(
			take(),
			(
				vec![client(c"shown Robot", false)],
				vec![client(c"shown Robot", true)]
			)
		);

		hooks.remove(api);

		assert_eq!(create(c"hidden Pyro"), EDICT);
		assert_eq!(take(), (vec![], vec![client(c"hidden Pyro", true)]));

		// Removed hooks can be replaced.
		// SAFETY: As above.
		let hooks = unsafe { api.install_fake_clients(engine.ptr(), report) }.unwrap();

		assert_eq!(create_ex(c"hidden Pyro", true), EDICT);
		assert_eq!(
			take(),
			(
				vec![client(c"hidden Pyro", true)],
				vec![client(c"hidden Pyro", false)]
			)
		);

		hooks.remove(api);
		HARNESS.set(std::ptr::null());
	});
}
