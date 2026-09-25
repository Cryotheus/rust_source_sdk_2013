//! C ABI shared with the small C++ `ISmmPlugin` shell.

use std::ffi::{c_char, c_int, c_void};
use std::mem::size_of;

#[derive(Debug)]
#[repr(C)]
pub struct PluginCallbacks {
	pub load: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_char, usize, bool) -> bool,
	pub all_plugins_loaded: unsafe extern "C" fn(*mut c_void),
	pub query_running: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> bool,
	pub unload: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> bool,
	pub pause: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> bool,
	pub unpause: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> bool,
}

#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct PluginMetadata {
	pub author: *const c_char,
	pub name: *const c_char,
	pub description: *const c_char,
	pub url: *const c_char,
	pub license: *const c_char,
	pub version: *const c_char,
	pub date: *const c_char,
	pub log_tag: *const c_char,
}

unsafe extern "C" {
	fn cpp_metamod_plugin_stable(
		callbacks: *const PluginCallbacks,
		metadata: PluginMetadata,
	) -> *mut c_void;
	fn cpp_metamod_plugin_dev(
		callbacks: *const PluginCallbacks,
		metadata: PluginMetadata,
	) -> *mut c_void;
	fn cpp_metamod_plugin_stable_instance() -> *mut c_void;
	fn cpp_metamod_plugin_dev_instance() -> *mut c_void;
}

/// Selects the C++ `ISmmPlugin` shell built against this API version's headers.
///
/// # Safety
///
/// `callbacks` and all metadata strings must remain valid until after Metamod
/// unloads the plugin. The shell must not be configured concurrently with its
/// callbacks.
pub unsafe fn cpp_metamod_plugin(
	api_version: c_int,
	callbacks: *const PluginCallbacks,
	metadata: PluginMetadata,
) -> *mut c_void {
	match api_version {
		16 => unsafe { cpp_metamod_plugin_stable(callbacks, metadata) },
		18 => unsafe { cpp_metamod_plugin_dev(callbacks, metadata) },
		_ => std::ptr::null_mut(),
	}
}

/// Returns the shell for a plugin API version without changing its state.
pub fn cpp_metamod_plugin_for_version(api_version: c_int) -> *mut c_void {
	match api_version {
		16 => unsafe { cpp_metamod_plugin_stable_instance() },
		18 => unsafe { cpp_metamod_plugin_dev_instance() },
		_ => std::ptr::null_mut(),
	}
}

const _: () = {
	let slot = size_of::<*const ()>();

	assert!(size_of::<PluginCallbacks>() == 6 * slot);
	assert!(size_of::<PluginMetadata>() == 8 * slot);
};
