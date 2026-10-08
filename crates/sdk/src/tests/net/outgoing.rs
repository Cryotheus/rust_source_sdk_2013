//! Tests of walking encoded messages, which this crate's encoders and the
//! tests themselves write, and of the engine's message objects, through mock
//! messages.

use super::*;
use crate::math::{QAngle, Vector};
use crate::net::NetMessage;

use crate::net::messages::{
	BspDecal, DecalTarget, EntityMessage, FixAngle, GameEvent, GetCvarValue, Prefetch, Print,
	SetConVar, SetPause, SetView, StringCmd, UserMessage, VoiceData,
};

use crate::test_support::net::MockMessage;

#[test]
fn a_message_cut_short_ends_the_walk_unparsed() {
	let first = Print { text: c"first" }.encode().unwrap();

	// A payload longer than the bits left.
	let mut payload = message(MessageId::USER_MESSAGE);

	payload.write_u8(4);
	payload.write_ubits(100, PAYLOAD_LENGTH_BITS);
	payload.write_bits(&data(99));

	// Text without its terminator.
	let mut text = message(MessageId::PRINT);

	text.write_bytes(b"no end");

	// Entities' data longer than the bits left.
	let mut entities = message(MessageId::PACKET_ENTITIES);

	entities.write_ubits(1000, MAX_EDICT_BITS);
	entities.write_bit(false);
	entities.write_bit(false);
	entities.write_ubits(3, MAX_EDICT_BITS);
	entities.write_ubits(500, DELTA_SIZE_BITS);
	entities.write_bit(false);
	entities.write_bits(&data(20));

	let cut = [
		(MessageId::USER_MESSAGE, payload),
		(MessageId::PRINT, text),
		(MessageId::PACKET_ENTITIES, entities),
	];

	for (id, bits) in cut {
		let stream = concat([&first, &bits]);
		let mut messages = walk(stream.reader());

		assert_eq!(
			messages.next(),
			Some(EncodedMessage {
				id: MessageId::PRINT,
				bits: first.len(),
				details: Details::Print {
					text: c"first".to_owned(),
				},
			})
		);

		assert_eq!(
			messages.next(),
			Some(EncodedMessage {
				id,
				bits: bits.len(),
				details: Details::Unparsed,
			})
		);

		assert_eq!(messages.remaining(), 0);
		assert_eq!(messages.next(), None);
	}
}

#[test]
fn connection_messages_are_passed_over_whole() {
	let mut file = message(MessageId::FILE);

	file.write_u32(77);
	file.write_cstr(c"maps/cp_badlands.bsp");
	file.write_bit(false);

	let mut signon_state = message(MessageId::SIGNON_STATE);

	signon_state.write_u8(2);
	signon_state.write_i32(3);

	let mut server_info = message(MessageId::SERVER_INFO);

	// The protocol, the server count, SourceTV, a dedicated server, the CRC
	// and the classes.
	server_info.write_u16(24);
	server_info.write_u32(3);
	server_info.write_bit(false);
	server_info.write_bit(true);
	server_info.write_u32(u32::MAX);
	server_info.write_u16(350);

	// The map's MD5, the slot, the most players, the tick interval and the
	// platform.
	server_info.write_bytes(&[0xab; 16]);
	server_info.write_u8(1);
	server_info.write_u8(24);
	server_info.write_f32(0.015);
	server_info.write_u8(b'l');
	server_info.write_cstr(c"tf");
	server_info.write_cstr(c"cp_badlands");
	server_info.write_cstr(c"sky_badlands_01");
	server_info.write_cstr(c"A server");

	// A replay server.
	server_info.write_bit(false);

	// Classes the client creates itself.
	let mut class_info = message(MessageId::CLASS_INFO);

	class_info.write_u16(350);
	class_info.write_bit(true);

	// A table of up to 8192 entries, whose count takes 14 bits.
	let mut table = message(MessageId::CREATE_STRING_TABLE);

	table.write_cstr(c"downloadables");
	table.write_u16(8192);
	table.write_ubits(5, 14);
	table.write_var_u32(700);
	table.write_bit(false);
	table.write_bit(false);
	table.write_bits(&data(700));

	// A table of up to 64 entries, with a fixed size of compressed user data.
	let mut fixed_table = message(MessageId::CREATE_STRING_TABLE);

	fixed_table.write_cstr(c"lightstyles");
	fixed_table.write_u16(64);
	fixed_table.write_ubits(64, 7);
	fixed_table.write_var_u32(30);
	fixed_table.write_bit(true);
	fixed_table.write_ubits(1, 12);
	fixed_table.write_ubits(2, 4);
	fixed_table.write_bit(true);
	fixed_table.write_bits(&data(30));

	// A quality of 255, followed by the sample rate.
	let mut voice_init = message(MessageId::VOICE_INIT);

	voice_init.write_cstr(c"steam");
	voice_init.write_u8(255);
	voice_init.write_u16(24000);

	let mut legacy_voice_init = message(MessageId::VOICE_INIT);

	legacy_voice_init.write_cstr(c"vaudio_speex");
	legacy_voice_init.write_u8(5);

	let mut menu = message(MessageId::MENU);

	menu.write_u16(2);
	menu.write_u16(3);
	menu.write_bytes(&[1, 2, 3]);

	let mut event_list = message(MessageId::GAME_EVENT_LIST);

	event_list.write_ubits(400, MAX_EVENT_BITS);
	event_list.write_ubits(5000, EVENT_LIST_LENGTH_BITS);
	event_list.write_bits(&data(5000));

	let mut key_values = message(MessageId::CMD_KEY_VALUES);

	key_values.write_u32(4);
	key_values.write_bytes(&[0, 1, 2, 3]);

	let messages = [
		message(MessageId::NOP),
		file,
		signon_state,
		server_info,
		class_info,
		table,
		fixed_table,
		voice_init,
		legacy_voice_init,
		menu,
		event_list,
		key_values,
	];

	let expected: Vec<_> = messages
		.iter()
		.map(|bits| (type_of(bits), bits.len(), Details::Other))
		.collect();

	assert_eq!(walked(&concat(&messages)), expected);
}

