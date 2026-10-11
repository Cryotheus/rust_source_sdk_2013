//! A hook on remote clients connecting, which may refuse them before they
//! join.

#[cfg(test)]
#[path = "../tests/hooks/connect.rs"]
mod tests;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use crate::{ErrorBuffer, MetamodApi};
use source_sdk_2013::edicts::Edict;
use source_sdk_2013::interfaces::ServerGameClients;

use source_sdk_2013::raw::interfaces::server_game_clients::{
	CLIENT_CONNECT_SLOT, ClientConnectFn as ClientConnect,
};

use source_sdk_2013::{Server, ServerBinding};
use std::borrow::Cow;
use std::cell::Cell;
use std::ffi::CStr;
use std::ptr::NonNull;

/// Decides whether a remote client may connect. A panic is contained by the
/// hook dispatcher, and leaves the client to the game.
pub type ClientConnectFn = for<'s> fn(Server<'s>, ConnectRequest<'_, 's>) -> ConnectAction;

/// `IServerGameClients::ClientConnect`, which asks the game whether a client
/// may connect.
const CLIENT_CONNECT: VirtualFunction<ClientConnect> = VirtualFunction::new(CLIENT_CONNECT_SLOT);

static CONNECTS: ConnectRoute = ConnectRoute(Cell::new(None));

/// What a [`ClientConnectFn`] does with a client asking to connect.
#[must_use]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ConnectAction {
	/// Leaves the client to the game, and to the hooks after this one.
	#[default]
	Allow,

	/// Refuses the client, which the engine disconnects with the reason. The
	/// game's own `ClientConnect` does not run. A reason starting with `#`
	/// names a localization token, as the engine's own refusals do, such as
	/// `#GameUI_ServerRejectServerFull`. One longer than the engine's buffer
	/// is cut short.
	Refuse(Cow<'static, CStr>),
}

/// A remote client asking to connect.
#[derive(Debug, Clone, Copy)]
pub struct ConnectRequest<'a, 's> {
	/// The edict of the player slot the engine gave the client, which is
	/// connected already. Its player entity does not exist yet.
	pub edict: Edict<'s>,

	/// The name the client asked to play under.
	pub name: &'a CStr,

	/// The client's network address, such as `192.0.2.1:27005`.
	pub address: &'a CStr,
}

/// The callback the hook runs, the server it runs for, and the hook.
struct ConnectRoute(Cell<Option<(HookId, ServerBinding, ClientConnectFn)>>);

impl Handler<ClientConnect> for ConnectRoute {
	fn call(&self, call: &HookCall<'_, ClientConnect>) -> HookAction<bool> {
		// An earlier hook, such as another plugin's, decided for the game.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let Some((_, binding, callback)) = self.0.get() else {
			return HookAction::Ignore;
		};

		let (edict, name, address, reject, reject_capacity) = call.args();

		let (Some(edict), false, false) = (NonNull::new(edict), name.is_null(), address.is_null())
		else {
			return HookAction::Ignore;
		};

		let scope = ();

		// SAFETY: The hook runs ahead of the game's `ClientConnect`, on the main
		// thread, with the engine's arguments: the edict of the client's player
		// slot, and its name and address, which last for the call.
		let (server, request) = unsafe {
			let server = binding.server(&scope);

			let request = ConnectRequest {
				edict: Edict::from_live(server, edict),
				name: CStr::from_ptr(name),
				address: CStr::from_ptr(address),
			};

			(server, request)
		};

		match callback(server, request) {
			ConnectAction::Allow => HookAction::Ignore,

			ConnectAction::Refuse(reason) => {
				let capacity = usize::try_from(reject_capacity).unwrap_or(0);

				// SAFETY: The engine passes a buffer of `reject_capacity` bytes it
				// reads the reason from once the call returns false, and nothing
				// else uses it during the call.
				unsafe { ErrorBuffer::from_raw(reject, capacity, &scope) }.write(&reason);

				HookAction::Supersede(false)
			}
		}
	}
}

// SAFETY: Only the server's main thread reaches it: hooks only run their
// handlers there, and `hook_client_connect` takes a `MetamodApi`, which is
// confined to it.
unsafe impl Sync for ConnectRoute {}

impl MetamodApi<'_> {
	/// Calls `callback` as each remote client connects, before the game
	/// decides whether it may, so that the callback can refuse it.
	///
	/// This hooks `IServerGameClients::ClientConnect` ahead of the game. Bots
	/// and other fake clients join without that call, so the callback never
	/// sees them. A client the callback allows can still be refused by the
	/// game, or by another plugin's hook. The hook stops calling back while
	/// the plugin is paused and when it unloads, and Metamod removes it after
	/// unloading the plugin. Install it while loading.
	pub fn hook_client_connect(
		self,
		clients: ServerGameClients<'_>,
		binding: ServerBinding,
		callback: ClientConnectFn,
	) -> Result<(), HookError> {
		if let Some((hook, ..)) = CONNECTS.0.get()
			&& self.has_hook(hook)
		{
			return Err(HookError::AlreadyInstalled);
		}

		let clients = NonNull::new(clients.as_ptr()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: A `MetamodApi` only exists during a callback, on the main
		// thread. `clients` is the game's interface, which outlives the plugin,
		// and has `ClientConnect` at the slot.
		let hook = unsafe {
			self.add_hook(
				CLIENT_CONNECT,
				HookTarget::instance(clients),
				HookTiming::Pre,
				&CONNECTS,
			)
		}?;

		CONNECTS.0.set(Some((hook, binding, callback)));
		Ok(())
	}
}
