//! Tests of sending messages through a client's net channel: what the
//! engine's `SendData` receives, and how its refusal is reported.

use super::*;
use crate::net::messages::{Print, Raw};
use crate::test_support::net::MockChannel;

#[test]
fn a_full_stream_is_an_error() {
	let mock = MockChannel::new();
	let mut body = BitWriter::new();

	body.write_bit(true);
	mock.refuse();

	let raw = Raw {
		id: MessageId::NOP,
		body: &body,
	};

	assert_eq!(
		mock.channel().send(&raw),
		Err(SendError::TooLarge { bits: 7 })
	);
}

#[test]
fn messages_are_sent_as_their_type_then_fields() {
	let mock = MockChannel::new();
	let channel = mock.channel();

	channel.send(&Print { text: c"hello" }).unwrap();
	channel
		.send_with(&Print { text: c"" }, Reliability::Unreliable)
		.unwrap();

	let sent = mock.take_sent();

	assert_eq!(sent.len(), 2);

	let (bits, reliable) = &sent[0];
	let mut reader = bits.reader();

	assert!(*reliable);
	assert_eq!(reader.read_ubits(MESSAGE_TYPE_BITS), Ok(7));
	assert_eq!(reader.read_cstring().as_deref(), Ok(c"hello"));
	assert_eq!(reader.remaining(), 0);
	assert!(!sent[1].1);
}
