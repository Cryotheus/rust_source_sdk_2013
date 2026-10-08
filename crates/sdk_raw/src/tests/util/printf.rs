//! Tests of the capture of `printf`-style output.

use super::*;

#[test]
fn captures_nest_and_end() {
	let (inner, outer) = capture_printf(|print| {
		// SAFETY: Each format matches its arguments.
		unsafe { print(c"Blue Team Wins: %d\n".as_ptr(), 3 as c_int) };

		let ((), inner) = capture_printf(|print| {
			// SAFETY: The format matches its arguments.
			unsafe { print(c"%-8s %6.1f\n".as_ptr(), c"Scout".as_ptr(), 2.5f64) }
		});

		// SAFETY: A null format is ignored.
		unsafe { print(std::ptr::null()) };
		inner
	});

	assert_eq!(outer.as_c_str(), c"Blue Team Wins: 3\n");
	assert_eq!(inner.as_c_str(), c"Scout       2.5\n");

	// Outside of every capture, a call is ignored.
	// SAFETY: The format reads no arguments.
	unsafe { append_captured(c"lost".as_ptr()) };
	assert_eq!(CAPTURED.take(), None);
}
