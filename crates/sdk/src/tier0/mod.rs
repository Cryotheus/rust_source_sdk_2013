//! tier0's console output, found at runtime so the crate links against no
//! Source library.

use std::ffi::{CStr, c_char, c_void};
use std::sync::OnceLock;

/// `Msg` from `public/tier0/dbg.h`, which tier0 exports with C linkage.
pub(crate) type MsgFn = unsafe extern "C" fn(format: *const c_char, ...);

/// The names tier0 has: on Windows, and in 64-bit and older Linux dedicated
/// servers.
const LIBRARIES: &[&CStr] = cfg_select! {
	windows => &[c"tier0.dll"],
	target_os = "linux" => &[c"libtier0.so", c"libtier0_srv.so"],
};

#[cfg(test)]
thread_local! {
	/// Stands in for tier0's `Msg` on this thread, since tests load no tier0.
	pub(crate) static TEST_MSG: std::cell::Cell<Option<MsgFn>> = const { std::cell::Cell::new(None) };
}

/// Looks up `Msg` in the tier0 library the process has already loaded.
/// Returns `None` if tier0 is not loaded or does not export it, and under Miri.
fn find_msg() -> Option<MsgFn> {
	// Miri cannot call the platform's loader.
	if cfg!(miri) {
		return None;
	}

	let address = LIBRARIES
		.iter()
		.find_map(|library| sdk_raw::util::loaded_symbol(library, c"Msg"))?;

	// SAFETY: tier0 exports `Msg` with this signature.
	Some(unsafe { std::mem::transmute::<*mut c_void, MsgFn>(address.as_ptr()) })
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
