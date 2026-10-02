use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::NonNull;

/// The names tier0 has in 64-bit and older dedicated servers.
const NAMES: [&CStr; 2] = [c"libtier0.so", c"libtier0_srv.so"];

/// `dlopen`'s flag to only find a library that is already loaded.
const RTLD_NOLOAD: c_int = 4;

/// `dlopen`'s flag to resolve every symbol before returning.
const RTLD_NOW: c_int = 2;

#[link(name = "dl")]
unsafe extern "C" {
	fn dlclose(handle: *mut c_void) -> c_int;
	fn dlopen(file: *const c_char, mode: c_int) -> *mut c_void;
	fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// The address of `name` in the first loaded library of `NAMES` that
/// exports it, or `None` if there is none.
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
