//! Tests of printing through tier0's `printf`-style `Msg`.

use source_sdk_2013_raw::tier0::print_through;
use std::cell::Cell;
use std::ffi::{CStr, c_char};
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

unsafe extern "C" fn record(format: *const c_char, _arguments: ...) {
	FORMAT.set(format);
}
