//! Messages clients send, laid out as the engine passes them to the incoming
//! hook.
//!
//! Every mock message here has the smallest `CNetMessage` the engine's
//! messages are read with, since the first message decoded fixes that size
//! for the process.

use sdk_raw::net::incoming::{RespondCvarValueFields, SMALLEST_MESSAGE_BASE};
use sdk_raw::test_support::net::mock_message;
use sdk_raw::util::cstr::buffer_from_cstr;
use std::ffi::{CStr, c_int};
use std::ptr::{NonNull, null_mut};

/// A `clc_RespondCvarValue` laid out as
/// [`IncomingMessage::decode`](crate::net::incoming::IncomingMessage::decode)
/// expects, answering the query carrying `cookie`.
///
/// For tests only.
///
/// # Panics
///
/// If the name or the value does not fit its buffer.
pub fn respond_cvar_value(
	cookie: c_int,
	status: c_int,
	name: &CStr,
	value: &CStr,
) -> NonNull<sys::INetMessage> {
	let message = mock_message(SMALLEST_MESSAGE_BASE + size_of::<RespondCvarValueFields>());
	let fields = message
		.as_ptr()
		.wrapping_byte_add(SMALLEST_MESSAGE_BASE)
		.cast::<RespondCvarValueFields>();

	// SAFETY: The object holds the fields past its `CNetMessage`, aligned for
	// them. The engine points the name and value at their buffers.
	unsafe {
		fields.write(RespondCvarValueFields {
			handler: null_mut(),
			cookie,
			name: (&raw const (*fields).name_buffer).cast(),
			value: (&raw const (*fields).value_buffer).cast(),
			status,
			name_buffer: buffer_from_cstr(name).expect("the name fits"),
			value_buffer: buffer_from_cstr(value).expect("the value fits"),
		});
	}

	message
}

/// A message whose reported size is too small for any kind's fields, as an
/// engine laid out otherwise might report, which nothing can decode.
///
/// For tests only.
pub fn unreadable() -> NonNull<sys::INetMessage> {
	mock_message(SMALLEST_MESSAGE_BASE)
}
