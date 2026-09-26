//! Messages the server sends clients through their [`NetChannel`]s.
//!
//! The fields follow TF2's protocol 24, as the engine's own messages encode
//! them. Each type checks its fields against the limits the client parses
//! them with, so a message this module encodes cannot make it disconnect.
//!
//! Messages the engine sends to keep a client's state, such as snapshots,
//! string tables, and the sign-on sequence, have no type here: sending one
//! out of turn desynchronizes the client. [`Raw`] sends anything else.
//!
//! [`NetChannel`]: super::NetChannel

use super::{EncodeError, MessageId, NetMessage};
use crate::bitbuf::BitWriter;
use crate::math::{QAngle, Vector};
use std::ffi::CStr;

/// The longest command a client accepts, less its terminator.
pub const MAX_COMMAND_LEN: usize = 1023;

/// The longest console variable name or value (`MAX_OSPATH`), less its
/// terminator.
pub const MAX_CONVAR_LEN: usize = 259;

/// The longest console text a client accepts, less its terminator.
pub const MAX_PRINT_LEN: usize = 2047;

/// The largest payload of a user or entity message (`MAX_USER_MSG_DATA`).
pub const MAX_MESSAGE_DATA_BYTES: usize = 255;

/// The longest console variable name a query may name, less its terminator.
pub const MAX_QUERY_NAME_LEN: usize = 255;

/// Bits in an entity index (`MAX_EDICT_BITS`).
const EDICT_BITS: u32 = 11;

/// Bits in a decal's texture index (`MAX_DECAL_INDEX_BITS`).
const DECAL_INDEX_BITS: u32 = 9;

/// Bits in a model index (`SP_MODEL_INDEX_BITS`).
const MODEL_INDEX_BITS: u32 = 13;

/// Bits in a server class's ID (`MAX_SERVER_CLASS_BITS`).
const SERVER_CLASS_BITS: u32 = 9;

/// Bits in a sound's precache index (`MAX_SOUND_INDEX_BITS`).
const SOUND_INDEX_BITS: u32 = 14;

/// Bits in the length, in bits, of an embedded payload.
const PAYLOAD_LENGTH_BITS: u32 = 11;

/// Runs a command on the client (`net_StringCmd`), as
/// [`PluginHelpers::client_command`] does.
///
/// Clients only run commands the server may execute, marked
/// `FCVAR_SERVER_CAN_EXECUTE`, and cheat commands only while their copy of
/// `sv_cheats` is set. They run them from their command buffer on their next
/// frame, not as the message arrives.
///
/// [`PluginHelpers::client_command`]: crate::interfaces::PluginHelpers::client_command
#[doc(alias = "NET_StringCmd")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StringCmd<'a> {
	pub command: &'a CStr,
}

impl NetMessage for StringCmd<'_> {
	fn id(&self) -> MessageId {
		MessageId::STRING_CMD
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		EncodeError::check_len("command", self.command.count_bytes(), MAX_COMMAND_LEN)?;
		out.write_cstr(self.command);
		Ok(())
	}
}

/// Sets the client's copies of replicated console variables
/// (`net_SetConVar`).
///
/// Clients only accept variables marked `FCVAR_REPLICATED`, and keep the
/// values until the server's own change or another message replaces them.
#[doc(alias = "NET_SetConVar")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetConVar<'a> {
	/// Names and values; at most 255.
	pub convars: &'a [(&'a CStr, &'a CStr)],
}

impl NetMessage for SetConVar<'_> {
	fn id(&self) -> MessageId {
		MessageId::SET_CONVAR
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		let count = u8::try_from(self.convars.len()).map_err(|_| EncodeError::OutOfRange {
			field: "convar count",
			value: self.convars.len() as u64,
			max: u8::MAX.into(),
		})?;

		out.write_u8(count);

		for &(name, value) in self.convars {
			EncodeError::check_len("convar name", name.count_bytes(), MAX_CONVAR_LEN)?;
			EncodeError::check_len("convar value", value.count_bytes(), MAX_CONVAR_LEN)?;
			out.write_cstr(name);
			out.write_cstr(value);
		}

		Ok(())
	}
}

/// Prints text to the client's console (`svc_Print`).
#[doc(alias = "SVC_Print")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Print<'a> {
	pub text: &'a CStr,
}

impl NetMessage for Print<'_> {
	fn id(&self) -> MessageId {
		MessageId::PRINT
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		EncodeError::check_len("text", self.text.count_bytes(), MAX_PRINT_LEN)?;
		out.write_cstr(self.text);
		Ok(())
	}
}

/// Shows or hides the client's paused screen (`svc_SetPause`).
///
/// Only the client's display changes: the server keeps simulating.
#[doc(alias = "SVC_SetPause")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetPause {
	pub paused: bool,
}

impl NetMessage for SetPause {
	fn id(&self) -> MessageId {
		MessageId::SET_PAUSE
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		out.write_bit(self.paused);
		Ok(())
	}
}

