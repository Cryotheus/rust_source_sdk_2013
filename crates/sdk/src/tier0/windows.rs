use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;

#[link(name = "kernel32")]
unsafe extern "system" {
	fn GetModuleHandleA(name: *const c_char) -> *mut c_void;
	fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// The address of `name` in the loaded `tier0.dll`, or `None` if it is not
/// loaded or does not export it.
pub(super) fn find_symbol(name: &CStr) -> Option<*mut c_void> {
	// SAFETY: A module handle stays valid while the module is loaded, and
	// the engine never unloads tier0.
	let module = NonNull::new(unsafe { GetModuleHandleA(c"tier0.dll".as_ptr()) })?;

	// SAFETY: As above.
	NonNull::new(unsafe { GetProcAddress(module.as_ptr(), name.as_ptr()) }).map(NonNull::as_ptr)
}
