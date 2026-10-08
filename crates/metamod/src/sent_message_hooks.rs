//! Hooks on what the server sends players that never reaches its own console:
//! user messages, such as the game's `TextMsg`, `SayText`, `SayText2` and
//! `HintText`, and the text `IVEngineServer::ClientPrintf` prints to a
//! client's console.
//!
//! The hooks only observe. They never change, block or delay what is sent,
//! and the listeners return nothing.
//!
//! # User messages
//!
//! The game sends a user message through `IVEngineServer` in three steps:
//! `UserMessageBegin` takes the recipient filter and the message's type and
//! returns a buffer, the game writes the payload to that buffer, and
//! `MessageEnd` sends it. A hook after `UserMessageBegin` notes the
//! recipients, the type and the buffer the game got, and a hook before
//! `MessageEnd` passes the listener the message with the payload written to
//! that buffer, as a [`SentUserMessage`].
//!
//! The engine has not sent the message yet then, and still has it open. It
//! has one message open at a time, so the listener must not begin another
//! user or entity message, such as with
//! [`user_messages::send`](source_sdk_2013::user_messages::send): send it from
//! a later frame instead.
//!
//! Entity messages, which `EntityMessageBegin` begins, end with the same
//! `MessageEnd`. A hook after `EntityMessageBegin` notes that the open message
//! is not a user message, so that its end reaches no listener.
//!
//! # Client prints
//!
//! A hook before `IVEngineServer::ClientPrintf` passes the other listener the
//! client's edict and the text, before the engine sends it. A print the
//! listener makes through the engine reaches it again, within its own call.
//!
//! # What gets through
//!
//! The listeners see what the game and plugins send through the engine's
//! `IVEngineServer`, such as with
//! [`user_messages::send`](source_sdk_2013::user_messages::send) and
//! [`ValveEngine::client_print`]. Messages to bots are seen too, though the
//! engine sends fake clients nothing: a user message lists them among its
//! recipients, and a print to a bot reaches the client print listener. A
//! message that other plugins' hooks on these functions block or replace, as
//! SourceMod's user message hooks can, still reaches the listener, with the
//! payload the game wrote.
//!
//! The listeners do not see:
//!
//! - text the engine prints to a client itself, through the client's
//!   `IClient::ClientPrintf`, which sends it as an `SVC_Print` message, nor
//!   [`GameClient::print`](source_sdk_2013::interfaces::GameClient::print),
//!   which calls that method directly;
//! - messages sent through a client's channel, such as with
//!   [`net::messages::UserMessage`](source_sdk_2013::net::messages::UserMessage);
//! - messages a plugin sends by calling the engine's functions past every
//!   plugin's hooks, as SourceMod does for user messages sent with
//!   `USERMSG_BLOCKHOOKS`;
//! - a user message whose payload overflowed its buffer;
//! - as with other Metamod hooks, messages sent off the server's main thread,
//!   or while the plugin is paused or after it unloads.
//!
//! # When to install
//!
//! The engine's interface lives as long as the engine, so install while
//! loading. Metamod disables the hooks while the plugin is paused, and removes
//! them before it unloads. With Metamod 2.0, when one of the functions is
//! already detoured, such as by SourceMod, KHook adds the hook from its worker
//! thread, so the messages just after an install can pass unseen.

#[cfg(test)]
#[path = "tests/sent_message_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use crate::server_hooks::with_server;
use source_sdk_2013::bitbuf::{BitReader, BitWriter};
use source_sdk_2013::edicts::Edict;
use source_sdk_2013::interfaces::ValveEngine;
use source_sdk_2013::raw::bitbuf::BfWrite;

use source_sdk_2013::raw::interfaces::valve_engine::{
	CLIENT_PRINTF_SLOT, ClientPrintfFn as ClientPrintf, ENTITY_MESSAGE_BEGIN_SLOT,
	EntityMessageBeginFn as EntityMessageBegin, MESSAGE_END_SLOT, MessageEndFn as MessageEnd,
	USER_MESSAGE_BEGIN_SLOT, UserMessageBeginFn as UserMessageBegin,
};