#[test]
fn engine_messages_report_their_type_name_and_stream() {
	let mut payload = BitWriter::new();

	payload.write_u8(1);

	let bits = UserMessage {
		message_type: 4,
		data: &payload,
	}
	.encode()
	.unwrap();

	let group = sys::INetChannelInfo_USERMESSAGES as c_int;
	let mock = MockMessage::new(23, group, c"svc_UserMessage", bits);
	let message = outgoing(&mock);

	assert_eq!(message.id(), Some(MessageId::USER_MESSAGE));
	assert_eq!(message.name().as_deref(), Some(c"svc_UserMessage"));
	assert_eq!(message.group(), group);
	assert_eq!(
		message.describe().as_deref(),
		Some(c"svc_UserMessage: 33 bits")
	);

	assert!(message.is_reliable());
	mock.make_unreliable();
	assert!(!message.is_reliable());

	// Nothing was written.
	assert_eq!(mock.writes(), 0);
}

#[test]
fn engine_messages_encode_as_they_write_themselves() {
	let bits = Print { text: c"hello" }.encode().unwrap();
	let mock = MockMessage::new(7, 0, c"svc_Print", bits.clone());
	let encoded = outgoing(&mock).encode().unwrap();

	assert_eq!(encoded, bits);
	assert_eq!(mock.writes(), 1);

	assert_eq!(
		walked(&encoded),
		[(
			MessageId::PRINT,
			bits.len(),
			Details::Print {
				text: c"hello".to_owned(),
			},
		)]
	);
}

#[test]
fn engine_messages_too_large_for_the_first_buffer_are_written_again() {
	// 20 KiB of data, more than the first buffer holds.
	let mut large = message(MessageId::CMD_KEY_VALUES);

	large.write_u32(20 * 1024);
	large.write_bytes(&vec![0x5a; 20 * 1024]);

	let mock = MockMessage::new(32, 0, c"svc_CmdKeyValues", large.clone());

	assert_eq!(outgoing(&mock).encode(), Some(large));
	assert_eq!(mock.writes(), 2);

	// More than the 512 KiB the second buffer holds.
	let mut huge = message(MessageId::CMD_KEY_VALUES);

	huge.write_u32(512 * 1024);
	huge.write_bytes(&vec![0; 512 * 1024]);

	let mock = MockMessage::new(32, 0, c"svc_CmdKeyValues", huge);

	assert_eq!(outgoing(&mock).encode(), None);
	assert_eq!(mock.writes(), 2);
}

#[test]
fn engine_messages_that_fail_to_write_or_have_no_type_are_reported() {
	let mock = MockMessage::new(7, 0, c"svc_Print", message(MessageId::PRINT));

	mock.fail();
	assert_eq!(outgoing(&mock).encode(), None);
	assert_eq!(mock.writes(), 1);

	for id in [-1, 64] {
		let mock = MockMessage::new(id, 0, c"unknown", BitWriter::new());

		assert_eq!(outgoing(&mock).id(), None);
	}
}

#[test]
fn fewer_bits_than_a_type_are_left_unwalked() {
	let mut stream = SetPause { paused: true }.encode().unwrap();

	// Packets are padded to whole bytes.
	stream.write_ubits(0, 5);

	let mut messages = walk(stream.reader());

	assert_eq!(
		messages.next(),
		Some(EncodedMessage {
			id: MessageId::SET_PAUSE,
			bits: 7,
			details: Details::Other,
		})
	);

	assert_eq!(messages.next(), None);
	assert_eq!(messages.remaining(), 5);
	assert_eq!(walk(BitWriter::new().reader()).next(), None);
}

