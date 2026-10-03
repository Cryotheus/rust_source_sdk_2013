//! Mock servers, whose interface factories export what a test registers.

use crate::server::{Game, InterfaceFactory, Module, Server, ServerBinding};
use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::null_mut;

thread_local! {
	/// The interfaces [`export`] registered on this thread, in order.
	static INTERFACES: RefCell<Vec<(Module, CString, *mut c_void)>> = const { RefCell::new(Vec::new()) };
}

/// The engine's factory of [`mock_server`].
unsafe extern "C" fn engine_factory(name: *const c_char, _return_code: *mut c_int) -> *mut c_void {
	find(Module::Engine, name)
}

/// Makes the mock factories of [`mock_server`] export an interface of
/// `module` as `version`.
///
/// For tests only. The first interface exported under a version is the one
/// found, and it must stay alive while the mock servers of this thread use
/// it.
pub fn export<T>(module: Module, version: &CStr, interface: *mut T) {
	INTERFACES.with_borrow_mut(|interfaces| {
		interfaces.push((module, version.to_owned(), interface.cast()));
	});
}

/// The interface [`export`] registered for `module` under `name`, or null.
fn find(module: Module, name: *const c_char) -> *mut c_void {
	// SAFETY: Factories are called with NUL-terminated names.
	let name = unsafe { CStr::from_ptr(name) };

	INTERFACES.with_borrow(|interfaces| {
		interfaces
			.iter()
			.find(|(owner, version, _)| *owner == module && version.as_c_str() == name)
			.map_or(null_mut(), |&(_, _, interface)| interface)
	})
}

/// The game server's factory of [`mock_server`].
unsafe extern "C" fn game_server_factory(
	name: *const c_char,
	_return_code: *mut c_int,
) -> *mut c_void {
	find(Module::GameServer, name)
}

/// A binding to the factories of [`mock_server`], for TF2.
///
/// For tests only.
pub fn mock_binding() -> ServerBinding {
	// SAFETY: Tests only export objects that outlive their use of the binding,
	// on the thread that exported them.
	unsafe {
		ServerBinding::new(
			InterfaceFactory::new(engine_factory),
			InterfaceFactory::new(game_server_factory),
			Game::TeamFortress2,
		)
	}
}

/// A TF2 server whose factories export only what [`export`] registered on
/// this thread.
///
/// For tests only.
pub fn mock_server<S: ?Sized>(scope: &S) -> Server<'_> {
	// SAFETY: Tests only export objects that outlive the scope they pass.
	unsafe { mock_binding().server(scope) }
}

/// A factory that exports no interface at all.
unsafe extern "C" fn no_interface(_: *const c_char, _: *mut c_int) -> *mut c_void {
	null_mut()
}

/// A server running `game` whose factories export no interface at all.
///
/// For tests only.
pub fn null_server<S: ?Sized>(game: Game, scope: &S) -> Server<'_> {
	let factory = InterfaceFactory::new(no_interface);

	// SAFETY: The factories export nothing, so nothing the server reaches can
	// outlive it.
	unsafe { Server::new(factory, factory, game, scope) }
}