use source_sdk_2013::raw::util::cstr::borrow_cstr;
use source_sdk_2013::raw::vcall;
use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::{CStr, c_int};
use std::ptr::NonNull;

/// Observes text the server prints to a client's console, with a
/// callback-scoped server, the client's edict, and the text as the engine
/// sends it. A panic is caught.
///
/// The listener runs before the engine checks the edict: the engine prints
/// nothing for an edict that no connected client owns, nor for a bot.
pub type ClientPrintFn = for<'s> fn(Server<'s>, Edict<'s>, &CStr);

/// Observes a user message the server is sending, with a callback-scoped
/// server. A panic is caught.
///
/// The engine still has the message open, so the listener must not begin
/// another; see the
/// [module documentation](crate::sent_message_hooks#user-messages).
pub type SentUserMessageFn = for<'s> fn(Server<'s>, &SentUserMessage<'_>);

/// `IVEngineServer::ClientPrintf`.
const CLIENT_PRINTF: VirtualFunction<ClientPrintf> = VirtualFunction::new(CLIENT_PRINTF_SLOT);

/// `IVEngineServer::EntityMessageBegin`.
const ENTITY_MESSAGE_BEGIN: VirtualFunction<EntityMessageBegin> =
	VirtualFunction::new(ENTITY_MESSAGE_BEGIN_SLOT);

/// `IVEngineServer::MessageEnd`.
const MESSAGE_END: VirtualFunction<MessageEnd> = VirtualFunction::new(MESSAGE_END_SLOT);

/// `IVEngineServer::UserMessageBegin`.
const USER_MESSAGE_BEGIN: VirtualFunction<UserMessageBegin> =
	VirtualFunction::new(USER_MESSAGE_BEGIN_SLOT);

static ROUTE: SentMessageRoute = SentMessageRoute::new();

/// A user message the engine began and has not ended.
struct OpenUserMessage {
	/// The message's type.
	id: c_int,

	/// The buffer the game writes the payload to.
	buffer: NonNull<sys::bf_write>,

	/// The entity index of each client the filter listed.
	recipients: Vec<c_int>,

	/// Whether the filter sends reliably.
	reliable: bool,

	/// Whether the filter makes an init message.
	init: bool,
}

impl OpenUserMessage {
	/// The message of type `id` to the clients `filter` lists, whose payload
	/// the game writes to `buffer`.
	///
	/// # Safety
	///
	/// `filter` must be a live `IRecipientFilter`, and the call must come from
	/// a hook on the server's main thread.
	unsafe fn new(
		id: c_int,
		filter: NonNull<sys::IRecipientFilter>,
		buffer: NonNull<sys::bf_write>,
	) -> Self {
		let filter = filter.as_ptr().cast_const();

		// SAFETY: As the caller promises. The slots read are below the count.
		unsafe {
			let count = vcall!(filter => IRecipientFilter_GetRecipientCount());

			Self {
				id,
				buffer,
				recipients: (0..count)
					.map(|slot| vcall!(filter => IRecipientFilter_GetRecipientIndex(slot)))
					.collect(),
				reliable: vcall!(filter => IRecipientFilter_IsReliable()),
				init: vcall!(filter => IRecipientFilter_IsInitMessage()),
			}
		}
	}
}

#[derive(Clone, Copy)]
struct RoutedSentMessages {
	binding: ServerBinding,
	hooks: SentMessageHooks,
	listeners: SentMessageListeners,
}

/// The hooks [`MetamodApi::listen_sent_messages`] installs.
///
/// Metamod disables them while the plugin is paused, and removes them before
/// it unloads. [`Self::remove`] stops them earlier.
#[must_use = "retain the hooks to remove them"]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SentMessageHooks {
	/// The hooks of `MessageEnd`, `EntityMessageBegin`, `UserMessageBegin` and
	/// `ClientPrintf`, in that order, for the listeners there are.
	hooks: [Option<HookId>; 4],
}