#[test]
fn messages_this_crate_encodes_are_walked_as_written() {
	let mut payload = BitWriter::new();

	payload.write_u8(7);
	payload.write_ubits(5, 3);

	let messages: [&dyn NetMessage; 14] = [
		&Print { text: c"hello" },
		&UserMessage {
			message_type: 4,
			data: &payload,
		},
		&GameEvent { data: &payload },
		&EntityMessage {
			entity: 1500,
			class_id: 300,
			data: &payload,
		},
		&SetView { entity: 3 },
		&FixAngle {
			relative: true,
			angles: QAngle {
				pitch: 10.0,
				yaw: 90.0,
				roll: 0.0,
			},
		},
		&BspDecal {
			position: Vector::new(1.0, -2.0, 3.5),
			texture: 12,
			target: Some(DecalTarget {
				entity: 40,
				model: 2,
			}),
			low_priority: true,
		},
		&BspDecal {
			position: Vector::new(0.0, 0.0, 0.0),
			texture: 511,
			target: None,
			low_priority: false,
		},
		&Prefetch { sound: 9000 },
		&SetPause { paused: true },
		&SetConVar {
			convars: &[(c"sv_cheats", c"1"), (c"mp_tournament", c"0")],
		},
		&StringCmd {
			command: c"echo hi",
		},
		&GetCvarValue {
			cookie: -5,
			name: c"cl_interp",
		},
		&VoiceData {
			speaker: 2,
			proximity: true,
			data: &[1, 2, 3],
		},
	];

	let encoded: Vec<_> = messages
		.iter()
		.map(|message| message.encode().unwrap())
		.collect();

	let found = walked(&concat(&encoded));

	assert_eq!(
		found
			.iter()
			.map(|(id, bits, _)| (*id, *bits))
			.collect::<Vec<_>>(),
		messages
			.iter()
			.zip(&encoded)
			.map(|(message, bits)| (message.id(), bits.len()))
			.collect::<Vec<_>>()
	);

	let details: Vec<_> = found.into_iter().map(|(_, _, details)| details).collect();

	assert_eq!(
		details[..4],
		[
			Details::Print {
				text: c"hello".to_owned(),
			},
			Details::UserMessage {
				message_type: 4,
				data: payload.clone(),
			},
			Details::GameEvent {
				data: payload.clone(),
			},
			Details::EntityMessage {
				entity: 1500,
				class_id: 300,
				data: payload,
			},
		]
	);

	assert!(
		details[4..]
			.iter()
			.all(|details| *details == Details::Other)
	);
}

#[test]
fn snapshot_messages_are_walked_with_their_counts() {
	let mut tick = message(MessageId::TICK);

	// The tick, the frame time and its deviation.
	tick.write_i32(123_456);
	tick.write_u16(1500);
	tick.write_u16(20);

	// One entry of table 13 changed.
	let mut one_string = message(MessageId::UPDATE_STRING_TABLE);

	one_string.write_ubits(13, TABLE_ID_BITS);
	one_string.write_bit(false);
	one_string.write_ubits(70, STRING_TABLE_LENGTH_BITS);
	one_string.write_bits(&data(70));

	// 300 entries of table 31 changed.
	let mut strings = message(MessageId::UPDATE_STRING_TABLE);

	strings.write_ubits(31, TABLE_ID_BITS);
	strings.write_bit(true);
	strings.write_u16(300);
	strings.write_ubits(1000, STRING_TABLE_LENGTH_BITS);
	strings.write_bits(&data(1000));

	// 25 entities updated from tick 123400.
	let mut delta = message(MessageId::PACKET_ENTITIES);

	delta.write_ubits(1700, MAX_EDICT_BITS);
	delta.write_bit(true);
	delta.write_i32(123_400);
	delta.write_bit(false);
	delta.write_ubits(25, MAX_EDICT_BITS);
	delta.write_ubits(900, DELTA_SIZE_BITS);
	delta.write_bit(false);
	delta.write_bits(&data(900));

	// 400 entities in full, which update the client's baseline.
	let mut full = message(MessageId::PACKET_ENTITIES);

	full.write_ubits(1700, MAX_EDICT_BITS);
	full.write_bit(false);
	full.write_bit(true);
	full.write_ubits(400, MAX_EDICT_BITS);
	full.write_ubits(5000, DELTA_SIZE_BITS);
	full.write_bit(true);
	full.write_bits(&data(5000));

	let mut temp_entities = message(MessageId::TEMP_ENTITIES);

	temp_entities.write_u8(3);
	temp_entities.write_var_u32(200);
	temp_entities.write_bits(&data(200));

	// A count of 0 stands for one reliable temporary entity.
	let mut reliable_temp_entity = message(MessageId::TEMP_ENTITIES);

	reliable_temp_entity.write_u8(0);
	reliable_temp_entity.write_var_u32(50);
	reliable_temp_entity.write_bits(&data(50));

	let mut sounds = message(MessageId::SOUNDS);

	sounds.write_bit(false);
	sounds.write_u8(4);
	sounds.write_ubits(300, SOUNDS_LENGTH_BITS);
	sounds.write_bits(&data(300));

	let mut reliable_sound = message(MessageId::SOUNDS);

	reliable_sound.write_bit(true);
	reliable_sound.write_ubits(90, SOUND_LENGTH_BITS);
	reliable_sound.write_bits(&data(90));

	let messages = [
		(tick, Details::Tick { tick: 123_456 }),
		(
			one_string,
			Details::UpdateStringTable {
				table: 13,
				changed: 1,
			},
		),
		(
			strings,
			Details::UpdateStringTable {
				table: 31,
				changed: 300,
			},
		),
		(
			delta,
			Details::PacketEntities {
				delta_from: Some(123_400),
				updated: 25,
			},
		),
		(
			full,
			Details::PacketEntities {
				delta_from: None,
				updated: 400,
			},
		),
		(
			temp_entities,
			Details::TempEntities {
				count: 3,
				reliable: false,
			},
		),
		(
			reliable_temp_entity,
			Details::TempEntities {
				count: 1,
				reliable: true,
			},
		),
		(
			sounds,
			Details::Sounds {
				count: 4,
				reliable: false,
			},
		),
		(
			reliable_sound,
			Details::Sounds {
				count: 1,
				reliable: true,
			},
		),
	];

	let mut stream = concat(messages.iter().map(|(bits, _)| bits));

	// The packet's padding.
	stream.write_ubits(0, 3);

	let expected: Vec<_> = messages
		.iter()
		.map(|(bits, details)| (type_of(bits), bits.len(), details.clone()))
		.collect();

	assert_eq!(walked(&stream), expected);
}

