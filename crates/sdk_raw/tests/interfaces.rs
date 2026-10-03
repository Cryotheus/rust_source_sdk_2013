//! Tests of asking a module's `CreateInterface` factory for its interfaces.

use source_sdk_2013_raw::interfaces::create_interface;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::{NonNull, null_mut};

static mut INTERFACE: u8 = 0;

unsafe extern "C" fn factory(name: *const c_char, return_code: *mut c_int) -> *mut c_void {
	// SAFETY: `create_interface` passes a terminated name.
	let found = unsafe { CStr::from_ptr(name) } == c"Interface001";

	// SAFETY: `create_interface` passes a writable return code.
	unsafe { return_code.write(c_int::from(!found)) };

	if found {
		(&raw mut INTERFACE).cast()
	} else {
		null_mut()
	}
}

#[test]
fn missing_interfaces_are_none() {
	// SAFETY: `factory` stays loaded and takes any thread.
	unsafe {
		assert_eq!(
			create_interface(factory, c"Interface001").map(NonNull::as_ptr),
			Some((&raw mut INTERFACE).cast())
		);
		assert_eq!(create_interface(factory, c"Interface002"), None);
	}
}
