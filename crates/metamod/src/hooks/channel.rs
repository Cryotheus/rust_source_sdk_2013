//! Hooks on what the engine sends through clients' net channels: its message
//! objects, data already encoded, and the packets it sends at once, such as
//! snapshots of the world.
//!
//! The hooks only observe. They never change, block or delay what is sent,
//! and the listener returns nothing.
//!
//! # Sends
//!
//! Everything the engine sends a client passes through the client's channel,
//! an `INetChannel`, by one of three methods, which the hooks patch on the
//! class of the engine's channels. The listener sees each call before the
//! channel takes what it is given, as a [`ChannelSend`]:
//!
//! - `SendNetMsg` takes one of the engine's message objects, such as a user
//!   message, a game event or a reliable sound, which encodes itself into one
//!   of the channel's streams. The listener gets it as an [`OutgoingMessage`].
//! - `SendData` appends messages already encoded to one of the streams, such
//!   as the data a client gets as it connects, a snapshot sent in full, or
//!   what [`NetChannel::send_encoded`] sends. The listener gets them as
//!   [`SentBits`].
//! - `SendDatagram` sends a packet at once, with what the streams hold and,
//!   unless there are none, messages for that packet alone, such as a
//!   snapshot that updates the client's last. The listener gets those
//!   messages as [`SentBits`].
//!
//! [`walk`] finds the messages in [`SentBits`], and in what an
//! [`OutgoingMessage`] encodes to.
//!
//! The listener runs for every send to every client, many times a frame, so
//! it should return at once for the channels it does not watch, before
//! copying or encoding anything.
//!
//! # What gets through
//!
//! The engine's channels all share the class, so the listener also sees the
//! channels of SourceTV's spectators, and on a listen server, the host's, in
//! both directions. It can tell clients apart by their channels, which
//! [`ValveEngine::net_channel`] returns. A send the listener makes through a
//! channel reaches it again, within its own call.
//!
//! The listener does not see:
//!
//! - what a channel writes to its packets itself, such as the reason it gives
//!   a client it disconnects, and the files it transfers: the listener sees
//!   the call that sends the packet, but not those bits;
//! - sends to bots and to SourceTV's own client on the game server, which have
//!   no channel;
//! - a send that a hook running before these superseded, such as another
//!   plugin's; with Metamod 2.0, KHook reports nothing of other plugins'
//!   hooks, so the listener sees those too;
//! - as with other Metamod hooks, sends made off the server's main thread,
//!   such as the snapshots worker threads send when `sv_parallel_sendsnapshot`
//!   is set, or while the plugin is paused or after it unloads. With Metamod
//!   1.12, those threads' sends go straight to the channel without entering
//!   SourceHook's hook loop, which is not safe to share between threads, so
//!   they also skip other plugins' SourceHook hooks on these methods when
//!   SourceHook patched them with this library's hook functions.
//!
//! The listener sees each send before the channel takes it, so it also sees
//! those the channel then refuses, such as data too large for its stream.
//!
//! # When to install
//!
//! The engine creates a channel for each player as they connect, so install
//! once one has, from that player's channel. The channels' class lasts as long
//! as the engine. The hooks stop calling back while the plugin is paused and
//! when it unloads, and Metamod removes them after unloading the plugin. With
//! Metamod 2.0, when one of the methods is already detoured by another
//! plugin, KHook adds the hook from its worker thread, so the sends just after
//! an install can pass unseen.
//!
//! [`ValveEngine::net_channel`]: source_sdk_2013::interfaces::ValveEngine::net_channel
//! [`walk`]: source_sdk_2013::net::outgoing::walk

#[cfg(test)]
#[path = "../tests/hooks/channel.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use crate::hooks::server::with_server;
use source_sdk_2013::bitbuf::BitWriter;
use source_sdk_2013::net::NetChannel;
use source_sdk_2013::net::outgoing::OutgoingMessage;
use source_sdk_2013::raw::bitbuf::BfWrite;

use source_sdk_2013::raw::net::outgoing::{
	SEND_DATA_SLOT, SEND_DATAGRAM_SLOT, SEND_NET_MSG_SLOT, SendDataFn as SendData,
	SendDatagramFn as SendDatagram, SendNetMsgFn as SendNetMsg,
};

use source_sdk_2013::{Server, ServerBinding, sys};
use std::cell::Cell;
use std::ffi::c_int;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// Observes what the engine is sending through a client's channel, with a
/// callback-scoped server, the channel, and what is sent, before the channel
/// takes it. A panic is caught.
pub type ChannelSendFn = for<'s> fn(Server<'s>, NetChannel<'s>, &ChannelSend<'_>);

/// `INetChannel::SendData`.
const SEND_DATA: VirtualFunction<SendData> = VirtualFunction::new(SEND_DATA_SLOT);

/// `INetChannel::SendDatagram`.
const SEND_DATAGRAM: VirtualFunction<SendDatagram> = VirtualFunction::new(SEND_DATAGRAM_SLOT);

