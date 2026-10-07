//! Tests of routing the events sent to a mock client, and of finding the
//! clients to hook.

use super::*;
use crate::test_support::net::cheats::MockEngine;
use crate::test_support::server::mock_binding;
use sdk_raw::test_support::net::mock_game_client;

/// A listener's `FireGameEvent` that no test calls.
unsafe extern "C" fn fire_game_event(
	_this: *mut sys::IGameEventListener2,
	_event: *mut sys::IGameEvent,
) {
}

/// A new mock client, and its `IGameEventListener2` base, whose layout this
/// confirms.
fn mock_client() -> (NonNull<sys::IClient>, NonNull<sys::IGameEventListener2>) {
	let client = mock_game_client("CGameClient", fire_game_event);

	// SAFETY: The mock is leaked, with the type information of a client.
	(client, unsafe { raw::listener_of_client(client) }.unwrap())
}

#[test]
fn a_panicking_callback_delivers_the_event() {
	let (_, listener) = mock_client();

	// SAFETY: As a hook would: the listener is a client's, and nothing reads
	// the event.
	let delivery = unsafe {
		route_event(&mock_binding(), listener, NonNull::dangling(), |_, _, _| {
			panic!("the callback failed")
		})
	};

	assert_eq!(delivery, Delivery::Deliver);
}

#[test]
fn events_reach_the_callback_with_their_client() {
	let (client, listener) = mock_client();
	let event = NonNull::dangling();
	let mut seen = None;

	// SAFETY: As above.
	let delivery = unsafe {
		route_event(&mock_binding(), listener, event, |_, client, event| {
			seen = Some((client.as_ptr(), event.as_ptr()));
			Delivery::Withhold
		})
	};

	assert_eq!(delivery, Delivery::Withhold);
	assert_eq!(seen, Some((client.as_ptr(), event.as_ptr())));
}

#[test]
fn no_client_can_be_hooked_before_the_server_has_one() {
	let engine = MockEngine::new(&[], c"0", &[]);

	assert!(matches!(
		hook_target(engine.server()),
		Err(HookTargetError::NotReady)
	));
}
