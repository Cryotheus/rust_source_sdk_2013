//! Tests of `crate::gc_hooks`: pre hooks of `SendMessage` on a mock
//! coordinator class, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, expect, on_both};

use source_sdk_2013::raw::steam::{
	GC_RESULT_NO_MESSAGE, GC_RESULT_NOT_LOGGED_ON, ISteamGameCoordinatorVtable, PROTOBUF_FLAG,
};

use std::cell::RefCell;

/// A message body, which the callback checks it sees whole.
const BODY: &[u8] = b"kill eater";

thread_local! {
	/// The types the mock coordinator received since the last [`send`].
	static SENT: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };

	/// What the callback saw since the last [`send`].
	static SEEN: RefCell<Vec<(u32, bool, usize)>> = const { RefCell::new(Vec::new()) };
}

/// The callback, which notes what it saw and blocks the Strange messages,
/// returning a result the mock coordinator never does.
fn block_strange(send: &GcSend<'_>) -> GcSendAction {
	expect(
		send.data.is_empty() || send.data == BODY,
		"the callback saw another body",
	);
	SEEN.with_borrow_mut(|seen| {
		seen.push((send.message_type.id(), send.protobuf, send.data.len()));
	});

	match send.message_type.id() {
		1071 | 1084 | 1097 => GcSendAction::Block(GcResult::NoMessage),
		_ => GcSendAction::Pass,
	}
}

/// The mock coordinator's `IsMessageAvailable`, which is never called.
unsafe extern "C" fn game_is_message_available(_: *mut ISteamGameCoordinator, _: *mut u32) -> bool {
	expect(false, "IsMessageAvailable was called");
	false
}

/// The mock coordinator's `RetrieveMessage`, which is never called.
unsafe extern "C" fn game_retrieve(
	_: *mut ISteamGameCoordinator,
	_: *mut u32,
	_: *mut c_void,
	_: u32,
	_: *mut u32,
) -> i32 {
	expect(false, "RetrieveMessage was called");
	GC_RESULT_NO_MESSAGE
}

/// The mock coordinator's `SendMessage`, which notes the type it received.
unsafe extern "C" fn game_send(
	_: *mut ISteamGameCoordinator,
	wire: u32,
	_: *const c_void,
	_: u32,
) -> i32 {
	SENT.with_borrow_mut(|sent| sent.push(wire));
	GC_RESULT_NOT_LOGGED_ON
}

/// A coordinator of a new class, whose vtable holds the mock functions.
fn new_coordinator() -> Box<ISteamGameCoordinator> {
	let vtable = Box::leak(Box::new(ISteamGameCoordinatorVtable {
		send_message: game_send,
		is_message_available: game_is_message_available,
		retrieve_message: game_retrieve,
	}));

	Box::new(ISteamGameCoordinator { vtable_: vtable })
}

/// Sends `wire` with `data`, or a null pointer and no bytes, through the
/// hooked vtable, and returns the result and what the coordinator and the
/// callback saw.
fn send(
	harness: &Harness,
	coordinator: &mut ISteamGameCoordinator,
	wire: u32,
	data: Option<&[u8]>,
) -> (i32, Vec<u32>, Vec<(u32, bool, usize)>) {
	let (data, len) = data.map_or((std::ptr::null(), 0), |data| {
		(data.as_ptr().cast(), data.len() as u32)
	});

	SENT.take();
	SEEN.take();

	let result = harness.call::<SendMessageFn>(coordinator, SEND_MESSAGE_SLOT, (wire, data, len));

	(result, SENT.take(), SEEN.take())
}

#[test]
fn strange_messages_are_blocked_and_others_reach_the_coordinator() {
	on_both(|harness| {
		let api = harness.api();
		let mut coordinator = new_coordinator();

		// SAFETY: The mock class has `SendMessage` at the slot, and is leaked.
		unsafe { api.install_send(NonNull::from(&mut *coordinator), block_strange) }.unwrap();

		// A second hook of the class is refused, so that each send is decided
		// once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_send(NonNull::from(&mut *coordinator), block_strange) },
			Err(HookError::AlreadyInstalled)
		));

		for wire in [
			1071,
			1071 | PROTOBUF_FLAG,
			1097 | PROTOBUF_FLAG,
			1084 | PROTOBUF_FLAG,
		] {
			let protobuf = wire & PROTOBUF_FLAG != 0;

			assert_eq!(
				send(harness, &mut coordinator, wire, Some(BODY)),
				(
					GC_RESULT_NO_MESSAGE,
					vec![],
					vec![(wire & !PROTOBUF_FLAG, protobuf, BODY.len())]
				),
				"{wire:#x}"
			);
		}

		for wire in [4007 | PROTOBUF_FLAG, 6295 | PROTOBUF_FLAG, 1005] {
			let protobuf = wire & PROTOBUF_FLAG != 0;

			assert_eq!(
				send(harness, &mut coordinator, wire, Some(BODY)),
				(
					GC_RESULT_NOT_LOGGED_ON,
					vec![wire],
					vec![(wire & !PROTOBUF_FLAG, protobuf, BODY.len())]
				),
				"{wire:#x}"
			);
		}

		// An empty message without data reaches the callback as an empty body.
		assert_eq!(
			send(harness, &mut coordinator, 4007 | PROTOBUF_FLAG, None),
			(
				GC_RESULT_NOT_LOGGED_ON,
				vec![4007 | PROTOBUF_FLAG],
				vec![(4007, true, 0)]
			)
		);
	});
}
