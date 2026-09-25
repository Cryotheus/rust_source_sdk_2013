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
	fn cpp_metamod_plugin_stable_is_loaded() -> bool;
	fn cpp_metamod_plugin_dev_is_loaded() -> bool;
	fn cpp_metamod_hook_client_commands_stable(
		clients: *mut c_void,
		callback: ClientCommandCallback,
		context: *mut c_void,
	) -> c_int;
	fn cpp_metamod_hook_client_commands_dev(
		clients: *mut c_void,
		callback: ClientCommandCallback,
		context: *mut c_void,
	) -> c_int;
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

/// Runs a client's string command: the arguments of
/// `IServerGameClients::ClientCommand(edict_t *, const CCommand &)`, and the
/// context given to [`cpp_metamod_hook_client_commands`].
///
/// Returns true if the command was handled, which keeps it from the game.
pub type ClientCommandCallback =
	unsafe extern "C" fn(context: *mut c_void, edict: *mut c_void, command: *const c_void) -> bool;

/// The result of [`cpp_metamod_hook_client_commands`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct HookStatus(pub c_int);

impl HookStatus {
	pub const INSTALLED: Self = Self(0);

	/// The shell is not between `Load` and `Unload`, or Metamod supplied no
	/// hooking library.
	pub const NOT_BOUND: Self = Self(1);

	pub const ALREADY_INSTALLED: Self = Self(2);
	pub const INVALID_ARGUMENT: Self = Self(3);

	/// The hooking library refused the hook.
	pub const REFUSED: Self = Self(4);

	/// No shell exists for the plugin API version.
	pub const UNSUPPORTED: Self = Self(-1);
}

/// Hooks `IServerGameClients::ClientCommand` ahead of the game, passing each
/// call to `callback` until Metamod unloads the plugin.
///
/// The shell stops calling `callback` when the plugin unloads or is paused,
/// and Metamod removes the hook after unloading the plugin.
///
/// # Safety
///
/// Call it on the server's main thread while Metamod runs one of the shell's
/// callbacks, from `Load` on. `clients` must be the game's `IServerGameClients`,
/// and `callback(context, ...)` must stay callable for as long as the library
/// is loaded. The hook calls it on the main thread.
pub unsafe fn cpp_metamod_hook_client_commands(
	api_version: c_int,
	clients: *mut c_void,
	callback: ClientCommandCallback,
	context: *mut c_void,
) -> HookStatus {
	HookStatus(match api_version {
		16 => unsafe { cpp_metamod_hook_client_commands_stable(clients, callback, context) },
		18 => unsafe { cpp_metamod_hook_client_commands_dev(clients, callback, context) },
		_ => return HookStatus::UNSUPPORTED,
	})
}

const _: () = {
	let slot = size_of::<*const ()>();

	assert!(size_of::<PluginCallbacks>() == 6 * slot);
	assert!(size_of::<PluginMetadata>() == 8 * slot);
};
