//! tier0's exports, found at runtime so that no Source library is linked, and
//! values from its headers.
//!
//! tier0 is the engine's base library, which every other Source module links
//! against. It is loaded before them and unloaded after them, so an export
//! found while it is loaded stays callable for as long as any module that
//! uses this crate is.

use crate::util::loaded_symbol;
use std::ffi::{CStr, c_char, c_void};
use std::sync::OnceLock;

/// `Msg` from `public/tier0/dbg.h`, which tier0 exports with C linkage, and
/// which prints a `printf`-style format and its arguments to the console.
#[doc(alias = "Msg")]
pub type MsgFn = unsafe extern "C" fn(format: *const c_char, ...);

/// The names tier0 has: on Windows, and in 64-bit and older Linux dedicated
/// servers.
const LIBRARIES: &[&CStr] = cfg_select! {
	windows => &[c"tier0.dll"],
	target_os = "linux" => &[c"libtier0.so", c"libtier0_srv.so"],
};

/// The size of a buffer for a file path, terminator included.
///
/// This is `MAX_PATH` from `public/tier0/platform.h`.
pub const MAX_PATH: usize = 260;

/// Looks up `Msg` in the tier0 library the process has already loaded.
/// Returns `None` if tier0 is not loaded or does not export it, and under Miri.
fn find_msg() -> Option<MsgFn> {
	// Miri cannot call the platform's loader.
	if cfg!(miri) {
		return None;
	}

	let address = LIBRARIES
		.iter()
		.find_map(|library| loaded_symbol(library, c"Msg"))?;

	// SAFETY: tier0 exports `Msg` with this signature.
	Some(unsafe { std::mem::transmute::<*mut c_void, MsgFn>(address.as_ptr()) })
}

/// tier0's `Msg`, or `None` if tier0 is not loaded or does not export it, and
/// under Miri.
///
/// The process's tier0 is looked up on the first call, and its result kept.
#[doc(alias = "Msg")]
pub fn msg() -> Option<MsgFn> {
	static MSG: OnceLock<Option<MsgFn>> = OnceLock::new();

	*MSG.get_or_init(find_msg)
}

/// Prints `message` through tier0's `Msg`, whose output the dedicated server's
/// console and rcon's redirection both receive. Returns `false` if tier0 is
/// not loaded.
///
/// The message is printed as is, never interpreted as a format.
pub fn print(message: &CStr) -> bool {
	let Some(msg) = msg() else {
		return false;
	};

	// SAFETY: `msg` is tier0's `Msg`, which formats as `printf` does, and
	// tier0 outlives every module that uses this crate.
	unsafe { print_through(msg, message) };
	true
}

/// Prints `message` through `msg`, a `printf`-style function such as tier0's
/// `Msg`, as the argument of a constant `"%s"` format, so the message is never
/// interpreted as one.
///
/// # Safety
///
/// `msg` must be callable, and read its format and arguments as `printf`
/// does.
pub unsafe fn print_through(msg: MsgFn, message: &CStr) {
	// SAFETY: As the caller promises; the format consumes exactly the one
	// string argument passed.
	unsafe { msg(c"%s".as_ptr(), message.as_ptr()) };
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::cell::Cell;
	use std::ptr::null;

	thread_local! {
		static FORMAT: Cell<*const c_char> = const { Cell::new(null()) };
	}

	#[test]
	fn messages_are_never_formats() {
		// SAFETY: `record` reads nothing but its format.
		unsafe { print_through(record, c"100%n") };

		// SAFETY: `print_through` passed a static, terminated format.
		assert_eq!(unsafe { CStr::from_ptr(FORMAT.get()) }, c"%s");
	}

	#[test]
	fn nothing_is_printed_without_tier0() {
		assert!(msg().is_none());
		assert!(!print(c"unprinted"));
	}

	unsafe extern "C" fn record(format: *const c_char, _arguments: ...) {
		FORMAT.set(format);
	}
}