#[test]
fn types_without_a_layout_end_the_walk_unparsed() {
	let after = Print { text: c"after" }.encode().unwrap();

	// Classes the client does not create itself, which follow in a layout the
	// walk does not read.
	let mut class_info = message(MessageId::CLASS_INFO);

	class_info.write_u16(2);
	class_info.write_bit(false);
	class_info.write_ubits(0, 2);
	class_info.write_cstr(c"CWorld");
	class_info.write_cstr(c"DT_World");

	let mut unknown = vec![class_info];

	// `net_Disconnect`, `svc_SendTable`, the unused 16, `svc_CrosshairAngle`,
	// the unused 22, and numbers past the last type.
	for id in [1, 9, 16, 20, 22, 33, 63] {
		let mut bits = message(MessageId::new(id).unwrap());

		bits.write_u32(0x1234_5678);
		unknown.push(bits);
	}

	for bits in unknown {
		let stream = concat([&bits, &after]);
		let mut messages = walk(stream.reader());

		// The message after it cannot be found, so it takes the rest.
		assert_eq!(
			messages.next(),
			Some(EncodedMessage {
				id: type_of(&bits),
				bits: stream.len(),
				details: Details::Unparsed,
			})
		);

		assert_eq!(messages.remaining(), 0);
		assert_eq!(messages.next(), None);
	}
}

/// Writes `messages` one after another.
fn concat<'a>(messages: impl IntoIterator<Item = &'a BitWriter>) -> BitWriter {
	let mut out = BitWriter::new();

	for message in messages {
		out.write_bits(message);
	}

	out
}

/// `len` bits of data, alternately set and clear.
fn data(len: usize) -> BitWriter {
	let mut out = BitWriter::new();

	for index in 0..len {
		out.write_bit(index % 2 == 0);
	}

	out
}

/// A new message of type `id`, its fields to be written.
fn message(id: MessageId) -> BitWriter {
	let mut out = BitWriter::new();

	out.write_ubits(id.get().into(), MESSAGE_TYPE_BITS);
	out
}

/// The engine's message `mock` points to.
fn outgoing(mock: &MockMessage) -> OutgoingMessage<'_> {
	// SAFETY: The mock is leaked, and answers each call the wrapper makes.
	unsafe { OutgoingMessage::from_raw(NonNull::new(mock.as_ptr()).unwrap()) }
}

/// The type an encoded message starts with.
fn type_of(message: &BitWriter) -> MessageId {
	let id = message.reader().read_ubits(MESSAGE_TYPE_BITS).unwrap();

	MessageId::new(id as u8).unwrap()
}

/// The type, size and details of each message the walk finds in `stream`.
fn walked(stream: &BitWriter) -> Vec<(MessageId, usize, Details)> {
	walk(stream.reader())
		.map(|message| (message.id, message.bits, message.details))
		.collect()
}