/// `INetChannel::SendNetMsg`.
const SEND_NET_MSG: VirtualFunction<SendNetMsg> = VirtualFunction::new(SEND_NET_MSG_SLOT);

static ROUTE: ChannelRoute = ChannelRoute(Cell::new(None));

/// The hooks [`MetamodApi::listen_channel_sends`] installs.
///
/// They stop calling back while the plugin is paused and when it unloads, and
/// Metamod removes them after unloading the plugin. [`Self::remove`] stops
/// them earlier.
#[must_use = "retain the hooks to remove them"]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelHooks {
	/// The hooks of `SendNetMsg`, `SendData` and `SendDatagram`, in that order.
	hooks: [HookId; 3],
}

impl ChannelHooks {
	/// Stops the hooks, so no further send reaches the listener.
	pub fn remove(self, api: MetamodApi<'_>) {
		for hook in self.hooks {
			api.remove_hook(hook);
		}

		// Hooks installed since these were removed keep their listener.
		if ROUTE.0.get().is_some_and(|routed| routed.hooks == self) {
			ROUTE.0.set(None);
		}
	}
}

/// The listener the hooks run, with the hooks.
struct ChannelRoute(Cell<Option<RoutedSends>>);

impl ChannelRoute {
	/// Passes `send`, which the engine is sending through `channel`, to the
	/// listener.
	fn deliver(&self, channel: *mut sys::INetChannel, send: &ChannelSend<'_>) {
		let (Some(routed), Some(channel)) = (self.0.get(), NonNull::new(channel)) else {
			return;
		};

		with_server(routed.binding, |server| {
			// SAFETY: The hook runs before a method of the engine's channel, on the
			// main thread, so the channel is live for the call.
			let channel = unsafe { NetChannel::from_raw(channel) };

			(routed.listener)(server, channel, send);
		});
	}

	/// Whether the route's hooks are installed, for this load of the plugin.
	fn installed(&self, api: MetamodApi<'_>) -> bool {
		self.0.get().is_some_and(|routed| {
			routed
				.hooks
				.hooks
				.into_iter()
				.any(|hook| api.has_hook(hook))
		})
	}
}

impl Handler<SendData> for ChannelRoute {
	/// Passes the data to the listener, before the channel appends it to a
	/// stream.
	fn call(&self, call: &HookCall<'_, SendData>) -> HookAction<bool> {
		// An earlier hook, such as another plugin's, superseded the send.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let (data, reliable) = call.args();

		if let Some(buffer) = NonNull::new(data) {
			let data = SentBits::new(buffer);

			self.deliver(call.this(), &ChannelSend::Data { data, reliable });
		}

		HookAction::Ignore
	}
}

impl Handler<SendDatagram> for ChannelRoute {
	/// Passes the packet's own messages, if any, to the listener, before the
	/// channel sends the packet.
	fn call(&self, call: &HookCall<'_, SendDatagram>) -> HookAction<c_int> {
		// An earlier hook, such as another plugin's, superseded the send.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let (data,) = call.args();
		let data = NonNull::new(data).map(SentBits::new);

		self.deliver(call.this(), &ChannelSend::Datagram(data));
		HookAction::Ignore
	}
}

impl Handler<SendNetMsg> for ChannelRoute {
	/// Passes the message to the listener, before it encodes itself into a
	/// stream.
	fn call(&self, call: &HookCall<'_, SendNetMsg>) -> HookAction<bool> {
		// An earlier hook, such as another plugin's, superseded the send.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let (message, force_reliable, voice) = call.args();

		if let Some(message) = NonNull::new(message) {
			// SAFETY: The hook runs before `SendNetMsg`, on the main thread, with
			// the engine's message, which outlives the call.
			let message = unsafe { OutgoingMessage::from_raw(message) };

			self.deliver(
				call.this(),
				&ChannelSend::Message {
					message,
					force_reliable,
					voice,
				},
			);
		}

		HookAction::Ignore
	}
}

// SAFETY: Installation requires a main-thread MetamodApi; the hook dispatcher
// calls handlers only on that thread.
unsafe impl Sync for ChannelRoute {}

/// What the engine is sending through a client's channel, as a
/// [`ChannelSendFn`] sees it before the channel takes it. Valid only for the
/// call.
#[derive(Debug, Clone, Copy)]
pub enum ChannelSend<'a> {
	/// One of the engine's message objects, which encodes itself into one of
	/// the channel's streams (`INetChannel::SendNetMsg`).
	Message {
		/// The message.
		message: OutgoingMessage<'a>,

		/// Whether the message goes in the reliable stream even if it is
		/// [unreliable](OutgoingMessage::is_reliable) itself.
		force_reliable: bool,

		/// Whether the message goes in the voice stream, whatever its
		/// reliability.
		voice: bool,
	},

	/// Encoded messages, which the channel appends to one of its streams
	/// (`INetChannel::SendData`).
	Data {
		/// The messages.
		data: SentBits<'a>,

		/// Whether they go in the reliable stream, rather than the unreliable.
		reliable: bool,
	},

	/// A packet the channel sends at once, with what its streams hold, and
	/// these encoded messages unless `None`, for that packet alone
	/// (`INetChannel::SendDatagram`).
	Datagram(Option<SentBits<'a>>),
}