impl SentMessageHooks {
	/// Stops the hooks, so no further message reaches the listeners.
	pub fn remove(self, api: MetamodApi<'_>) {
		for hook in self.hooks.into_iter().flatten() {
			api.remove_hook(hook);
		}

		// Hooks installed since these were removed keep their listeners.
		if ROUTE.state.get().is_some_and(|routed| routed.hooks == self) {
			ROUTE.state.set(None);
			ROUTE.open.take();
		}
	}
}

/// The listeners [`MetamodApi::listen_sent_messages`] passes what the server
/// sends players to. Only the hooks of the listeners that are set are
/// installed.
#[derive(Debug, Clone, Copy, Default)]
pub struct SentMessageListeners {
	/// Called with each user message, before the engine sends it.
	pub user_message: Option<SentUserMessageFn>,

	/// Called with each text printed to a client's console through
	/// `IVEngineServer::ClientPrintf`, before the engine sends it.
	pub client_print: Option<ClientPrintFn>,
}

/// The listeners the hooks run, and the user message the engine has open.
struct SentMessageRoute {
	state: Cell<Option<RoutedSentMessages>>,

	/// The user message from its `UserMessageBegin` until its `MessageEnd`, or
	/// until an entity message begins.
	open: Cell<Option<OpenUserMessage>>,
}

impl SentMessageRoute {
	const fn new() -> Self {
		Self {
			state: Cell::new(None),
			open: Cell::new(None),
		}
	}

	/// Whether the route's hooks are installed, for this load of the plugin.
	fn installed(&self, api: MetamodApi<'_>) -> bool {
		self.state.get().is_some_and(|routed| {
			routed
				.hooks
				.hooks
				.into_iter()
				.flatten()
				.any(|hook| api.has_hook(hook))
		})
	}
}

impl Handler<ClientPrintf> for SentMessageRoute {
	/// Passes the client and the text to the listener, before the engine
	/// prints it.
	fn call(&self, call: &HookCall<'_, ClientPrintf>) -> HookAction<()> {
		let (client, message) = call.args();

		// SAFETY: The caller passes null or terminated text, which outlives the
		// call.
		let message = unsafe { borrow_cstr(message) };

		if let Some(routed) = self.state.get()
			&& let Some(listener) = routed.listeners.client_print
			&& let (Some(client), Some(message)) = (NonNull::new(client), message)
		{
			with_server(routed.binding, |server| {
				// SAFETY: The caller passes a slot of the engine's edict table, which
				// outlives the call.
				let client = unsafe { Edict::from_live(server, client) };

				listener(server, client, message);
			});
		}

		HookAction::Ignore
	}
}

impl Handler<EntityMessageBegin> for SentMessageRoute {
	/// Forgets any user message: the message the engine began is an entity
	/// message, whose end reaches no listener.
	fn call(&self, _call: &HookCall<'_, EntityMessageBegin>) -> HookAction<*mut sys::bf_write> {
		self.open.take();
		HookAction::Ignore
	}
}

