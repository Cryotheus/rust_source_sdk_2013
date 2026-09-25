//! Helpers shared by the wrappers that call into C++.

use std::ffi::{CStr, CString, c_char};
use std::marker::PhantomData;

/// Makes a type `!Send` and `!Sync`.
///
/// Engine state may only be touched from the server's main thread.
pub(crate) type NotThreadSafe = PhantomData<*mut ()>;

/// Calls a virtual method through a generated binding's vtable.
///
/// `vcall!(this => Method(arguments...))` reads the vtable pointer and calls
/// `Method` with `this` as the receiver, without creating a reference to the
/// object. It must be used inside `unsafe`, with `this` pointing to a live
/// object whose vtable matches the binding.
macro_rules! vcall {
	($this:expr => $method:ident($($argument:expr),* $(,)?)) => {{
		let this = $this;
		let vtable = (&raw const (*this).vtable_).read();

		((*vtable).$method)(this $(, $argument)*)
	}};
}

pub(crate) use vcall;

/// Borrows a string the engine returned, or returns `None` for null.
///
/// # Safety
///
/// A non-null `pointer` must reference a NUL-terminated string that stays
/// allocated and unmodified for `'a`.
pub(crate) unsafe fn borrow_cstr<'a>(pointer: *const c_char) -> Option<&'a CStr> {
	// SAFETY: The caller upholds the contract for non-null pointers.
	(!pointer.is_null()).then(|| unsafe { CStr::from_ptr(pointer) })
}

/// Copies a string the engine returned, or returns `None` for null.
///
/// # Safety
///
/// A non-null `pointer` must reference a NUL-terminated string for the
/// duration of the call.
pub(crate) unsafe fn copy_cstr(pointer: *const c_char) -> Option<CString> {
	// SAFETY: The string only needs to live until it is copied.
	unsafe { borrow_cstr(pointer) }.map(CStr::to_owned)
}

/// Copies the NUL-terminated prefix of a buffer the engine wrote into.
///
/// An unterminated buffer is copied whole.
pub(crate) fn cstring_from_buffer(buffer: &[c_char]) -> CString {
	let bytes = buffer
		.iter()
		.map(|&byte| byte as u8)
		.take_while(|&byte| byte != 0)
		.collect();

	// SAFETY: `take_while` stopped before the first NUL.
	unsafe { CString::from_vec_unchecked(bytes) }
}

/// Copies a string into a fixed buffer for an in/out parameter.
///
/// Returns `None` if the string and its terminator do not fit.
pub(crate) fn buffer_from_cstr<const N: usize>(value: &CStr) -> Option<[c_char; N]> {
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

#[cfg(test)]
pub(crate) mod test_support {
	use std::mem::size_of;

	/// Builds a vtable whose every slot holds `fill`, then lets `patch` install
	/// the methods a test expects to be called.
	///
	/// # Safety
	///
	/// `V` must consist solely of function pointer slots, and `fill` must be
	/// callable through any of them without being reached, e.g. an aborting stub.
	pub(crate) unsafe fn mock_vtable<V>(fill: *const (), patch: impl FnOnce(*mut V)) -> Box<V> {
		const SLOT: usize = size_of::<*const ()>();

		assert_eq!(size_of::<V>() % SLOT, 0);

		let mut vtable = Box::<V>::new_uninit();
		let slots = vtable.as_mut_ptr().cast::<*const ()>();

		for slot in 0..size_of::<V>() / SLOT {
			// SAFETY: The slot lies within the allocation.
			unsafe { slots.add(slot).write(fill) };
		}

		patch(vtable.as_mut_ptr());

		// SAFETY: Every slot now holds a function pointer.
		unsafe { vtable.assume_init() }
	}

	/// A vtable slot that fails the test if it is ever called.
	pub(crate) unsafe extern "C" fn unexpected_call() {
		// Unwinding cannot leave an `extern "C"` function, so this aborts.
		panic!("unexpected virtual call");
	}
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
}
