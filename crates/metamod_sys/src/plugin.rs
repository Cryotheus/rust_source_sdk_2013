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
	fn cpp_metamod_hook_game_frame_stable(
		game_dll: *mut c_void,
		callback: GameFrameCallback,
		context: *mut c_void,
	) -> c_int;
	fn cpp_metamod_hook_game_frame_dev(
		game_dll: *mut c_void,
		callback: GameFrameCallback,
		context: *mut c_void,
	) -> c_int;
	fn cpp_metamod_listen_levels_stable(
		init: Option<LevelInitCallback>,
		shutdown: Option<LevelShutdownCallback>,
		context: *mut c_void,
	) -> c_int;
	fn cpp_metamod_listen_levels_dev(
		init: Option<LevelInitCallback>,
		shutdown: Option<LevelShutdownCallback>,
		context: *mut c_void,
	) -> c_int;
	fn cpp_metamod_hook_net_messages_stable(
		handler: *mut c_void,
		slots: *const c_int,
		kinds: c_int,
		callback: NetMessageCallback,
		context: *mut c_void,
	) -> c_int;
	fn cpp_metamod_hook_net_messages_dev(
		handler: *mut c_void,
		slots: *const c_int,
		kinds: c_int,
		callback: NetMessageCallback,
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

/// Runs once per server frame, before the game's own frame: the argument of
/// `IServerGameDLL::GameFrame(bool simulating)`, and the context given to
/// [`cpp_metamod_hook_game_frame`].
pub type GameFrameCallback = unsafe extern "C" fn(context: *mut c_void, simulating: bool);

/// Metamod's `IMetamodListener::OnLevelInit`, with the name of the level
/// loading.
pub type LevelInitCallback = unsafe extern "C" fn(context: *mut c_void, map: *const c_char);

/// Metamod's `IMetamodListener::OnLevelShutdown`.
pub type LevelShutdownCallback = unsafe extern "C" fn(context: *mut c_void);

/// A net message from a client, before its handler processes it.
///
/// `kind` indexes the slots given to [`cpp_metamod_hook_net_messages`],
/// `handler` is the engine's message handler, which the method is called on,
/// and `message` its argument. Returns true to block the message, which the
/// engine then treats as processed.
pub type NetMessageCallback = unsafe extern "C" fn(
	context: *mut c_void,
	kind: c_int,
	handler: *mut c_void,
	message: *mut c_void,
) -> bool;

/// Hooks `IServerGameDLL::GameFrame` ahead of the game, passing each frame to
/// `callback` until Metamod unloads the plugin.
///
/// The shell stops calling `callback` when the plugin unloads or is paused,
/// and Metamod removes the hook after unloading the plugin.
///
/// # Safety
///
/// As for [`cpp_metamod_hook_client_commands`], with the game's
/// `IServerGameDLL` as `game_dll`.
pub unsafe fn cpp_metamod_hook_game_frame(
	api_version: c_int,
	game_dll: *mut c_void,
	callback: GameFrameCallback,
	context: *mut c_void,
) -> HookStatus {
	HookStatus(match api_version {
		16 => unsafe { cpp_metamod_hook_game_frame_stable(game_dll, callback, context) },
		18 => unsafe { cpp_metamod_hook_game_frame_dev(game_dll, callback, context) },
		_ => return HookStatus::UNSUPPORTED,
	})
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

/// Hooks the `Process*` methods at `slots` of the vtable `handler` shares with
/// every other object of its class, passing each call to `callback` before the
/// engine's handler runs, until Metamod unloads the plugin.
///
/// # Safety
///
/// As for [`cpp_metamod_hook_client_commands`]. `handler` must be a live
/// object whose vtable's methods at `slots` each take one pointer and return
/// `bool`, and at most 14 slots may be given.
pub unsafe fn cpp_metamod_hook_net_messages(
	api_version: c_int,
	handler: *mut c_void,
	slots: &[c_int],
	callback: NetMessageCallback,
	context: *mut c_void,
) -> HookStatus {
	let Ok(kinds) = c_int::try_from(slots.len()) else {
		return HookStatus::INVALID_ARGUMENT;
	};

	HookStatus(match api_version {
		16 => unsafe {
			cpp_metamod_hook_net_messages_stable(handler, slots.as_ptr(), kinds, callback, context)
		},
		18 => unsafe {
			cpp_metamod_hook_net_messages_dev(handler, slots.as_ptr(), kinds, callback, context)
		},
		_ => return HookStatus::UNSUPPORTED,
	})
}

const _: () = {
	let slot = size_of::<*const ()>();

	assert!(size_of::<PluginCallbacks>() == 6 * slot);
	assert!(size_of::<PluginMetadata>() == 8 * slot);
};
