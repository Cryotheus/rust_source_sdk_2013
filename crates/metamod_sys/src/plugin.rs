//! Raw plugin callbacks and status for the Rust-owned Metamod shell state.
//! Compiler-built C++ objects supply only the native virtual dispatch boundary.

use std::ffi::{c_char, c_int, c_void};
use std::mem::{offset_of, size_of};

/// Metamod's `IMetamodListener::OnLevelInit`, with the name of the level
/// loading.
pub type LevelInitCallback = unsafe extern "C" fn(context: *mut c_void, map: *const c_char);

/// Metamod's `IMetamodListener::OnLevelShutdown`.
pub type LevelShutdownCallback = unsafe extern "C" fn(context: *mut c_void);

const _: () = {
	let slot = size_of::<*const ()>();

	assert!(size_of::<PluginCallbacks>() == 6 * slot);
	assert!(size_of::<PluginMetadata>() == 8 * slot);

	assert!(offset_of!(PluginStatus, generation) == 0);
	assert!(offset_of!(PluginStatus, id) == 8);
	assert!(offset_of!(PluginStatus, loaded) == 12);
	assert!(offset_of!(PluginStatus, paused) == 13);
	assert!(size_of::<PluginStatus>() == 16);
};

/// The result of [`cpp_metamod_listen_levels`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct HookStatus(pub c_int);

impl HookStatus {
	pub const ALREADY_INSTALLED: Self = Self(2);
	pub const INSTALLED: Self = Self(0);
	pub const INVALID_ARGUMENT: Self = Self(3);

	/// The shell is not between `Load` and `Unload`.
	pub const NOT_BOUND: Self = Self(1);

	/// The hooking library refused the hook.
	pub const REFUSED: Self = Self(4);

	/// No shell exists for the plugin API version.
	pub const UNSUPPORTED: Self = Self(-1);
}

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

/// What the shell knows of the plugin Metamod loaded through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct PluginStatus {
	/// Counts the shell's `Load` calls, telling apart what an earlier load of
	/// a library that stayed mapped left behind.
	pub generation: u64,

	/// The `PluginId` Metamod passed to `Load`, while loaded, or else 0.
	pub id: c_int,

	/// Whether Metamod has loaded the shell and not yet unloaded it, as
	/// [`cpp_metamod_plugin_is_loaded`] reports.
	pub loaded: bool,

	/// Whether Metamod has paused the plugin.
	pub paused: bool,
}

impl PluginStatus {
	/// Reported for a plugin API version without a shell.
	pub const UNLOADED: Self = Self {
		generation: 0,
		id: 0,
		loaded: false,
		paused: false,
	};
}

unsafe extern "C" {
	fn cpp_metamod_listen_levels_dev(api: *mut c_void);
	fn cpp_metamod_listen_levels_stable(api: *mut c_void);
	fn cpp_metamod_plugin_dev_instance(dispatch: *const crate::shell::Dispatch) -> *mut c_void;
	fn cpp_metamod_plugin_stable_instance(dispatch: *const crate::shell::Dispatch) -> *mut c_void;
}

/// The listener notification generation, including notifications while paused.
/// Starts at zero on registration and advances for each init or shutdown;
/// unavailable before registration, after unload, for unsupported APIs or counter
/// exhaustion. It contains no map pointers and resets on a new load.
///
/// # Safety
///
/// Call only on the server main thread during a live shell callback.
pub unsafe fn cpp_metamod_level_generation(api_version: c_int) -> Option<u64> {
	crate::shell::level_generation(api_version)
}

/// Registers a Metamod listener until Metamod unloads the plugin.
/// The Rust route is reserved before the native registration call and no state
/// guard survives that call. Notifications are suppressed while paused.
///
/// # Safety
///
/// Call on the server main thread from a live shell callback, from Load on.
/// Callbacks and context must remain valid until the plugin unloads.
pub unsafe fn cpp_metamod_listen_levels(
	api_version: c_int,
	init: Option<LevelInitCallback>,
	shutdown: Option<LevelShutdownCallback>,
	context: *mut c_void,
) -> HookStatus {
	let api = match crate::shell::listen(api_version, init, shutdown, context) {
		Ok(api) => api,
		Err(status) => return status,
	};
	// SAFETY: The Rust-owned shell supplied this load's API and reserved its
	// listener once; the caller guarantees the main thread and callback lifetime.
	unsafe {
		match api_version {
			16 => cpp_metamod_listen_levels_stable(api),
			18 => cpp_metamod_listen_levels_dev(api),
			_ => return HookStatus::UNSUPPORTED,
		}
	}
	HookStatus::INSTALLED
}

/// Configures Rust-owned state and selects the compiler-owned ISmmPlugin object
/// built against this plugin API version.
///
/// # Safety
///
/// Call on the server main thread. Callbacks and metadata strings must remain
/// valid until after Metamod unloads the plugin; do not reconfigure concurrently
/// with callbacks.
pub unsafe fn cpp_metamod_plugin(
	api_version: c_int,
	callbacks: *const PluginCallbacks,
	metadata: PluginMetadata,
) -> *mut c_void {
	if !crate::shell::configure(api_version, callbacks, metadata) {
		return std::ptr::null_mut();
	}
	cpp_metamod_plugin_for_version(api_version)
}

/// Returns the shell for a plugin API version without changing its state.
pub fn cpp_metamod_plugin_for_version(api_version: c_int) -> *mut c_void {
	static STABLE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
	static DEV: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
	// Initialize each compiler object exactly once; the private dispatch table has
	// library lifetime and no native caller can observe an uninitialized shell.
	match api_version {
		16 => *STABLE.get_or_init(|| unsafe {
			cpp_metamod_plugin_stable_instance(&crate::shell::DISPATCH) as usize
		}) as *mut c_void,

		18 => *DEV.get_or_init(|| unsafe {
			cpp_metamod_plugin_dev_instance(&crate::shell::DISPATCH) as usize
		}) as *mut c_void,

		_ => std::ptr::null_mut(),
	}
}

/// Whether Metamod has loaded this API version shell and not yet unloaded it.
/// The synchronized Rust state also permits observation outside native callbacks.
pub fn cpp_metamod_plugin_is_loaded(api_version: c_int) -> bool {
	crate::shell::status(api_version).loaded
}

/// A synchronized snapshot of the Rust-owned state of this plugin API shell.
pub fn cpp_metamod_plugin_status(api_version: c_int) -> PluginStatus {
	crate::shell::status(api_version)
}
