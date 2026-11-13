//! Game events the server sends each client, seen before the engine sends
//! them.
//!
//! The game event manager passes each event it fires to the listeners of its
//! kind, and the engine's clients listen too, for the events broadcast to
//! them: a client's `IGameEventListener2::FireGameEvent` writes the event to
//! the client's channel. A Metamod plugin hooks that method on the clients'
//! class, which [`hook_target`] finds and checks, and passes each call to
//! [`route_event`], which lets a callback keep the event from that client,
//! while other clients, and the server's own listeners, still get it.
//!
//! [`send`] sends an event to chosen clients alone, as the engine sends each
//! one a fired event, without firing it.
//!
//! Only the game server's clients are covered. SourceTV's spectators are
//! clients of SourceTV's own server, of another class.

#[cfg(test)]
#[path = "../tests/net/events.rs"]
mod tests;

use crate::InterfaceError;
use crate::interfaces::game_event::GameEvent;
use crate::interfaces::game_server::GameClient;
use crate::net::incoming::HookTargetError;
use crate::net::messages::GameEvent as GameEventMessage;
use crate::net::{EncodeError, NetChannel, NetMessage, Reliability};
use crate::server::{Server, ServerBinding};
use crate::user_messages::Recipients;
use sdk_raw::net::events as raw;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;

/// Whether the engine sends a client an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Delivery {
	/// Lets the engine send the event.
	#[default]
	Deliver,

	/// Keeps the event from the client. The engine goes on to the event's other
	/// listeners as though it sent it.
	Withhold,
}

/// Why [`send`] sent an event to no one.
#[derive(Debug, thiserror::Error)]
pub enum SendEventError {
	/// The encoded event does not fit in an `svc_GameEvent`, which carries
	/// fewer than 2048 bits.
	#[error(transparent)]
	Encode(#[from] EncodeError),

	/// An interface needed to send the event is missing.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The game event manager did not encode the event, which it refuses for
	/// an event without a description.
	#[error("the game event manager could not encode the event")]
	NotSerialized,
}

/// The net channels of `recipients` that have a player entity, to [`send_to`].
pub(crate) fn channels<'s>(
	server: Server<'s>,
	recipients: &Recipients,
) -> Result<Vec<NetChannel<'s>>, InterfaceError> {
	let engine = server.valve_engine()?;

	Ok(recipients
		.players()
		.iter()
		.filter_map(|&index| engine.edict_of_index(index))
		.filter(|edict| edict.entity().is_some())
		.filter_map(|edict| engine.net_channel(edict))
		.collect())
}

/// Finds the game event listener of one of the engine's clients to hook, after
/// checking its class and place through the engine's run-time type
/// information. Every client shares its vtable.
pub fn hook_target(
	server: Server<'_>,
) -> Result<NonNull<sys::IGameEventListener2>, HookTargetError> {
	let client = server
		.valve_engine()?
		.game_server()
		.ok_or(HookTargetError::NoServer)?
		.client(0)
		.ok_or(HookTargetError::NotReady)?;

	let client = NonNull::new(client.as_ptr()).ok_or(HookTargetError::NotReady)?;

	// SAFETY: The client is one of the engine's live clients, which the engine
	// built with run-time type information, like each of its bases. The
	// engine's module stays loaded during the server's callback.
	Ok(unsafe { raw::listener_of_client(client) }?)
}

/// Passes an event the engine is about to send a client to `callback`,
/// returning what it decided.
///
/// # Safety
///
/// Call it from a hook on `FireGameEvent` of the vtable [`hook_target`] found,
/// before the method runs, on the server's main thread: `this` is the listener
/// the engine called, and `event` its argument. Panics are caught, and deliver
/// the event.
pub unsafe fn route_event(
	binding: &ServerBinding,
	this: NonNull<sys::IGameEventListener2>,
	event: NonNull<sys::IGameEvent>,
	callback: impl for<'s> FnOnce(Server<'s>, GameClient<'s>, GameEvent<'s>) -> Delivery,
) -> Delivery {
	// SAFETY: As the caller promises, `this` is a listener of the class whose
	// vtable `hook_target` found, the start of one of the engine's clients.
	let Some(client) = (unsafe { raw::client_of_listener(this) }) else {
		return Delivery::Deliver;
	};

	let scope = ();

	// SAFETY: The hook runs during the engine's call into the listener, on the
	// main thread.
	let server = unsafe { binding.server(&scope) };

	// SAFETY: The engine keeps its clients while it sends them events.
	let client = unsafe { GameClient::from_raw(client) };

	// SAFETY: The manager keeps the event it fires until each of its listeners
	// has had it.
	let event = unsafe { GameEvent::from_raw(event) };

	catch_unwind(AssertUnwindSafe(|| callback(server, client, event))).unwrap_or(Delivery::Deliver)
}

/// Sends `event` to `recipients` alone, as the engine sends a client each
/// event broadcast to it: as an `svc_GameEvent`, in the stream the event's
/// description asks for. The event is sent, never fired, so no listener on the
/// server and no other client gets it, and hooks on the manager's `FireEvent`
/// or on the clients' `FireGameEvent` (see [`route_event`]) never see it. The
/// event stays the caller's, to send again, fire or drop. Returns how many
/// recipients it was sent to.
///
/// Only recipients with a player entity and a net channel get the event.
/// Bots, SourceTV and empty slots have no channel, and a client still
/// connecting has no player until the game creates one
/// (`ClientPutInServer`), so all of them are skipped. A recipient whose stream
/// has no room left for the event is not counted. [`Recipients::reliable`] has
/// no effect: the event's description chooses the stream.
#[doc(alias("svc_GameEvent"))]
pub fn send(
	server: Server<'_>,
	recipients: &Recipients,
	event: GameEvent<'_>,
) -> Result<usize, SendEventError> {
	let channels = channels(server, recipients)?;

	send_to(server, channels, event)
}

/// Sends `event` through each of `channels`, as [`send`] does, returning how
/// many took it. Encodes nothing for no channel.
pub(crate) fn send_to(
	server: Server<'_>,
	channels: Vec<NetChannel<'_>>,
	event: GameEvent<'_>,
) -> Result<usize, SendEventError> {
	if channels.is_empty() {
		return Ok(0);
	}

	// The engine, which the SDK does not include, sends a fired event to each
	// client in the stream `IGameEvent::IsReliable` names.
	let reliability = match event.is_reliable() {
		true => Reliability::Reliable,
		false => Reliability::Unreliable,
	};
	let data = server
		.game_events()?
		.serialize_event(event)
		.ok_or(SendEventError::NotSerialized)?;
	let message = GameEventMessage { data: &data }.encode()?;

	Ok(channels
		.into_iter()
		.filter(|channel| channel.send_encoded(&message, reliability).is_ok())
		.count())
}
