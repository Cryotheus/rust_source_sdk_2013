//! Tests of learning the size of the engine's `CNetMessage` from a message
//! that names the handler it was passed to, before any client sent a message
//! that points into its own buffers.
//!
//! A test binary of its own, since the first message read fixes the size for
//! the process.

use source_sdk_2013_raw::net::incoming::{
	ClientMessage, MessageClass, SMALLEST_MESSAGE_BASE, TickFields, read_message,
};
use source_sdk_2013_raw::test_support::net::mock_message;
use std::ptr::NonNull;

/// The size of `CNetMessage` in mock messages.
const BASE: usize = SMALLEST_MESSAGE_BASE;

/// A `NET_Tick` of `tick` whose `CNetMessage` is `base` bytes, naming
/// `handler` as the handler it is processed by.
fn tick(
	base: usize,
	handler: NonNull<sys::IClientMessageHandler>,
	tick: i32,
) -> NonNull<sys::INetMessage> {
	let message = mock_message(base + size_of::<TickFields>());

	// SAFETY: The mock holds the fields past `base`, aligned for them.
	unsafe {
		message
			.as_ptr()
			.byte_add(base)
			.cast::<TickFields>()
			.write(TickFields {
				handler: handler.as_ptr().cast(),
				tick,
				host_frame_time: 0.015,
				host_frame_time_std_deviation: 0.0,
			});
	}

	message
}

#[test]
fn a_message_naming_the_handler_it_was_passed_to_confirms_the_base() {
	let handler = NonNull::<u64>::from(Box::leak(Box::new(0))).cast();
	let other = NonNull::<u64>::from(Box::leak(Box::new(0))).cast();

	// Naming another handler, it confirms nothing.
	// SAFETY: The mock is live and unchanged.
	assert!(unsafe { read_message(MessageClass::Tick, handler, tick(BASE, other, 1)) }.is_none());

	// SAFETY: As above.
	let Some(ClientMessage::Tick(fields)) =
		(unsafe { read_message(MessageClass::Tick, handler, tick(BASE, handler, 2)) })
	else {
		panic!("the tick is read");
	};

	assert_eq!(fields.tick, 2);

	// Once the base is known, messages of another base are refused, whatever
	// they name.
	// SAFETY: As above.
	assert!(
		unsafe { read_message(MessageClass::Tick, handler, tick(BASE + 8, handler, 3)) }.is_none()
	);

	// And those of the base are read without naming the handler.
	// SAFETY: As above.
	assert!(unsafe { read_message(MessageClass::Tick, handler, tick(BASE, other, 4)) }.is_some());
}
