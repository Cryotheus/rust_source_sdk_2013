//! Tests of `crate::client_hooks`: post hooks of `ClientPutInServer` and
//! `ClientActive` on a mock `IServerGameClients`, through the mock SourceHook
//! and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::tf2_binding;
use std::cell::RefCell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;

/// The size of the mock `IServerGameClients`' vtable, which holds
/// [`game_put_in_server`] and [`game_client_active`] at their slots.
const SLOTS: usize = 8;

thread_local! {
	/// What ran during the calls since the last [`put_in_server`] or
	/// [`activate`], in order, with the edict each ran for.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };

	/// The mock `IServerGameClients` the game server's factory exports.
	static GAME_CLIENTS: Cell<*mut c_void> = const { Cell::new(ptr::null_mut()) };
}

/// Calls the mock's hooked `ClientActive` for `edict`, and returns what ran.
fn activate(
	harness: &Harness,
	clients: ServerGameClients<'_>,
	edict: *mut sys::edict_t,
) -> Vec<(&'static str, usize)> {
	CALLS.take();
	harness.call::<ClientActive>(clients.as_ptr(), CLIENT_ACTIVE_SLOT, (edict, false));
	CALLS.take()
}

/// A binding to a game server exporting only the mock of [`game_clients`].
fn binding() -> ServerBinding {
	tf2_binding(game_server_factory)
}

/// An edict for the tests, which only compare its address.
fn edict() -> *mut sys::edict_t {
	Box::into_raw(Box::new([0_u8; 64])).cast()
}

/// The game's `ClientActive`, which notes that it ran.
unsafe extern "C" fn game_client_active(
	_this: *mut sys::IServerGameClients,
	edict: *mut sys::edict_t,
	_load_game: bool,
) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", edict.addr())));
}

/// A new mock of the game's `IServerGameClients`, which the game server's
/// factory exports from then on.
fn game_clients(scope: &()) -> ServerGameClients<'_> {
	let vtable = Vec::leak(vec![ptr::null_mut::<c_void>(); SLOTS]);

	vtable[CLIENT_ACTIVE_SLOT] = game_client_active as ClientActive as *mut c_void;
	vtable[CLIENT_PUT_IN_SERVER_SLOT] = game_put_in_server as ClientPutInServer as *mut c_void;

	let object = Box::leak(Box::new(sys::IServerGameClients {
		vtable_: vtable.as_mut_ptr().cast(),
	}));

	GAME_CLIENTS.set(ptr::from_mut(object).cast());

	// SAFETY: The tests leak the mock the factory exports, and only turn the
	// binding into servers on the thread running their hooks, within the
	// test's call.
	let server = unsafe { binding().server(scope) };

	server
		.server_game_clients()
		.expect("the factory exports the mock")
}

/// The game's `ClientPutInServer`, which notes that it ran.
unsafe extern "C" fn game_put_in_server(
	_this: *mut sys::IServerGameClients,
	edict: *mut sys::edict_t,
	_name: *const c_char,
) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", edict.addr())));
}

unsafe extern "C" fn game_server_factory(
	name: *const c_char,
	_return_code: *mut c_int,
) -> *mut c_void {
	// SAFETY: Factories are called with NUL-terminated names.
	let name = unsafe { CStr::from_ptr(name) };

	if name == ServerGameClients::VERSION {
		GAME_CLIENTS.get()
	} else {
		ptr::null_mut()
	}
}

fn on_active(_server: Server<'_>, edict: Edict<'_>) {
	CALLS.with_borrow_mut(|calls| calls.push(("active", edict.as_ptr().addr())));
}

fn on_put_in_server(_server: Server<'_>, edict: Edict<'_>) {
	CALLS.with_borrow_mut(|calls| calls.push(("put in server", edict.as_ptr().addr())));
}

/// Calls the mock's hooked `ClientPutInServer` for `edict`, and returns what
/// ran.
fn put_in_server(
	harness: &Harness,
	clients: ServerGameClients<'_>,
	edict: *mut sys::edict_t,
) -> Vec<(&'static str, usize)> {
	CALLS.take();
	harness.call::<ClientPutInServer>(
		clients.as_ptr(),
		CLIENT_PUT_IN_SERVER_SLOT,
		(edict, c"player".as_ptr()),
	);
	CALLS.take()
}

#[test]
fn client_events_run_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let clients = game_clients(&scope);
		let edict = edict();

		let events = ClientEvents {
			put_in_server: Some(on_put_in_server),
			active: Some(on_active),
		};

		api.hook_client_events(clients, binding(), events).unwrap();

		// The callbacks are installed once.
		assert_eq!(
			api.hook_client_events(clients, binding(), events),
			Err(HookError::AlreadyInstalled)
		);

		assert_eq!(
			put_in_server(harness, clients, edict),
			[("game", edict.addr()), ("put in server", edict.addr())]
		);
		assert_eq!(
			activate(harness, clients, edict),
			[("game", edict.addr()), ("active", edict.addr())]
		);
	});
}

#[test]
fn client_events_only_hook_their_callbacks() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let clients = game_clients(&scope);
		let edict = edict();

		let events = ClientEvents {
			active: Some(on_active),
			..ClientEvents::default()
		};

		api.hook_client_events(clients, binding(), events).unwrap();

		assert_eq!(
			put_in_server(harness, clients, edict),
			[("game", edict.addr())]
		);
		assert_eq!(
			activate(harness, clients, edict),
			[("game", edict.addr()), ("active", edict.addr())]
		);
	});
}
