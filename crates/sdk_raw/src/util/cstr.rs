//! Conversions between Rust strings and the C strings the engine reads and
//! writes.

use std::ffi::{CStr, CString, c_char};

/// Borrows a string the engine returned, or returns `None` for null.
///
/// # Safety
///
/// A non-null `pointer` must reference a NUL-terminated string that stays
/// allocated and unmodified for `'a`.
pub unsafe fn borrow_cstr<'a>(pointer: *const c_char) -> Option<&'a CStr> {
	// SAFETY: The caller upholds the contract for non-null pointers.
	(!pointer.is_null()).then(|| unsafe { CStr::from_ptr(pointer) })
}

/// Copies a string into a fixed buffer for an in/out parameter.
///
/// Returns `None` if the string and its terminator do not fit.
pub fn buffer_from_cstr<const N: usize>(value: &CStr) -> Option<[c_char; N]> {
	let bytes = value.to_bytes_with_nul();
	let mut buffer = [0; N];

	if bytes.len() > N {
		return None;
	}

	for (slot, &byte) in buffer.iter_mut().zip(bytes) {
		*slot = byte as c_char;
	}

	Some(buffer)
}

/// Copies a string the engine returned, or returns `None` for null.
///
/// # Safety
///
/// A non-null `pointer` must reference a NUL-terminated string for the
/// duration of the call.
pub unsafe fn copy_cstr(pointer: *const c_char) -> Option<CString> {
	// SAFETY: The string only needs to live until it is copied.
	unsafe { borrow_cstr(pointer) }.map(CStr::to_owned)
}

/// Copies the NUL-terminated prefix of a buffer the engine wrote into.
///
/// An unterminated buffer is copied whole.
pub fn cstring_from_buffer(buffer: &[c_char]) -> CString {
	let bytes = buffer
		.iter()
		.map(|&byte| byte as u8)
		.take_while(|&byte| byte != 0)
		.collect();

	// SAFETY: `take_while` stopped before the first NUL.
	unsafe { CString::from_vec_unchecked(bytes) }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn buffers_round_trip_strings() {
		let buffer = buffer_from_cstr::<8>(c"ctf_2f").unwrap();

		assert_eq!(cstring_from_buffer(&buffer).as_c_str(), c"ctf_2f");
		assert_eq!(buffer_from_cstr::<6>(c"ctf_2f"), None);
		assert_eq!(
			cstring_from_buffer(&[b'a' as c_char, b'b' as c_char]).as_c_str(),
			c"ab"
		);
	}

	#[test]
	fn null_pointers_are_none() {
		// SAFETY: Null is always accepted.
		assert!(unsafe { borrow_cstr(std::ptr::null()) }.is_none());
		// SAFETY: As above.
		assert!(unsafe { copy_cstr(std::ptr::null()) }.is_none());
		// SAFETY: The literal is NUL-terminated and static.
		assert_eq!(
			unsafe { copy_cstr(c"cp_dustbowl".as_ptr()) }.as_deref(),
			Some(c"cp_dustbowl")
		);
	}
}
