//! C ABI shared with the small C++ `ISmmPlugin` shell.

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
	fn cpp_metamod_listen_levels_dev(
		init: Option<LevelInitCallback>,
		shutdown: Option<LevelShutdownCallback>,
		context: *mut c_void,
	) -> c_int;

	fn cpp_metamod_listen_levels_stable(
		init: Option<LevelInitCallback>,
		shutdown: Option<LevelShutdownCallback>,
		context: *mut c_void,
	) -> c_int;

	fn cpp_metamod_plugin_dev(
		callbacks: *const PluginCallbacks,
		metadata: PluginMetadata,
	) -> *mut c_void;

	fn cpp_metamod_plugin_dev_instance() -> *mut c_void;
	fn cpp_metamod_plugin_dev_is_loaded() -> bool;
	fn cpp_metamod_plugin_dev_status() -> PluginStatus;

	fn cpp_metamod_plugin_stable(
		callbacks: *const PluginCallbacks,
		metadata: PluginMetadata,
	) -> *mut c_void;

	fn cpp_metamod_plugin_stable_instance() -> *mut c_void;
	fn cpp_metamod_plugin_stable_is_loaded() -> bool;
	fn cpp_metamod_plugin_stable_status() -> PluginStatus;
}

/// Registers a Metamod listener passing level notifications to the callbacks
/// until Metamod unloads the plugin.
///
/// The shell stops calling them when the plugin unloads or is paused, and
/// Metamod removes the listener after unloading the plugin.
///
/// # Safety
///
/// Call it on the server's main thread while Metamod runs one of the shell's
/// callbacks, from `Load` on. The callbacks must stay callable with `context`
/// for as long as the library is loaded. They are called on the main thread.
pub unsafe fn cpp_metamod_listen_levels(
	api_version: c_int,
	init: Option<LevelInitCallback>,
	shutdown: Option<LevelShutdownCallback>,
	context: *mut c_void,
) -> HookStatus {
	HookStatus(match api_version {
		16 => unsafe { cpp_metamod_listen_levels_stable(init, shutdown, context) },
		18 => unsafe { cpp_metamod_listen_levels_dev(init, shutdown, context) },
		_ => return HookStatus::UNSUPPORTED,
	})
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

/// Whether Metamod has loaded the shell for a plugin API version and not yet
/// unloaded it.
///
/// Metamod tracks what a plugin registers by the object it loaded, from before
/// `Load` until after `Unload`, or after a refused `Load`.
pub fn cpp_metamod_plugin_is_loaded(api_version: c_int) -> bool {
	match api_version {
		16 => unsafe { cpp_metamod_plugin_stable_is_loaded() },
		18 => unsafe { cpp_metamod_plugin_dev_is_loaded() },
		_ => false,
	}
}

/// The state of the shell for a plugin API version.
pub fn cpp_metamod_plugin_status(api_version: c_int) -> PluginStatus {
	match api_version {
		16 => unsafe { cpp_metamod_plugin_stable_status() },
		18 => unsafe { cpp_metamod_plugin_dev_status() },
		_ => PluginStatus::UNLOADED,
	}
}

/// The listener's notification generation, including notifications while paused.
/// Starts at zero on registration and advances for each init or shutdown;
/// unavailable before registration, after unload, for unsupported APIs or
/// counter exhaustion. It contains no map pointers and resets on a new load.
///
/// # Safety
///
/// Call only on the server's main thread during a live shell callback. The
/// listener and getter use ordinary non-atomic state on that thread.
pub unsafe fn cpp_metamod_level_generation(api_version: c_int) -> Option<u64> {
	let mut generation = 0;
	let found = match api_version {
		16 => unsafe { cpp_metamod_level_generation_stable(&mut generation) },
		18 => unsafe { cpp_metamod_level_generation_dev(&mut generation) },
		_ => return None,
	};
	found.then_some(generation)
}

unsafe extern "C" {
	fn cpp_metamod_level_generation_dev(generation: *mut u64) -> bool;
	fn cpp_metamod_level_generation_stable(generation: *mut u64) -> bool;
}
