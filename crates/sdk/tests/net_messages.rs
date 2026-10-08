//! Tests of the wire encodings of the messages the server sends clients,
//! which must match what the engine writes and the client parses.

use source_sdk_2013::bitbuf::{BitReader, BitWriter};
use source_sdk_2013::math::{QAngle, Vector};
use source_sdk_2013::net::messages::{
	BspDecal, DecalTarget, EntityMessage, FixAngle, GameEvent, GetCvarValue, Prefetch, Print,
	SetConVar, SetPause, SetView, StringCmd, UserMessage,
};
use source_sdk_2013::net::{MESSAGE_TYPE_BITS, MessageId, NetMessage};

#[test]
fn decals_on_entities_carry_their_model() {
	let decal = BspDecal {
		position: Vector::new(1.0, -2.5, 0.0),
		texture: 300,
		target: Some(DecalTarget {
			entity: 2000,
			model: 8000,
		}),
		low_priority: true,
	};
	let (bits, at) = fields(&decal, MessageId::BSP_DECAL);
	let mut reader = reader_at(&bits, at);

	assert_eq!(reader.read_bit_vec3_coord(), Ok(decal.position));
	assert_eq!(reader.read_ubits(9), Ok(300));
	assert_eq!(reader.read_bit(), Ok(true));
	assert_eq!(reader.read_ubits(11), Ok(2000));
	assert_eq!(reader.read_ubits(13), Ok(8000));
	assert_eq!(reader.read_bit(), Ok(true));
	assert_eq!(reader.remaining(), 0);

	let world = BspDecal {
		target: None,
		low_priority: false,
		..decal
	};
	let (bits, at) = fields(&world, MessageId::BSP_DECAL);
	let mut reader = reader_at(&bits, at);

	reader.read_bit_vec3_coord().unwrap();
	reader.read_ubits(9).unwrap();
	assert_eq!(reader.read_bit(), Ok(false));
	assert_eq!(reader.read_bit(), Ok(false));
	assert_eq!(reader.remaining(), 0);
}

/// Encodes a message and checks its type, returning a reader at its fields.
fn fields(message: &impl NetMessage, id: MessageId) -> (BitWriter, usize) {
	let bits = message.encode().unwrap();
	let mut reader = bits.reader();

	assert_eq!(reader.read_ubits(MESSAGE_TYPE_BITS), Ok(id.get().into()));

	let position = reader.position();

	(bits, position)
}

#[test]
fn payloads_are_prefixed_with_their_length_in_bits() {
	let mut data = BitWriter::new();

	data.write_ubits(0b10110, 5);

	let (bits, at) = fields(
		&UserMessage {
			message_type: 11,
			data: &data,
		},
		MessageId::USER_MESSAGE,
	);
	let mut reader = reader_at(&bits, at);

	assert_eq!(reader.read_u8(), Ok(11));
	assert_eq!(reader.read_ubits(11), Ok(5));
	assert_eq!(reader.read_ubits(5), Ok(0b10110));
	assert_eq!(reader.remaining(), 0);

	let (bits, at) = fields(
		&EntityMessage {
			entity: 7,
			class_id: 300,
			data: &data,
		},
		MessageId::ENTITY_MESSAGE,
	);
	let mut reader = reader_at(&bits, at);

	assert_eq!(reader.read_ubits(11), Ok(7));
	assert_eq!(reader.read_ubits(9), Ok(300));
	assert_eq!(reader.read_ubits(11), Ok(5));
	assert_eq!(reader.read_ubits(5), Ok(0b10110));

	let (bits, at) = fields(&GameEvent { data: &data }, MessageId::GAME_EVENT);
	let mut reader = reader_at(&bits, at);

	assert_eq!(reader.read_ubits(11), Ok(5));
	assert_eq!(reader.read_ubits(5), Ok(0b10110));
}

/// A reader of `bits` past their first `position` bits.
fn reader_at(bits: &BitWriter, position: usize) -> BitReader<'_> {
	let mut reader = bits.reader();

	reader.read_bits(position).unwrap();
	reader
}

#[test]
fn small_messages() {
	let (bits, at) = fields(&SetPause { paused: true }, MessageId::SET_PAUSE);

	assert_eq!(bits.len(), at + 1);

	let (bits, at) = fields(&Prefetch { sound: 16383 }, MessageId::PREFETCH);

	assert_eq!(reader_at(&bits, at).read_ubits(14), Ok(16383));

	let (bits, at) = fields(
		&GetCvarValue {
			cookie: -3,
			name: c"cl_downloadfilter",
		},
		MessageId::GET_CVAR_VALUE,
	);
	let mut reader = reader_at(&bits, at);

	assert_eq!(reader.read_i32(), Ok(-3));
	assert_eq!(reader.read_cstring().as_deref(), Ok(c"cl_downloadfilter"));

	let (bits, at) = fields(&Print { text: c"boo" }, MessageId::PRINT);

	assert_eq!(reader_at(&bits, at).read_cstring().as_deref(), Ok(c"boo"));
}

#[test]
fn string_commands_and_convars() {
	let (bits, at) = fields(
		&StringCmd {
			command: c"cl_soundscape_flush",
		},
		MessageId::STRING_CMD,
	);
	let mut reader = reader_at(&bits, at);

	assert_eq!(reader.read_cstring().as_deref(), Ok(c"cl_soundscape_flush"));
	assert_eq!(reader.remaining(), 0);

	let convars = [(c"sv_cheats", c"1"), (c"mp_friendlyfire", c"0")];
	let (bits, at) = fields(&SetConVar { convars: &convars }, MessageId::SET_CONVAR);
	let mut reader = reader_at(&bits, at);

	assert_eq!(reader.read_u8(), Ok(2));

	for (name, value) in convars {
		assert_eq!(reader.read_cstring().as_deref(), Ok(name));
		assert_eq!(reader.read_cstring().as_deref(), Ok(value));
	}

	assert_eq!(reader.remaining(), 0);
}

#[test]
fn views_and_angles() {
	let (bits, at) = fields(&SetView { entity: 1234 }, MessageId::SET_VIEW);
	let mut reader = reader_at(&bits, at);

	assert_eq!(reader.read_ubits(11), Ok(1234));
	assert_eq!(reader.remaining(), 0);

	let angles = QAngle {
		pitch: 45.0,
		yaw: 180.0,
		roll: 0.0,
	};
	let (bits, at) = fields(
		&FixAngle {
			relative: true,
			angles,
		},
		MessageId::FIX_ANGLE,
	);
	let mut reader = reader_at(&bits, at);

	assert_eq!(reader.read_bit(), Ok(true));
	assert_eq!(reader.read_u16(), Ok(8192));
	assert_eq!(reader.read_u16(), Ok(32768));
	assert_eq!(reader.read_u16(), Ok(0));
	assert_eq!(reader.remaining(), 0);
}
