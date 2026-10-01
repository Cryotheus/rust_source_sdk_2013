//! Console commands from `source_sdk_2013`, registered and run through
//! Metamod.

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use crate::sys::plugin as raw;
use source_sdk_2013::ServerBinding;
use source_sdk_2013::commands::{CommandRegistrar, UnlinksBeforeUnload, route_client_command};
use source_sdk_2013::interfaces::ServerGameClients;
use source_sdk_2013::sys::{self, ConCommandBase};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

/// `void IServerGameClients::ClientCommand(edict_t *, const CCommand &)`.
type ClientCommand =
	unsafe extern "C" fn(*mut sys::IServerGameClients, *mut sys::edict_t, *const sys::CCommand);

/// `IServerGameClients` declares no virtual destructor, so `ClientCommand` has
/// this slot under the MSVC and Itanium ABIs alike.
const CLIENT_COMMAND: VirtualFunction<ClientCommand> = VirtualFunction::new(5);

static ROUTED_SERVER: RoutedServer = RoutedServer(Cell::new(None));

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
// 1.12 build 1226 and 2.0 builds 1469 through 1472 alike, and Metamod keeps it
// only to unlink it again.
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

/// The server the client-command hook runs commands for, with the hook.
struct RoutedServer(Cell<Option<(HookId, ServerBinding)>>);

impl Handler<ClientCommand> for RoutedServer {
	fn call(&self, call: &HookCall<'_, ClientCommand>) -> HookAction<()> {
		// An earlier hook, such as a SourceMod command listener, blocked it.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let (edict, command) = call.args();

		let (Some((_, binding)), Some(edict), Some(command)) = (
			self.0.get(),
			NonNull::new(edict),
			NonNull::new(command.cast_mut()),
		) else {
			return HookAction::Ignore;
		};

		// SAFETY: The hook runs ahead of the game's `ClientCommand`, on the main
		// thread, with the engine's arguments. Panics are caught inside.
		match unsafe { route_client_command(&binding, edict, command) }.is_handled() {
			true => HookAction::Supersede(()),
			false => HookAction::Ignore,
		}
	}
}

// SAFETY: Only the server's main thread reaches it: hooks only run their
// handlers there, and `route_client_commands` takes a `MetamodApi`, which is
// confined to it.
unsafe impl Sync for RoutedServer {}

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
		if let Some((hook, _)) = ROUTED_SERVER.0.get()
			&& self.has_hook(hook)
		{
			return Err(HookError::AlreadyInstalled);
		}

		let clients = NonNull::new(clients.as_ptr()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: A `MetamodApi` only exists during a callback, on the main
		// thread. `clients` is the game's interface, which outlives the plugin,
		// and has `ClientCommand` at the slot.
		let hook = unsafe {
			self.add_hook(
				CLIENT_COMMAND,
				HookTarget::instance(clients),
				HookTiming::Pre,
				&ROUTED_SERVER,
			)
		}?;

		ROUTED_SERVER.0.set(Some((hook, binding)));
		Ok(())
	}
}
