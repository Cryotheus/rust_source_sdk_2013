//! Tests of finding the event listener of mock clients, which carry the
//! run-time type information of the engine's own.

use super::*;
use crate::test_support::net::mock_game_client;

/// A listener's `FireGameEvent` that no test calls.
unsafe extern "C" fn fire_game_event(
	_this: *mut sys::IGameEventListener2,
	_event: *mut sys::IGameEvent,
) {
}

#[test]
fn a_game_clients_listener_starts_it_and_leads_back_to_it() {
	let client = mock_game_client("CGameClient", fire_game_event);

	// SAFETY: The mock is leaked, with the type information of a client.
	let listener = unsafe { listener_of_client(client) }.unwrap();

	assert_eq!(listener.addr().get() + CLIENT_OFFSET, client.addr().get());

	// SAFETY: The listener is the mock's, whose layout was just confirmed.
	assert_eq!(unsafe { client_of_listener(listener) }, Some(client));
}

#[test]
fn clients_of_other_classes_are_refused() {
	let client = mock_game_client("CHLTVClient", fire_game_event);

	// SAFETY: As for a `CGameClient`'s mock.
	let listener = unsafe { listener_of_client(client) };

	assert_eq!(listener, Err(ClientLayoutError));
}
