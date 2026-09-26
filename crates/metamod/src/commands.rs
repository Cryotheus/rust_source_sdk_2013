//! Console commands from `source_sdk_2013`, registered and run through
//! Metamod.

use crate::MetamodApi;
use crate::hooks::HookError;
use crate::sys::plugin as raw;
use source_sdk_2013::ServerBinding;
use source_sdk_2013::commands::{CommandRegistrar, UnlinksBeforeUnload, route_client_command};
use source_sdk_2013::interfaces::ServerGameClients;
use source_sdk_2013::sys::ConCommandBase;
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::{self, NonNull};

/// Registers console commands through `ISmmAPI::RegisterConCommandBase`.
///
/// Metamod unlinks every command a plugin registered after the plugin's
/// `Unload`, or after a refused `Load`, and before unloading its library, so
/// commands registered through it are always gone before their code is.
#[derive(Debug, Clone, Copy)]
pub struct MetamodRegistrar<'callback> {
	api: MetamodApi<'callback>,
	plugin: NonNull<c_void>,
}

// SAFETY: Both calls pass the command unchanged to the engine's `ICvar`, in
// 1.12 build 1226 and 2.0 build 1469 alike, and Metamod keeps it only to unlink
// it again.
unsafe impl CommandRegistrar for MetamodRegistrar<'_> {
	unsafe fn link(&self, command: NonNull<ConCommandBase>) {
		// SAFETY: `command_registrar` checked that Metamod loaded the plugin
		// object, and the caller passes a live command.
		unsafe {
			self.api
				.register_con_command_base(self.plugin, command.cast())
		};
	}

	unsafe fn unlink(&self, command: NonNull<ConCommandBase>) {
		// SAFETY: As for `link`.
		unsafe {
			self.api
				.unregister_con_command_base(self.plugin, command.cast())
		};
	}
}

// SAFETY: Metamod tracks each command under the plugin object it loaded, which
// `command_registrar` checked is the one passed, and unlinks the commands after
// `Unload` (forced or not) or a refused `Load`, before the library is unloaded.
unsafe impl UnlinksBeforeUnload for MetamodRegistrar<'_> {}

/// The server the client-command hook runs commands for.
struct RoutedServer(Cell<Option<ServerBinding>>);

// SAFETY: Only the server's main thread reaches it: the hook runs there, and
// `route_client_commands` takes a `MetamodApi`, which is confined to it.
unsafe impl Sync for RoutedServer {}

static ROUTED_SERVER: RoutedServer = RoutedServer(Cell::new(None));

impl<'callback> MetamodApi<'callback> {
	/// Registers commands for the plugin whose callback is running.
	///
	/// Returns `None` unless Metamod loaded this library's plugin shell, which
	/// is the object Metamod tracks registrations by.
	pub fn command_registrar(self) -> Option<MetamodRegistrar<'callback>> {
		let version = self.version().plugin_api_version();

		if !raw::cpp_metamod_plugin_is_loaded(version) {
			return None;
		}

		Some(MetamodRegistrar {
			api: self,
			plugin: NonNull::new(raw::cpp_metamod_plugin_for_version(version))?,
		})
	}

	/// Lets clients run the commands registered through
	/// `source_sdk_2013::commands` that accept them.
	///
	/// This hooks `IServerGameClients::ClientCommand` ahead of the game and
	/// passes each call to [`route_client_command`], which builds the handler's
	/// server from `binding`. The hook stops routing while the plugin is paused
	/// and when it unloads, and Metamod removes it after unloading the plugin.
	/// Install it while loading.
	pub fn route_client_commands(
		self,
		clients: ServerGameClients<'_>,
		binding: ServerBinding,
	) -> Result<(), HookError> {
		ROUTED_SERVER.0.set(Some(binding));

		// SAFETY: A `MetamodApi` only exists during a callback, on the main
		// thread. `clients` is the game's interface, and the callback is a
		// function of this library, which only reads a static.
		let status = unsafe {
			raw::cpp_metamod_hook_client_commands(
				self.version().plugin_api_version(),
				clients.as_ptr().cast(),
				route,
				ptr::from_ref(&ROUTED_SERVER).cast_mut().cast(),
			)
		};

		HookError::check(status)
	}
}

/// The shell's client-command callback.
unsafe extern "C" fn route(
	context: *mut c_void,
	edict: *mut c_void,
	command: *const c_void,
) -> bool {
	let (Some(edict), Some(command)) = (NonNull::new(edict), NonNull::new(command.cast_mut()))
	else {
		return false;
	};

	// SAFETY: `route_client_commands` passes a pointer to the static.
	let Some(binding) = (unsafe { &*context.cast::<RoutedServer>() }).0.get() else {
		return false;
	};

	// SAFETY: The shell calls this from its hook on
	// `IServerGameClients::ClientCommand`, ahead of the game, on the main thread,
	// with the engine's arguments. Panics are caught inside.
	unsafe { route_client_command(&binding, edict.cast(), command.cast()) }.is_handled()
}