#[derive(Clone, Copy)]
struct RoutedSends {
	binding: ServerBinding,
	hooks: ChannelHooks,
	listener: ChannelSendFn,
}

/// Encoded messages the engine is sending through a channel, in one of its
/// buffers, which the listener copies only if it needs them. Valid only for
/// the call.
#[doc(alias("bf_write"))]
#[derive(Debug, Clone, Copy)]
pub struct SentBits<'a> {
	buffer: NonNull<sys::bf_write>,
	_call: PhantomData<&'a ()>,
}

impl SentBits<'_> {
	/// The engine's buffer, which holds the messages for the call.
	const fn new(buffer: NonNull<sys::bf_write>) -> Self {
		Self {
			buffer,
			_call: PhantomData,
		}
	}

	/// The number of bits written, which the messages take.
	#[doc(alias("GetNumBitsWritten"))]
	pub fn bit_len(&self) -> usize {
		let buffer = BfWrite::from_sys(self.buffer).as_ptr();

		// SAFETY: The engine's buffer is live for the call, and only read.
		let written = unsafe { (&raw const (*buffer).cur_bit).read() };

		usize::try_from(written).unwrap_or(0)
	}

	/// Copies the messages, which
	/// [`walk`](source_sdk_2013::net::outgoing::walk) reads, or returns `None`
	/// if the buffer overflowed or its fields are inconsistent.
	pub fn copy(&self) -> Option<BitWriter> {
		// SAFETY: The engine's buffer is live for the call, and its storage holds
		// the bytes it describes, word-aligned as `bf_write` stores them, which
		// nothing writes to while the listener runs.
		unsafe { BfWrite::read_back(BfWrite::from_sys(self.buffer)) }.map(BitWriter::from)
	}
}

impl MetamodApi<'_> {
	/// Hooks `SendNetMsg`, `SendData` and `SendDatagram` on the class of
	/// `channel`.
	///
	/// # Safety
	///
	/// `channel` must be a live channel of the engine's, whose vtable holds
	/// functions of the signatures [`SendNetMsg`], [`SendData`] and
	/// [`SendDatagram`] at their slots, until Metamod unloads the plugin.
	unsafe fn install_channel_sends(
		self,
		channel: NonNull<sys::INetChannel>,
		binding: ServerBinding,
		listener: ChannelSendFn,
	) -> Result<ChannelHooks, HookError> {
		if ROUTE.installed(self) {
			return Err(HookError::AlreadyInstalled);
		}

		let target = HookTarget::class_of(channel);
		let mut installed = [None; 3];

		ROUTE.0.set(None);

		let hooked = (|| -> Result<[HookId; 3], HookError> {
			// SAFETY: As the caller promises; a `MetamodApi` only exists on the main
			// thread.
			let message = *installed[0]
				.insert(unsafe { self.add_hook(SEND_NET_MSG, target, HookTiming::Pre, &ROUTE) }?);

			// SAFETY: As above.
			let data = *installed[1]
				.insert(unsafe { self.add_hook(SEND_DATA, target, HookTiming::Pre, &ROUTE) }?);

			// SAFETY: As above.
			let datagram = *installed[2]
				.insert(unsafe { self.add_hook(SEND_DATAGRAM, target, HookTiming::Pre, &ROUTE) }?);

			Ok([message, data, datagram])
		})();

		let hooks = match hooked {
			Ok(hooks) => ChannelHooks { hooks },

			Err(error) => {
				for hook in installed.into_iter().flatten() {
					self.remove_hook(hook);
				}

				return Err(error);
			}
		};

		ROUTE.0.set(Some(RoutedSends {
			binding,
			hooks,
			listener,
		}));

		Ok(hooks)
	}

	/// Passes what the engine sends through each client's channel to
	/// `listener`, before the channel takes it; see the
	/// [module documentation](crate::hooks::channel).
	///
	/// This hooks `INetChannel::SendNetMsg`, `SendData` and `SendDatagram`
	/// before the call, on the class of `channel`, which the engine's channels
	/// share. Bots have none: pass a player's, such as from
	/// [`ValveEngine::net_channel`]. The hooks only observe. Installing again
	/// while the hooks are installed returns [`HookError::AlreadyInstalled`];
	/// removing them allows replacement.
	///
	/// [`ValveEngine::net_channel`]: source_sdk_2013::interfaces::ValveEngine::net_channel
	pub fn listen_channel_sends(
		self,
		channel: NetChannel<'_>,
		binding: ServerBinding,
		listener: ChannelSendFn,
	) -> Result<ChannelHooks, HookError> {
		let channel = NonNull::new(channel.as_ptr()).ok_or(HookError::InvalidArgument)?;

		// SAFETY: A `NetChannel` is a live channel of the engine's, whose class
		// lasts as long as the engine, and whose vtable holds the three methods
		// at their slots, which the generated binding checks.
		unsafe { self.install_channel_sends(channel, binding, listener) }
	}
}
