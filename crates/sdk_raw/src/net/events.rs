//! Hand-written layout of the engine's clients as listeners of game events.
//!
//! The game event manager passes each event it fires to the listeners of its
//! kind through their `IGameEventListener2::FireGameEvent`, a
//! [`FireGameEventFn`]. The engine's clients listen too, for the events
//! broadcast to them, and their `FireGameEvent` writes each to the client's
//! channel. They are `CGameClient`s, whose `IGameEventListener2` base starts
//! the object, with their `IClient` base right after it, as
//! [`incoming`](super::incoming) describes. [`listener_of_client`] confirms
//! that layout through the clients' run-time type information, and
//! [`client_of_listener`] relies on it.
//!
//! [`FireGameEventFn`]: crate::interfaces::game_event::FireGameEventFn

#[cfg(test)]
#[path = "../tests/net/events.rs"]
mod tests;

use super::incoming::{CLIENT_OFFSET, ClientLayoutError, GAME_CLIENT};
use crate::util::rtti;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

/// How far a client's `IClient` base lies past its `IGameEventListener2`
/// base, once [`listener_of_client`] has confirmed it, or zero before.
static LISTENER_TO_CLIENT: AtomicUsize = AtomicUsize::new(0);

/// The client whose `IGameEventListener2` base `listener` points to, or `None`
/// until [`listener_of_client`] has confirmed the engine's clients' layout.
///
/// # Safety
///
/// `listener` must point to the `IGameEventListener2` base of one of the
/// engine's clients, such as the `this` the engine passes to a method of the
/// vtable of a listener [`listener_of_client`] returned.
pub unsafe fn client_of_listener(
	listener: NonNull<sys::IGameEventListener2>,
) -> Option<NonNull<sys::IClient>> {
	match LISTENER_TO_CLIENT.load(Ordering::Relaxed) {
		0 => None,

		// SAFETY: The listener is a client's base, which `listener_of_client`
		// confirmed lies `offset` before the client's `IClient` base in every
		// client, all of one class.
		offset => Some(unsafe { listener.byte_add(offset) }.cast()),
	}
}

/// Finds the `IGameEventListener2` base of one of the engine's clients, after
/// checking through run-time type information that the client is a
/// `CGameClient`, with its bases where this module expects. Records that the
/// engine's clients are laid out so, for [`client_of_listener`].
///
/// # Safety
///
/// `client` must point to the `IClient` base of a live client the engine
/// made, a polymorphic subobject whose vtable the engine's module emitted with
/// run-time type information, and which stays loaded for the call.
#[doc(alias("CGameClient"))]
pub unsafe fn listener_of_client(
	client: NonNull<sys::IClient>,
) -> Result<NonNull<sys::IGameEventListener2>, ClientLayoutError> {
	let client = client.as_ptr().cast_const();

	// SAFETY: As the caller promises.
	let client_offset = unsafe { rtti::subobject_offset(client.cast(), GAME_CLIENT) };

	if client_offset.and_then(|offset| usize::try_from(offset).ok()) != Some(CLIENT_OFFSET) {
		return Err(ClientLayoutError);
	}

	// SAFETY: The complete object is a `CGameClient`, confirmed above with its
	// `IClient` base at `CLIENT_OFFSET`. `CBaseClient` declares
	// `IGameEventListener2`, a polymorphic base of one vtable pointer, first,
	// so `listener` is the start of the object, at that base's vtable pointer.
	let listener = unsafe { client.byte_sub(CLIENT_OFFSET) };

	// SAFETY: As above, `listener` is the client's `IGameEventListener2` base, a
	// polymorphic subobject of the same live `CGameClient`, by the engine's
	// declared base order; this run-time type read confirms that placement
	// before it is relied on.
	let listener_offset = unsafe { rtti::subobject_offset(listener.cast(), GAME_CLIENT) };

	if listener_offset != Some(0) {
		return Err(ClientLayoutError);
	}

	let listener = NonNull::new(listener.cast::<sys::IGameEventListener2>().cast_mut())
		.ok_or(ClientLayoutError)?;

	LISTENER_TO_CLIENT.store(CLIENT_OFFSET, Ordering::Relaxed);
	Ok(listener)
}