/// Renders the client's view from an entity (`svc_SetView`), as
/// `IVEngineServer::SetView` does.
#[doc(alias = "SVC_SetView")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetView {
	/// An entity index below 2048.
	pub entity: u16,
}

impl NetMessage for SetView {
	fn id(&self) -> MessageId {
		MessageId::SET_VIEW
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		EncodeError::check_bits("entity", self.entity.into(), EDICT_BITS)?;
		out.write_ubits(self.entity.into(), EDICT_BITS);
		Ok(())
	}
}

/// Sets the client's view angles (`svc_FixAngle`), as teleports do.
#[doc(alias = "SVC_FixAngle")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixAngle {
	/// Adds the angles to the client's own instead of replacing them.
	pub relative: bool,
	pub angles: QAngle,
}

impl NetMessage for FixAngle {
	fn id(&self) -> MessageId {
		MessageId::FIX_ANGLE
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		out.write_bit(self.relative);
		out.write_bit_angle(self.angles.pitch, 16);
		out.write_bit_angle(self.angles.yaw, 16);
		out.write_bit_angle(self.angles.roll, 16);
		Ok(())
	}
}

/// The entity a [`BspDecal`] is applied to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecalTarget {
	/// An entity index below 2048.
	pub entity: u16,

	/// The entity's model index, below 8192.
	pub model: u16,
}

/// Places a decal on the world or a brush entity (`svc_BSPDecal`).
#[doc(alias = "SVC_BSPDecal")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BspDecal {
	pub position: Vector,

	/// The decal's precache index, below 512.
	pub texture: u16,

	/// The entity the decal is on, or the world.
	pub target: Option<DecalTarget>,

	/// Lets the client drop the decal first when it has too many.
	pub low_priority: bool,
}

impl NetMessage for BspDecal {
	fn id(&self) -> MessageId {
		MessageId::BSP_DECAL
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		EncodeError::check_bits("texture", self.texture.into(), DECAL_INDEX_BITS)?;

		if let Some(target) = self.target {
			EncodeError::check_bits("entity", target.entity.into(), EDICT_BITS)?;
			EncodeError::check_bits("model", target.model.into(), MODEL_INDEX_BITS)?;
		}

		out.write_bit_vec3_coord(self.position);
		out.write_ubits(self.texture.into(), DECAL_INDEX_BITS);

		match self.target {
			Some(target) => {
				out.write_bit(true);
				out.write_ubits(target.entity.into(), EDICT_BITS);
				out.write_ubits(target.model.into(), MODEL_INDEX_BITS);
			}
			None => out.write_bit(false),
		}

		out.write_bit(self.low_priority);
		Ok(())
	}
}

/// Checks a payload's size, then writes its length in bits and its bits.
fn write_payload(
	out: &mut BitWriter,
	field: &'static str,
	data: &BitWriter,
	max_bytes: usize,
) -> Result<(), EncodeError> {
	EncodeError::check_len(field, data.byte_len(), max_bytes)?;
	EncodeError::check_bits(field, data.len() as u64, PAYLOAD_LENGTH_BITS)?;
	out.write_ubits(data.len() as u32, PAYLOAD_LENGTH_BITS);
	out.write_bits(data);
	Ok(())
}

/// A user message for this client alone (`svc_UserMessage`).
///
/// The type is the index of a user message the game registered, which
/// [`ServerGameDll::user_messages`] lists. Unlike
/// [`user_messages::send`], nothing checks the payload against the
/// size the game registered.
///
/// [`ServerGameDll::user_messages`]: crate::interfaces::ServerGameDll::user_messages
/// [`user_messages::send`]: crate::user_messages::send
#[doc(alias = "SVC_UserMessage")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserMessage<'a> {
	pub message_type: u8,

	/// At most [`MAX_MESSAGE_DATA_BYTES`] bytes.
	pub data: &'a BitWriter,
}

impl NetMessage for UserMessage<'_> {
	fn id(&self) -> MessageId {
		MessageId::USER_MESSAGE
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		let mut body = BitWriter::new();

		body.write_u8(self.message_type);
		write_payload(
			&mut body,
			"user message data",
			self.data,
			MAX_MESSAGE_DATA_BYTES,
		)?;
		out.write_bits(&body);
		Ok(())
	}
}

/// A message for one entity's client-side class (`svc_EntityMessage`).
#[doc(alias = "SVC_EntityMessage")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityMessage<'a> {
	/// An entity index below 2048.
	pub entity: u16,

	/// The ID of the entity's server class, below 512.
	pub class_id: u16,

	/// At most [`MAX_MESSAGE_DATA_BYTES`] bytes.
	pub data: &'a BitWriter,
}

impl NetMessage for EntityMessage<'_> {
	fn id(&self) -> MessageId {
		MessageId::ENTITY_MESSAGE
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		EncodeError::check_bits("entity", self.entity.into(), EDICT_BITS)?;
		EncodeError::check_bits("class ID", self.class_id.into(), SERVER_CLASS_BITS)?;

		let mut body = BitWriter::new();

		body.write_ubits(self.entity.into(), EDICT_BITS);
		body.write_ubits(self.class_id.into(), SERVER_CLASS_BITS);
		write_payload(
			&mut body,
			"entity message data",
			self.data,
			MAX_MESSAGE_DATA_BYTES,
		)?;
		out.write_bits(&body);
		Ok(())
	}
}

