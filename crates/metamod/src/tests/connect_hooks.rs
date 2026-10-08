//! Tests of `crate::connect_hooks`: pre hooks of `ClientConnect` on a mock
//! `IServerGameClients`, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::tf2_binding;
use source_sdk_2013::sys;
use std::cell::RefCell;
use std::ffi::{CString, c_char, c_int, c_void};
use std::ptr;

/// The reason [`on_connect`] refuses [`REFUSED`] with.
const REASON: &CStr = c"#GameUI_ServerRejectServerFull";

/// The name of the client [`on_connect`] refuses.
const REFUSED: &CStr = c"refused";

/// The size of the mock `IServerGameClients`'s vtable, every slot of which
/// holds [`game_connect`].
const SLOTS: usize = 8;

thread_local! {
	/// What ran during the calls since the last [`connect`], in order, with
	/// the name of the client each ran for.
	static CALLS: RefCell<Vec<(&'static str, CString)>> = const { RefCell::new(Vec::new()) };

	/// The mock `IServerGameClients` the game server's factory exports.
	static CLIENTS: Cell<*mut c_void> = const { Cell::new(ptr::null_mut()) };

	/// The edict, name and address the callback was last given.
	static REQUEST: RefCell<Option<(usize, CString, CString)>> = const { RefCell::new(None) };
}

/// A binding to a game server exporting only the mock of [`clients`].
fn binding() -> ServerBinding {
	tf2_binding(game_server_factory)
}

#[test]
fn client_connect_hooks_refuse_clients_with_a_reason() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let clients = clients(&scope);

		// SAFETY: An all-zero edict is a free slot of no table, which only its
		// address is taken of.
		let edict: *mut sys::edict_t = Box::leak(Box::new(unsafe { std::mem::zeroed() }));

		api.hook_client_connect(clients, binding(), on_connect)
			.unwrap();

		assert!(matches!(
			api.hook_client_connect(clients, binding(), on_connect),
			Err(HookError::AlreadyInstalled)
		));

		// An allowed client is left to the game, which lets it connect.
		assert_eq!(
			connect(harness, clients, edict, c"allowed", 64),
			(
				true,
				c"".to_owned(),
				vec![
					("callback", c"allowed".to_owned()),
					("game", c"allowed".to_owned())
				],
			)
		);

		assert_eq!(
			REQUEST.take(),
			Some((
				edict.addr(),
				c"allowed".to_owned(),
				c"192.0.2.1:27005".to_owned()
			))
		);

		// A refused client never reaches the game, and gets the reason.
		assert_eq!(
			connect(harness, clients, edict, REFUSED, 64),
			(
				false,
				REASON.to_owned(),
				vec![("callback", REFUSED.to_owned())]
			)
		);

		// A reason longer than the buffer is cut short, and still terminated.
		assert_eq!(
			connect(harness, clients, edict, REFUSED, 8),
			(
				false,
				c"#GameUI".to_owned(),
				vec![("callback", REFUSED.to_owned())]
			)
		);
	});
}

/// A new mock of the game's `IServerGameClients`, which the game server's
/// factory exports from then on.
fn clients(scope: &()) -> ServerGameClients<'_> {
	let vtable = Vec::leak(vec![game_connect as ClientConnect as *mut c_void; SLOTS]);

	let object = Box::leak(Box::new(sys::IServerGameClients {
		vtable_: vtable.as_mut_ptr().cast(),
	}));

	CALLS.take();
	REQUEST.take();
	CLIENTS.set(ptr::from_mut(object).cast());

	// SAFETY: The tests leak the mock the factory exports, and only turn the
	// binding into servers on the thread running their hooks, within the
	// test's call.
	let server = unsafe { binding().server(scope) };

	server
		.server_game_clients()
		.expect("the factory exports the mock")
}

/// Asks the mock's hooked `ClientConnect` whether a client named `name` may
/// connect, with an empty reject buffer of `capacity` bytes, and returns the
/// answer, what the buffer then holds, and what ran.
fn connect(
	harness: &Harness,
	clients: ServerGameClients<'_>,
	edict: *mut sys::edict_t,
	name: &CStr,
	capacity: usize,
) -> (bool, CString, Vec<(&'static str, CString)>) {
	let mut reject = vec![0 as c_char; capacity];

	let allowed = harness.call::<ClientConnect>(
		clients.as_ptr(),
		CLIENT_CONNECT.index(),
		(
			edict,
			name.as_ptr(),
			c"192.0.2.1:27005".as_ptr(),
			reject.as_mut_ptr(),
			c_int::try_from(capacity).unwrap(),
		),
	);

	// SAFETY: The buffer starts empty, and the hook writes a terminated reason
	// within it.
	let reason = unsafe { CStr::from_ptr(reject.as_ptr()) }.to_owned();

	(allowed, reason, CALLS.take())
}

/// The game's `ClientConnect`, which notes the client, and lets it connect.
unsafe extern "C" fn game_connect(
	_this: *mut sys::IServerGameClients,
	_edict: *mut sys::edict_t,
	name: *const c_char,
	_address: *const c_char,
	_reject: *mut c_char,
	_reject_capacity: c_int,
) -> bool {
	// SAFETY: The tests pass terminated names.
	let name = unsafe { CStr::from_ptr(name) }.to_owned();

	CALLS.with_borrow_mut(|calls| calls.push(("game", name)));
	true
}

unsafe extern "C" fn game_server_factory(
	name: *const c_char,
	_return_code: *mut c_int,
) -> *mut c_void {
	// SAFETY: Factories are called with NUL-terminated names.
	let name = unsafe { CStr::from_ptr(name) };

	if name == ServerGameClients::VERSION {
		CLIENTS.get()
	} else {
		ptr::null_mut()
	}
}

/// The callback, which notes the request, and refuses [`REFUSED`].
fn on_connect(_server: Server<'_>, request: ConnectRequest<'_, '_>) -> ConnectAction {
	let name = request.name.to_owned();

	REQUEST.set(Some((
		request.edict.as_ptr().addr(),
		name.clone(),
		request.address.to_owned(),
	)));

	CALLS.with_borrow_mut(|calls| calls.push(("callback", name)));

	if request.name == REFUSED {
		ConnectAction::Refuse(REASON.into())
	} else {
		ConnectAction::Allow
	}
}
