//! Tests of formatting `printf`-style output with the C runtime, and of
//! capturing it.

use source_sdk_2013_raw::util::printf::{CAPTURE_LINE_CAPACITY, capture_printf, vsnprintf};
use std::ffi::{CStr, CString, c_char, c_int};

#[test]
fn captured_lines_are_truncated() {
	let long = CString::new("x".repeat(CAPTURE_LINE_CAPACITY + 10)).unwrap();
	let ((), output) = capture_printf(|print| {
		// SAFETY: The format matches its argument.
		unsafe { print(c"%s".as_ptr(), long.as_ptr()) };
	});

	assert_eq!(
		output.as_bytes(),
		&long.as_bytes()[..CAPTURE_LINE_CAPACITY - 1]
	);
}

/// Formats its arguments with [`vsnprintf`] into a buffer of 6 bytes.
unsafe extern "C" fn format_into(
	output: *mut [c_char; 6],
	format: *const c_char,
	arguments: ...
) -> c_int {
	// SAFETY: The tests pass a live buffer, and formats matching their
	// arguments.
	unsafe { vsnprintf(&mut *output, format, arguments) }
}

#[test]
fn formatting_truncates_and_terminates() {
	let mut buffer = [-1 as c_char; 6];

	// SAFETY: The buffer is live, and the format matches its argument.
	let len = unsafe { format_into(&raw mut buffer, c"cp_%s".as_ptr(), c"dustbowl".as_ptr()) };

	assert_eq!(len, 11);
	// SAFETY: The formatter terminated the buffer.
	assert_eq!(unsafe { CStr::from_ptr(buffer.as_ptr()) }, c"cp_du");
}

#[test]
fn unwinding_captures_restore_the_outer_one() {
	let ((), outer) = capture_printf(|print| {
		let unwound = std::panic::catch_unwind(|| capture_printf(|_| panic!("unwinds")));

		assert!(unwound.is_err());

		// SAFETY: The format reads no arguments.
		unsafe { print(c"kept".as_ptr()) };
	});

	assert_eq!(outer.as_c_str(), c"kept");
}