impl Handler<MessageEnd> for SentMessageRoute {
	/// Passes the open user message to the listener, before the engine sends
	/// it, and forgets it.
	fn call(&self, _call: &HookCall<'_, MessageEnd>) -> HookAction<()> {
		// The message ends here, whatever the listener does.
		let (Some(message), Some(routed)) = (self.open.take(), self.state.get()) else {
			return HookAction::Ignore;
		};

		let Some(listener) = routed.listeners.user_message else {
			return HookAction::Ignore;
		};

		// SAFETY: The hook runs before `MessageEnd`, on the main thread, so the
		// buffer the game got from `UserMessageBegin` still holds the payload,
		// and its owner, the engine or the hook that supplied it, keeps it until
		// the message ends. Nothing writes to it during the call. An overflowed
		// payload reads as `None`.
		let Some(bits) = (unsafe { BfWrite::read_back(BfWrite::from_sys(message.buffer)) }) else {
			return HookAction::Ignore;
		};

		let data = BitWriter::from(bits);
		let bytes = data.to_bytes();

		with_server(routed.binding, |server| {
			let name = usize::try_from(message.id)
				.ok()
				.and_then(|index| server.server_game_dll().ok()?.user_message(index))
				.map(|registered| registered.name);

			listener(
				server,
				&SentUserMessage {
					id: message.id,
					name: name.as_deref(),
					recipients: &message.recipients,
					reliable: message.reliable,
					init: message.init,
					data: &data,
					bytes: &bytes,
				},
			);
		});

		HookAction::Ignore
	}
}

impl Handler<UserMessageBegin> for SentMessageRoute {
	/// Notes the user message the engine began, with its recipients and the
	/// buffer the game writes its payload to.
	fn call(&self, call: &HookCall<'_, UserMessageBegin>) -> HookAction<*mut sys::bf_write> {
		let (filter, id) = call.args();

		// The buffer the game got, which a hook that superseded the engine's
		// function may have supplied.
		let buffer = call.return_value().and_then(NonNull::new);

		let message = match (self.state.get(), NonNull::new(filter), buffer) {
			(Some(_), Some(filter), Some(buffer)) => {
				// SAFETY: The hook runs after `UserMessageBegin`, on the main thread,
				// with the game's filter, which outlives the message.
				Some(unsafe { OpenUserMessage::new(id, filter, buffer) })
			}

			// Without a buffer, there is no payload to pass on.
			_ => None,
		};

		self.open.set(message);
		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread. The cells' contents are never borrowed
// over calls.
unsafe impl Sync for SentMessageRoute {}

/// A user message the server is sending, as a [`SentUserMessageFn`] sees it
/// before the engine sends it. Valid only for the call.
#[doc(alias("UserMessageBegin", "MessageEnd"))]
#[derive(Debug, Clone, Copy)]
pub struct SentUserMessage<'a> {
	id: c_int,
	name: Option<&'a CStr>,
	recipients: &'a [c_int],
	reliable: bool,
	init: bool,
	data: &'a BitWriter,
	bytes: &'a [u8],
}

impl<'a> SentUserMessage<'a> {
	/// The number of bits in the payload.
	#[doc(alias("GetNumBitsWritten"))]
	pub fn bit_len(&self) -> usize {
		self.data.len()
	}

	/// The payload, as bytes. Bits past [`bit_len`](Self::bit_len) in the last
	/// byte are zero.
	pub fn bytes(&self) -> &'a [u8] {
		self.bytes
	}

	/// The payload as the game wrote it, which
	/// [`RawUserMessage`](source_sdk_2013::user_messages::RawUserMessage)
	/// takes to send it again.
	pub fn data(&self) -> &'a BitWriter {
		self.data
	}

	/// The message's type: the index the game registered it at, as
	/// `UserMessageBegin` got it.
	pub fn id(&self) -> c_int {
		self.id
	}

	/// Whether the message is an init message, which the engine adds to the
	/// level's signon data, which each client gets as it connects, rather than
	/// sending it to the recipients.
	#[doc(alias("IsInitMessage"))]
	pub fn is_init_message(&self) -> bool {
		self.init
	}

	/// Whether the engine sends the message in each client's reliable stream.
	#[doc(alias("IsReliable"))]
	pub fn is_reliable(&self) -> bool {
		self.reliable
	}

	/// The name the game registered the message under, such as `SayText2`,
	/// from [`ServerGameDll::user_message`], or `None` if the game registered
	/// no message of this type, or its `IServerGameDLL` is missing.
	///
	/// [`ServerGameDll::user_message`]: source_sdk_2013::interfaces::ServerGameDll::user_message
	pub fn name(&self) -> Option<&'a CStr> {
		self.name
	}

	/// Reads the payload from its start.
	pub fn reader(&self) -> BitReader<'a> {
		self.data.reader()
	}

	/// The entity index of each client the message is addressed to, in the
	/// order the game's recipient filter listed them as the message began,
	/// bots included.
	#[doc(alias("GetRecipientIndex"))]
	pub fn recipients(&self) -> &'a [c_int] {
		self.recipients
	}
}

