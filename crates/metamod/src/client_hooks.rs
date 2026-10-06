//! Callbacks as each client's player is put in the server and enters the
//! game, through hooks after the game's `IServerGameClients` methods.

#[cfg(test)]
#[path = "tests/client_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::edicts::Edict;
use source_sdk_2013::interfaces::ServerGameClients;

use source_sdk_2013::raw::interfaces::server_game_clients::{
	CLIENT_ACTIVE_SLOT, CLIENT_PUT_IN_SERVER_SLOT, ClientActiveFn as ClientActive,
	ClientPutInServerFn as ClientPutInServer,
};

use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ptr::NonNull;

/// A callback-scoped server and the edict of a client's player. A panic is
/// contained by the hook dispatcher.
pub type ClientFn = for<'s> fn(Server<'s>, Edict<'s>);

/// `IServerGameClients::ClientActive`, which has a client's player enter the
/// game.
const CLIENT_ACTIVE: VirtualFunction<ClientActive> = VirtualFunction::new(CLIENT_ACTIVE_SLOT);

/// `IServerGameClients::ClientPutInServer`, which creates a client's player.
const CLIENT_PUT_IN_SERVER: VirtualFunction<ClientPutInServer> =
	VirtualFunction::new(CLIENT_PUT_IN_SERVER_SLOT);

static CLIENTS: ClientRoute = ClientRoute(Cell::new(None));

/// The game's notifications about each client's player, after the game handled
/// them.
///
/// The engine notifies the game for every client, bots and SourceTV included,
/// and again for each client still connected after a level change, which
/// reconnects everyone.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClientEvents {
	/// Called after `IServerGameClients::ClientPutInServer`. The game created
	/// the client's player entity there, of its final class, and has not
	/// spawned it yet.
	pub put_in_server: Option<ClientFn>,

	/// Called after `IServerGameClients::ClientActive`, once the client finished
	/// connecting and its player entered the game. TF2 spawns the player there,
	/// the first time with its `InitialSpawn`
	/// (`game/server/tf/tf_client.cpp:52-108`), so this runs after that spawn.
	pub active: Option<ClientFn>,
}

/// The callbacks and the server they run for, kept for the hooks that run
/// them.
struct ClientRoute(Cell<Option<RoutedClients>>);

impl ClientRoute {
	/// Whether the route's hooks are installed, for this load of the plugin.
	fn installed(&self, api: MetamodApi<'_>) -> bool {
		self.0.get().is_some_and(|routed| {
			routed
				.hooks
				.into_iter()
				.flatten()
				.any(|hook| api.has_hook(hook))
		})
	}
}

impl Handler<ClientActive> for ClientRoute {
	fn call(&self, call: &HookCall<'_, ClientActive>) -> HookAction<()> {
		let (edict, _load_game) = call.args();

		if let Some(routed) = self.0.get()
			&& let Some(callback) = routed.events.active
		{
			// SAFETY: The hook runs after the game's `ClientActive`, on the main
			// thread, with the engine's edict.
			unsafe { dispatch(routed.binding, callback, edict) };
		}

		HookAction::Ignore
	}
}

impl Handler<ClientPutInServer> for ClientRoute {
	fn call(&self, call: &HookCall<'_, ClientPutInServer>) -> HookAction<()> {
		let (edict, _name) = call.args();

		if let Some(routed) = self.0.get()
			&& let Some(callback) = routed.events.put_in_server
		{
			// SAFETY: The hook runs after the game's `ClientPutInServer`, on the
			// main thread, with the engine's edict.
			unsafe { dispatch(routed.binding, callback, edict) };
		}

		HookAction::Ignore
	}
}

// SAFETY: Only the server's main thread reaches it: hooks only run their
// handlers there, and `hook_client_events` takes a `MetamodApi`, which is
// confined to it.
unsafe impl Sync for ClientRoute {}

#[derive(Clone, Copy)]
struct RoutedClients {
	/// The hooks of `ClientPutInServer` and `ClientActive`, for the callbacks
	/// there are.
	hooks: [Option<HookId>; 2],
	binding: ServerBinding,
	events: ClientEvents,
}

impl MetamodApi<'_> {
	/// Passes the game's notifications about each client's player to
	/// `events`, after the game handled them.
	///
	/// This hooks `IServerGameClients::ClientPutInServer` and `ClientActive`
	/// after the call, for each callback `events` has. A hook runs even if
	/// another plugin's hook superseded the call. The hooks stop calling back
	/// while the plugin is paused and when it unloads, and Metamod removes them
	/// after unloading the plugin. Install them while loading. Under Metamod
	/// 2.0, when a function is already detoured, such as by another plugin,
	/// KHook adds the hook from a worker thread, so it may miss the next few
	/// calls (see [`crate::hook`]).
	pub fn hook_client_events(
		self,
		clients: ServerGameClients<'_>,
		binding: ServerBinding,
		events: ClientEvents,
	) -> Result<(), HookError> {
		if CLIENTS.installed(self) {
			return Err(HookError::AlreadyInstalled);
		}

		let clients = NonNull::new(clients.as_ptr()).ok_or(HookError::InvalidArgument)?;
		let mut hooks = [None; 2];

		let hooked = (|| -> Result<(), HookError> {
			if events.put_in_server.is_some() {
				// SAFETY: A `MetamodApi` only exists during a callback, on the main
				// thread. `clients` is the game's interface, which outlives the
				// plugin, and has `ClientPutInServer` at the slot.
				hooks[0] = Some(unsafe {
					self.add_hook(
						CLIENT_PUT_IN_SERVER,
						HookTarget::instance(clients),
						HookTiming::Post,
						&CLIENTS,
					)
				}?);
			}

			if events.active.is_some() {
				// SAFETY: As above, with `ClientActive` at its slot.
				hooks[1] = Some(unsafe {
					self.add_hook(
						CLIENT_ACTIVE,
						HookTarget::instance(clients),
						HookTiming::Post,
						&CLIENTS,
					)
				}?);
			}

			Ok(())
		})();

		if let Err(error) = hooked {
			for hook in hooks.into_iter().flatten() {
				self.remove_hook(hook);
			}

			return Err(error);
		}

		CLIENTS.0.set(Some(RoutedClients {
			hooks,
			binding,
			events,
		}));

		Ok(())
	}
}

/// Runs `callback` with a server for the current call from the engine, and the
/// edict `edict` points to, if any.
///
/// # Safety
///
/// The call must come from a hook of the engine's call of an
/// `IServerGameClients` method, on the main thread, with the edict it passed.
unsafe fn dispatch(binding: ServerBinding, callback: ClientFn, edict: *mut sys::edict_t) {
	let Some(edict) = NonNull::new(edict) else {
		return;
	};

	let scope = ();

	// SAFETY: As the caller promises, this runs during a single call from the
	// engine, on the main thread.
	let server = unsafe { binding.server(&scope) };

	// SAFETY: The engine passes a slot of its edict table.
	let edict = unsafe { Edict::from_live(server, edict) };

	callback(server, edict);
}
