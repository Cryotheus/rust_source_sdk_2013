//! The exact base-filesystem interface version and generated ABI checks.
//!
//! `IBaseFileSystem` is the second base of `IFileSystem`; its exported pointer
//! must be requested directly, rather than cast from an `IFileSystem` pointer.

use std::ffi::CStr;
use std::mem::{align_of, offset_of, size_of};

const _: () = {
	assert!(size_of::<sys::IBaseFileSystem>() == 8);
	assert!(align_of::<sys::IBaseFileSystem>() == 8);
	assert!(size_of::<sys::FileHandle_t>() == 8);
	assert!(size_of::<sys::IBaseFileSystem__bindgen_vtable>() == 17 * 8);
	assert!(align_of::<sys::IBaseFileSystem__bindgen_vtable>() == 8);
	assert!(offset_of!(sys::IBaseFileSystem__bindgen_vtable, IBaseFileSystem_Read) == 0);
	assert!(offset_of!(sys::IBaseFileSystem__bindgen_vtable, IBaseFileSystem_Open) == 2 * 8);
	assert!(offset_of!(sys::IBaseFileSystem__bindgen_vtable, IBaseFileSystem_Close) == 3 * 8);
	assert!(offset_of!(sys::IBaseFileSystem__bindgen_vtable, IBaseFileSystem_Flush) == 8 * 8);

	#[cfg(target_os = "linux")]
	{
		assert!(offset_of!(sys::IBaseFileSystem__bindgen_vtable, IBaseFileSystem_Size) == 6 * 8);
		assert!(offset_of!(sys::IBaseFileSystem__bindgen_vtable, IBaseFileSystem_Size1) == 7 * 8);
	}

	#[cfg(target_os = "windows")]
	{
		assert!(offset_of!(sys::IBaseFileSystem__bindgen_vtable, IBaseFileSystem_Size) == 7 * 8);
		assert!(offset_of!(sys::IBaseFileSystem__bindgen_vtable, IBaseFileSystem_Size1) == 6 * 8);
	}
};

/// `IBaseFileSystem` as declared by the SDK's `filesystem.h`.
pub const VERSION: &CStr = c"VBaseFileSystem011";
