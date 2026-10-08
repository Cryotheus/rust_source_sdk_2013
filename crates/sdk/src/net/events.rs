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
//! Only the game server's clients are covered. SourceTV's spectators are
//! clients of SourceTV's own server, of another class.

#[cfg(test)]
#[path = "../tests/net/events.rs"]
mod tests;

use crate::interfaces::game_event::GameEvent;
use crate::interfaces::game_server::GameClient;
use crate::net::incoming::HookTargetError;
use crate::server::{Server, ServerBinding};
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