/// A game event for this client alone (`svc_GameEvent`), as
/// [`GameEventManager::serialize_event`] encodes it.
///
/// [`GameEventManager::serialize_event`]: crate::interfaces::GameEventManager::serialize_event
#[doc(alias = "SVC_GameEvent")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameEvent<'a> {
	/// The event's ID and fields, in fewer than 2048 bits.
	pub data: &'a BitWriter,
}

impl NetMessage for GameEvent<'_> {
	fn id(&self) -> MessageId {
		MessageId::GAME_EVENT
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		write_payload(out, "game event data", self.data, usize::MAX)
	}
}

/// Loads a precached sound ahead of its first use (`svc_Prefetch`).
#[doc(alias = "SVC_Prefetch")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefetch {
	/// The sound's precache index, below 16384.
	pub sound: u16,
}

impl NetMessage for Prefetch {
	fn id(&self) -> MessageId {
		MessageId::PREFETCH
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		EncodeError::check_bits("sound", self.sound.into(), SOUND_INDEX_BITS)?;
		out.write_ubits(self.sound.into(), SOUND_INDEX_BITS);
		Ok(())
	}
}

/// Asks the client for a console variable's value (`svc_GetCvarValue`).
///
/// The client answers with `clc_RespondCvarValue`, carrying the cookie.
/// [`PluginHelpers::start_query_cvar_value`] sends the same query with a
/// cookie the engine chooses, and reports the answer to server plugins.
///
/// [`PluginHelpers::start_query_cvar_value`]: crate::interfaces::PluginHelpers::start_query_cvar_value
#[doc(alias = "SVC_GetCvarValue")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetCvarValue<'a> {
	pub cookie: i32,
	pub name: &'a CStr,
}

impl NetMessage for GetCvarValue<'_> {
	fn id(&self) -> MessageId {
		MessageId::GET_CVAR_VALUE
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		EncodeError::check_len("name", self.name.count_bytes(), MAX_QUERY_NAME_LEN)?;
		out.write_i32(self.cookie);
		out.write_cstr(self.name);
		Ok(())
	}
}

/// Any message, from its type and encoded fields.
///
/// Nothing checks the fields, and a malformed message makes the client
/// disconnect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Raw<'a> {
	pub id: MessageId,
	pub body: &'a BitWriter,
}

impl NetMessage for Raw<'_> {
	fn id(&self) -> MessageId {
		self.id
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		out.write_bits(self.body);
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::bitbuf::BitReader;
	use crate::net::MESSAGE_TYPE_BITS;

	/// Encodes a message and checks its type, returning a reader at its fields.
	fn fields(message: &impl NetMessage, id: MessageId) -> (BitWriter, usize) {
		let bits = message.encode().unwrap();
		let mut reader = bits.reader();

		assert_eq!(reader.read_ubits(MESSAGE_TYPE_BITS), Ok(id.get().into()));

		let position = reader.position();

		(bits, position)
	}

	fn reader_at(bits: &BitWriter, position: usize) -> BitReader<'_> {
		let mut reader = bits.reader();

		reader.read_bits(position).unwrap();
		reader
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
	fn limits_are_checked_before_writing() {
		let long = std::ffi::CString::new(vec![b'a'; MAX_COMMAND_LEN + 1]).unwrap();

		assert!(matches!(
			StringCmd { command: &long }.encode(),
			Err(EncodeError::TooLong {
				max: MAX_COMMAND_LEN,
				..
			})
		));

		let many = vec![(c"a", c"b"); 256];

		assert!(matches!(
			SetConVar { convars: &many }.encode(),
			Err(EncodeError::OutOfRange { max: 255, .. })
		));
		assert!(matches!(
			SetView { entity: 2048 }.encode(),
			Err(EncodeError::OutOfRange { max: 2047, .. })
		));

		let mut data = BitWriter::new();

		data.write_bytes(&[0; MAX_MESSAGE_DATA_BYTES + 1]);

		assert!(matches!(
			UserMessage {
				message_type: 1,
				data: &data,
			}
			.encode(),
			Err(EncodeError::TooLong { .. })
		));
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

		let mut large = BitWriter::new();

		large.write_bytes(&[0; 256]);

		assert!(GameEvent { data: &large }.encode().is_err());
	}

	#[test]
	fn small_messages() {
		let (bits, at) = fields(&SetPause { paused: true }, MessageId::SET_PAUSE);

		assert_eq!(bits.len(), at + 1);

		let (bits, at) = fields(&Prefetch { sound: 16383 }, MessageId::PREFETCH);

		assert_eq!(reader_at(&bits, at).read_ubits(14), Ok(16383));
		assert!(Prefetch { sound: 16384 }.encode().is_err());

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
}
