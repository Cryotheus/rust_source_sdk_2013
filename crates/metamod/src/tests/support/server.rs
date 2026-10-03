//! A TF2 server without an engine, for the hooks that run with a server.

use source_sdk_2013::raw::interfaces::CreateInterfaceFn;
use source_sdk_2013::{Game, InterfaceFactory, ServerBinding};
use std::ffi::{c_char, c_int, c_void};
use std::ptr;

/// A module's `CreateInterface`, exporting nothing.
pub(crate) unsafe extern "C" fn no_interfaces(
	_name: *const c_char,
	_return_code: *mut c_int,
) -> *mut c_void {
	ptr::null_mut()
}

/// A TF2 binding to an engine exporting no interface, and a game server
/// exporting what `game_server` does.
///
/// Turn it into servers only on the test's thread, for calls that reach no
/// interface but the ones `game_server` exports, which must outlive them.
pub(crate) fn tf2_binding(game_server: CreateInterfaceFn) -> ServerBinding {
	// SAFETY: There is no running server, so the tests vouch instead for what
	// they reach through the binding's servers, as above.
	unsafe {
		ServerBinding::new(
			InterfaceFactory::new(no_interfaces),
			InterfaceFactory::new(game_server),
			Game::TeamFortress2,
		)
	}
}
