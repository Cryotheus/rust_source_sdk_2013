//! Tests of sending messages through a client's net channel: what the
//! engine's `SendData` receives, and how its refusal is reported.

use super::*;
use crate::net::messages::{MAX_VOICE_DATA_BYTES, Print, Raw, VoiceData};
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

#[test]
fn voice_is_sent_unreliably_as_the_speaker_then_its_length_in_bits() {
	let mock = MockChannel::new();
	let voice = VoiceData {
		speaker: 3,
		proximity: true,
		data: &[0xAB, 0xCD],
	};

	mock.channel().send(&voice).unwrap();

	let sent = mock.take_sent();
	let (bits, reliable) = &sent[0];
	let mut reader = bits.reader();

	assert!(!*reliable);
	assert_eq!(reader.read_ubits(MESSAGE_TYPE_BITS), Ok(15));
	assert_eq!(reader.read_u8(), Ok(3));
	assert_eq!(reader.read_u8(), Ok(1));
	assert_eq!(reader.read_u16(), Ok(16));
	assert_eq!(reader.read_bytes(2).as_deref(), Ok(&[0xAB, 0xCD][..]));
	assert_eq!(reader.remaining(), 0);
}

#[test]
fn voice_longer_than_its_length_field_is_refused() {
	let data = vec![0; MAX_VOICE_DATA_BYTES + 1];
	let voice = VoiceData {
		speaker: 0,
		proximity: false,
		data: &data,
	};

	assert!(matches!(
		voice.encode(),
		Err(EncodeError::TooLong { max: MAX_VOICE_DATA_BYTES, .. })
	));
	assert!(
		VoiceData {
			data: &data[..MAX_VOICE_DATA_BYTES],
			..voice
		}
		.encode()
		.is_ok()
	);
}
