//! Tests of `crate::hooks::delivery`: hooks of `FireGameEvent` on a mock client
//! of the engine's, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::raw::net::events::listener_of_client;
use source_sdk_2013::raw::test_support::net::mock_game_client;
use std::cell::RefCell;
use std::ptr;

thread_local! {
	/// What happened during the deliveries since the last [`send`], in order.
	static SEEN: RefCell<Vec<Seen>> = const { RefCell::new(Vec::new()) };

	/// The address of the event the callback withholds.
	static WITHHELD: Cell<usize> = const { Cell::new(0) };
}

/// Something the client or the callback noticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seen {
	/// The callback saw the event at this address, for the client whose
	/// `IClient` base is at this address.
	Callback(usize, usize),

	/// The client whose listener is at this address was sent the event at this
	/// address.
	Client(usize, usize),
}

#[test]
fn events_an_earlier_hook_withheld_reach_neither_the_callback_nor_the_client() {
	on_both(|harness| {
		let api = harness.api();
		let (_, listener) = mock_client();

		fn withhold(_call: &HookCall<'_, FireGameEvent>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: The mock client is leaked with its vtables, whose listener's
		// has `FireGameEvent` at its slot.
		unsafe {
			api.add_hook(
				FIRE_GAME_EVENT,
				HookTarget::class_of(listener),
				HookTiming::Pre,
				&withhold,
			)
			.unwrap();
			api.install_event_delivery(listener, tf2_binding(no_interfaces), on_delivery)
				.unwrap();
		}

		assert_eq!(send(harness, listener, 0x20), vec![]);
	});
}

#[test]
fn events_reach_the_callback_and_withheld_ones_skip_the_client() {
	on_both(|harness| {
		let api = harness.api();
		let (client, listener) = mock_client();
		let (client, listener_address) = (client.addr().get(), listener.addr().get());

		// SAFETY: The mock client is leaked with its vtables, whose listener's
		// has `FireGameEvent` at its slot.
		unsafe { api.install_event_delivery(listener, tf2_binding(no_interfaces), on_delivery) }
			.unwrap();

		// A second hook is refused, so that each delivery is decided once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe {
				api.install_event_delivery(listener, tf2_binding(no_interfaces), on_delivery)
			},
			Err(HookError::AlreadyInstalled)
		));

		let scope = ();

		// SAFETY: The binding's server is only asked whether the hook is
		// installed, which reaches no interface.
		let server = unsafe { tf2_binding(no_interfaces).server(&scope) };

		// So is one through the engine, before it looks for the clients.
		assert!(matches!(
			api.hook_event_delivery(server, tf2_binding(no_interfaces), on_delivery),
			Err(DeliveryHookError::Hook(HookError::AlreadyInstalled))
		));

		WITHHELD.set(0x30);

		assert_eq!(
			send(harness, listener, 0x20),
			vec![
				Seen::Callback(client, 0x20),
				Seen::Client(listener_address, 0x20)
			]
		);

		assert_eq!(
			send(harness, listener, 0x30),
			vec![Seen::Callback(client, 0x30)]
		);

		// A null event reaches the client only.
		assert_eq!(
			send(harness, listener, 0),
			vec![Seen::Client(listener_address, 0)]
		);
	});
}

/// The mock client's own `FireGameEvent`, which notes the event.
unsafe extern "C" fn client_fire_game_event(
	this: *mut sys::IGameEventListener2,
	event: *mut sys::IGameEvent,
) {
	SEEN.with_borrow_mut(|seen| seen.push(Seen::Client(this.addr(), event.addr())));
}

/// A new mock client of the engine's, and its `IGameEventListener2` base,
/// whose layout this confirms.
fn mock_client() -> (NonNull<sys::IClient>, NonNull<sys::IGameEventListener2>) {
	let client = mock_game_client("CGameClient", client_fire_game_event);

	// SAFETY: The mock is leaked, with the type information of a client.
	(client, unsafe { listener_of_client(client) }.unwrap())
}

/// The callback, which notes what it saw, and withholds the event at
/// [`WITHHELD`].
fn on_delivery(_server: Server<'_>, client: GameClient<'_>, event: GameEvent<'_>) -> Delivery {
	let (client, event) = (client.as_ptr().addr(), event.as_ptr().addr());

	SEEN.with_borrow_mut(|seen| seen.push(Seen::Callback(client, event)));

	match event == WITHHELD.get() {
		true => Delivery::Withhold,
		false => Delivery::Deliver,
	}
}

/// Sends the event at `event` to the mock client whose listener is
/// `listener`, through its hooked vtable, and returns what happened. The
/// event is not read.
fn send(harness: &Harness, listener: NonNull<sys::IGameEventListener2>, event: usize) -> Vec<Seen> {
	SEEN.take();

	harness.call::<FireGameEvent>(
		listener.as_ptr(),
		FIRE_GAME_EVENT_SLOT,
		(ptr::without_provenance_mut(event),),
	);

	SEEN.take()
}
