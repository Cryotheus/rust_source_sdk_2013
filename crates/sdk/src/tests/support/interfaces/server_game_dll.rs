//! A mock `IServerGameDLL`, which hands out the standard send proxies.

use super::super::datatables::proxies;
use super::super::leak;
use super::super::server::export;
use crate::interfaces::ServerGameDll;
use crate::server::Module;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::Cell;
use std::ptr::null_mut;

thread_local! {
	/// What [`standard_send_proxies`] returns.
	static PROXIES: Cell<*mut sys::CStandardSendProxies> = const { Cell::new(null_mut()) };
}

/// Exports a game DLL from the game server's factory of
/// [`mock_server`](super::super::server::mock_server), whose
/// `GetStandardSendProxies` returns new leaked [`proxies`] without registered
/// pointer-preserving proxies.
///
/// For tests only.
pub fn export_standard_proxies() {
	PROXIES.set(leak(proxies(null_mut())));

	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes a slot of the vtable
	// being built.
	let vtable = Box::leak(unsafe {
		mock_vtable::<sys::IServerGameDLL__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerGameDLL_GetStandardSendProxies).write(standard_send_proxies);
		})
	});

	export(
		Module::GameServer,
		ServerGameDll::VERSION,
		leak(sys::IServerGameDLL { vtable_: vtable }),
	);
}

/// `IServerGameDLL::GetStandardSendProxies`, which returns the proxies the
/// last [`export_standard_proxies`] made on this thread.
unsafe extern "C" fn standard_send_proxies(
	_: *mut sys::IServerGameDLL,
) -> *mut sys::CStandardSendProxies {
	PROXIES.get()
}
