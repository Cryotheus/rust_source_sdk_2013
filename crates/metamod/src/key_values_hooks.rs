//! A hook on the commands clients send as key values, which can block each
//! one before the game handles it.
//!
//! Clients send some commands as key values rather than as text, such as
//! TF2's `AchievementEarned`, after which the game announces the achievement
//! to every player, and Mann vs. Machine's upgrades. The engine reads each
//! into key values, which it owns and frees after the call, and hands them to
//! the game's `IServerGameClients::ClientCommandKeyValues`. The hook patches
//! that method on the game's interface, so the callback sees each command
//! before the game does, named by [`KeyValues::name`].
//!
//! # When to install
//!
//! The interface lives as long as the game library, so install while loading.
//! The hook stops calling back while the plugin is paused and when it unloads,
//! and Metamod removes it after unloading the plugin.
//!
//! # Blocking
//!
//! A blocked command does not reach the game: the hook supersedes the call.
//! The engine still frees the key values, so a block leaves nothing to clean
//! up, and other plugins' hooks see the command whole.
//!
//! A command that a hook running before this one blocked, such as SourceMod's
//! for a command a SourceMod plugin handled, reaches no callback. With
//! Metamod 2.0, KHook reports nothing of other plugins' hooks, so the callback
//! runs for those too.
//!
//! # What gets through
//!
//! As with other Metamod hooks, the callback runs only for commands handled on
//! the server's main thread, and not while the plugin is paused or after it
//! unloads, so commands sent then reach the game. With Metamod 2.0, when
//! `ClientCommandKeyValues` is already detoured by another plugin, KHook adds
//! the hook from its worker thread, so the commands just after an install can
//! pass unseen.

#[cfg(test)]
#[path = "tests/key_values_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use source_sdk_2013::edicts::Edict;
use source_sdk_2013::interfaces::ServerGameClients;
use source_sdk_2013::key_values::KeyValues;

use source_sdk_2013::raw::interfaces::server_game_clients::{
	CLIENT_COMMAND_KEY_VALUES_SLOT, ClientCommandKeyValuesFn as ClientCommandKeyValues,
};

use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ptr::NonNull;

/// Decides whether a command a client sent as key values reaches the game. It
/// gets the client's edict and the command, whose [name](KeyValues::name) says
/// which it is. A panic is contained by the hook dispatcher, and lets the
/// command through.
pub type ClientKeyValuesFn =
	for<'s> fn(Server<'s>, Edict<'s>, KeyValues<'s>) -> ClientKeyValuesAction;

/// `IServerGameClients::ClientCommandKeyValues`.
const CLIENT_COMMAND_KEY_VALUES: VirtualFunction<ClientCommandKeyValues> =
	VirtualFunction::new(CLIENT_COMMAND_KEY_VALUES_SLOT);

static ROUTE: KeyValuesRoute = KeyValuesRoute(Cell::new(None));

/// What to do with a command a client sent as key values.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ClientKeyValuesAction {
	/// Let the game handle it.
	#[default]
	Continue,

	/// Keep it from the game; see the
	/// [module documentation](crate::key_values_hooks#blocking).
	Block,
}

/// The callback the hook runs, with the hook.
struct KeyValuesRoute(Cell<Option<RoutedKeyValues>>);

impl Handler<ClientCommandKeyValues> for KeyValuesRoute {
	fn call(&self, call: &HookCall<'_, ClientCommandKeyValues>) -> HookAction<()> {
		// An earlier hook, such as SourceMod's, blocked it.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let (edict, key_values) = call.args();

		let (Some(routed), Some(edict), Some(key_values)) =
			(self.0.get(), NonNull::new(edict), NonNull::new(key_values))
		else {
			return HookAction::Ignore;
		};

		let scope = ();

		// SAFETY: The hook dispatcher runs on the main thread, during one call
		// from the engine. The plugin supplied the binding with the hook.
		let server = unsafe { routed.binding.server(&scope) };

		// SAFETY: The engine passes the client's slot of its edict table.
		let edict = unsafe { Edict::from_live(server, edict) };

		// SAFETY: The engine made the key values, laid out as TF2's, which it
		// neither renames nor frees during the call.
		let key_values = unsafe { KeyValues::from_raw(key_values) };

		match (routed.callback)(server, edict, key_values) {
			ClientKeyValuesAction::Continue => HookAction::Ignore,
			ClientKeyValuesAction::Block => HookAction::Supersede(()),
		}
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread.
unsafe impl Sync for KeyValuesRoute {}

#[derive(Clone, Copy)]
struct RoutedKeyValues {
	binding: ServerBinding,
	callback: ClientKeyValuesFn,
	hook: HookId,
}

impl MetamodApi<'_> {
	/// Runs `callback` before the game handles each command a client sends as
	/// key values, which decides whether the game gets it; see the
	/// [module documentation](crate::key_values_hooks).
	///
	/// This hooks `IServerGameClients::ClientCommandKeyValues` before the
	/// call. Installing again while the hook is installed returns
	/// [`HookError::AlreadyInstalled`].
	pub fn hook_client_key_values(
		self,
		clients: ServerGameClients<'_>,
		binding: ServerBinding,
		callback: ClientKeyValuesFn,
	) -> Result<(), HookError> {
		let clients = NonNull::new(clients.as_ptr()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: `clients` is the game's interface, which outlives the plugin,
		// and has `ClientCommandKeyValues` at the slot.
		unsafe { self.install_client_key_values(clients, binding, callback) }
	}

	/// Hooks `ClientCommandKeyValues` on `clients`.
	///
	/// # Safety
	///
	/// `clients` must be live, and its vtable must hold a function of the
	/// signature [`ClientCommandKeyValues`] at
	/// [`CLIENT_COMMAND_KEY_VALUES_SLOT`], until Metamod unloads the plugin.
	unsafe fn install_client_key_values(
		self,
		clients: NonNull<sys::IServerGameClients>,
		binding: ServerBinding,
		callback: ClientKeyValuesFn,
	) -> Result<(), HookError> {
		if ROUTE
			.0
			.get()
			.is_some_and(|routed| self.has_hook(routed.hook))
		{
			return Err(HookError::AlreadyInstalled);
		}

		// SAFETY: As the caller promises; a `MetamodApi` only exists on the main
		// thread.
		let hook = unsafe {
			self.add_hook(
				CLIENT_COMMAND_KEY_VALUES,
				HookTarget::instance(clients),
				HookTiming::Pre,
				&ROUTE,
			)
		}?;

		ROUTE.0.set(Some(RoutedKeyValues {
			binding,
			callback,
			hook,
		}));
		Ok(())
	}
}