impl MetamodApi<'_> {
	/// Hooks the functions `listeners` need on `engine`.
	///
	/// # Safety
	///
	/// `engine` must be live, its vtable must hold functions of the signatures
	/// [`MessageEnd`], [`EntityMessageBegin`], [`UserMessageBegin`] and
	/// [`ClientPrintf`] at their slots, and they must stay loaded until Metamod
	/// unloads the plugin.
	unsafe fn install_sent_messages(
		self,
		engine: NonNull<sys::IVEngineServer>,
		binding: ServerBinding,
		listeners: SentMessageListeners,
	) -> Result<SentMessageHooks, HookError> {
		if ROUTE.installed(self) {
			return Err(HookError::AlreadyInstalled);
		}

		let target = HookTarget::instance(engine);
		let mut hooks = [None; 4];

		ROUTE.state.set(None);
		ROUTE.open.take();

		let hooked = (|| -> Result<(), HookError> {
			if listeners.user_message.is_some() {
				// The hook that opens user messages comes last: KHook can add hooks
				// late, from its worker, and those that end messages should not
				// miss the end of one it saw begin.
				//
				// SAFETY: As the caller promises.
				hooks[0] =
					Some(unsafe { self.add_hook(MESSAGE_END, target, HookTiming::Pre, &ROUTE) }?);

				// SAFETY: As above.
				hooks[1] = Some(unsafe {
					self.add_hook(ENTITY_MESSAGE_BEGIN, target, HookTiming::Post, &ROUTE)
				}?);

				// SAFETY: As above.
				hooks[2] = Some(unsafe {
					self.add_hook(USER_MESSAGE_BEGIN, target, HookTiming::Post, &ROUTE)
				}?);
			}

			if listeners.client_print.is_some() {
				// SAFETY: As above.
				hooks[3] =
					Some(unsafe { self.add_hook(CLIENT_PRINTF, target, HookTiming::Pre, &ROUTE) }?);
			}

			Ok(())
		})();

		if let Err(error) = hooked {
			for hook in hooks.into_iter().flatten() {
				self.remove_hook(hook);
			}

			return Err(error);
		}

		let hooks = SentMessageHooks { hooks };

		ROUTE.state.set(Some(RoutedSentMessages {
			binding,
			hooks,
			listeners,
		}));

		Ok(hooks)
	}

	/// Passes what the server sends players through `engine` to `listeners`:
	/// each user message, and each text printed to a client's console; see the
	/// [module documentation](crate::sent_message_hooks).
	///
	/// For a user message listener, this hooks `IVEngineServer::MessageEnd`
	/// before the call, and `EntityMessageBegin` and `UserMessageBegin` after
	/// it; for a client print listener, `ClientPrintf` before the call. The
	/// hooks only observe. Installing again while the hooks are installed
	/// returns [`HookError::AlreadyInstalled`]; removing them allows
	/// replacement.
	pub fn listen_sent_messages(
		self,
		engine: ValveEngine<'_>,
		binding: ServerBinding,
		listeners: SentMessageListeners,
	) -> Result<SentMessageHooks, HookError> {
		let engine = NonNull::new(engine.as_ptr()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: The engine's interface lives as long as the engine, which
		// outlives the plugin, and its vtable holds the four functions at their
		// slots, which the generated binding checks.
		unsafe { self.install_sent_messages(engine, binding, listeners) }
	}
}
