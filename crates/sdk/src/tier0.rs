//! tier0's console output, found at runtime so the crate links against no
//! Source library.

use std::ffi::{CStr, c_char, c_void};
use std::sync::OnceLock;

/// `Msg` from `public/tier0/dbg.h`, which tier0 exports with C linkage.
pub(crate) type MsgFn = unsafe extern "C" fn(format: *const c_char, ...);

#[cfg(test)]
thread_local! {
	/// Stands in for tier0's `Msg` on this thread, since tests load no tier0.
	pub(crate) static TEST_MSG: std::cell::Cell<Option<MsgFn>> = const { std::cell::Cell::new(None) };
}

/// Prints through tier0's `Msg`, whose output the dedicated server's console
/// and rcon's redirection both receive. Returns `false` if tier0 is not
/// loaded.
pub(crate) fn print(message: &CStr) -> bool {
	static MSG: OnceLock<Option<MsgFn>> = OnceLock::new();

	#[cfg(test)]
	let test_msg = TEST_MSG.get();
	#[cfg(not(test))]
	let test_msg = None;

	let Some(msg) = test_msg.or_else(|| *MSG.get_or_init(find_msg)) else {
		return false;
	};

	// SAFETY: `Msg` is a `printf`-style function, and the message is passed as
	// the argument of a constant format, so it is never interpreted as one.
	unsafe { msg(c"%s".as_ptr(), message.as_ptr()) };
	true
}

fn find_msg() -> Option<MsgFn> {
	// Miri cannot call the platform's loader.
	if cfg!(miri) {
		return None;
	}

	let address = platform::find_symbol(c"Msg")?;

	// SAFETY: tier0 exports `Msg` with this signature.
	Some(unsafe { std::mem::transmute::<*mut c_void, MsgFn>(address) })
}

#[cfg(windows)]
mod platform {
	use std::ffi::{CStr, c_char, c_void};
	use std::ptr::NonNull;

	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
		fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
	}

	pub(super) fn find_symbol(name: &CStr) -> Option<*mut c_void> {
		// SAFETY: A module handle stays valid while the module is loaded, and
		// the engine never unloads tier0.
		let module = NonNull::new(unsafe { GetModuleHandleA(c"tier0.dll".as_ptr()) })?;

		// SAFETY: As above.
		NonNull::new(unsafe { GetProcAddress(module.as_ptr(), name.as_ptr()) }).map(NonNull::as_ptr)
	}
}

#[cfg(target_os = "linux")]
mod platform {
	use std::ffi::{CStr, c_char, c_int, c_void};
	use std::ptr::NonNull;

	const RTLD_NOW: c_int = 2;
	const RTLD_NOLOAD: c_int = 4;

	/// The names tier0 has in 64-bit and older dedicated servers.
	const NAMES: [&CStr; 2] = [c"libtier0.so", c"libtier0_srv.so"];

	#[link(name = "dl")]
	unsafe extern "C" {
		fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
		fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
		fn dlclose(handle: *mut c_void) -> c_int;
	}

	pub(super) fn find_symbol(name: &CStr) -> Option<*mut c_void> {
		NAMES.into_iter().find_map(|library| {
			// SAFETY: `RTLD_NOLOAD` only finds a library that is already loaded.
			let handle = NonNull::new(unsafe { dlopen(library.as_ptr(), RTLD_NOW | RTLD_NOLOAD) })?;

			// SAFETY: The handle is live until closed.
			let symbol = unsafe { dlsym(handle.as_ptr(), name.as_ptr()) };

			// SAFETY: This releases only the reference `dlopen` added; the engine
			// keeps tier0 loaded.
			unsafe { dlclose(handle.as_ptr()) };

			NonNull::new(symbol).map(NonNull::as_ptr)
		})
	}
}
