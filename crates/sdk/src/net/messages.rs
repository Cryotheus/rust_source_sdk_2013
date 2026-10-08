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

use super::{EncodeError, MessageId, NetMessage, Reliability};
use crate::bitbuf::BitWriter;
use crate::math::{QAngle, Vector};
use sdk_raw::edicts::MAX_EDICT_BITS;
use std::ffi::CStr;

/// Bits in each of a [`FixAngle`]'s angles.
const ANGLE_BITS: u32 = 16;

/// Bits in a decal's texture index (`MAX_DECAL_INDEX_BITS`).
const DECAL_INDEX_BITS: u32 = 9;

/// The longest command a client accepts, less its terminator.
pub const MAX_COMMAND_LEN: usize = 1023;

/// The longest console variable name or value (`MAX_OSPATH`), less its
/// terminator.
pub const MAX_CONVAR_LEN: usize = sdk_raw::net::incoming::MAX_OSPATH - 1;

/// The largest payload of a user or entity message (`MAX_USER_MSG_DATA`).
#[doc(alias("MAX_USER_MSG_DATA"))]
pub const MAX_MESSAGE_DATA_BYTES: usize = 255;

/// The longest console text a client accepts, less its terminator.
pub const MAX_PRINT_LEN: usize = 2047;

/// The longest console variable name a query may name, less its terminator.
pub const MAX_QUERY_NAME_LEN: usize = 255;

/// The most voice a [`VoiceData`] carries, as its length in bits fits in 16
/// bits.
pub const MAX_VOICE_DATA_BYTES: usize = ((1 << VOICE_LENGTH_BITS) - 1) / 8;

/// Bits in a model index (`SP_MODEL_INDEX_BITS`).
const MODEL_INDEX_BITS: u32 = 13;

/// Bits in the length, in bits, of an embedded payload.
const PAYLOAD_LENGTH_BITS: u32 = 11;

/// Bits in a server class's ID (`MAX_SERVER_CLASS_BITS`).
const SERVER_CLASS_BITS: u32 = 9;

/// Bits in a sound's precache index (`MAX_SOUND_INDEX_BITS`).
const SOUND_INDEX_BITS: u32 = 14;

/// Bits in the length, in bits, of a [`VoiceData`]'s voice.
const VOICE_LENGTH_BITS: u32 = 16;

/// Places a decal on the world or a brush entity (`svc_BSPDecal`).
#[doc(alias("SVC_BSPDecal"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BspDecal {
	/// Where the decal is, in world coordinates.
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
			EncodeError::check_bits("entity", target.entity.into(), MAX_EDICT_BITS)?;
			EncodeError::check_bits("model", target.model.into(), MODEL_INDEX_BITS)?;
		}

		out.write_bit_vec3_coord(self.position);
		out.write_ubits(self.texture.into(), DECAL_INDEX_BITS);

		match self.target {
			Some(target) => {
				out.write_bit(true);
				out.write_ubits(target.entity.into(), MAX_EDICT_BITS);
				out.write_ubits(target.model.into(), MODEL_INDEX_BITS);
			}

			None => out.write_bit(false),
		}

		out.write_bit(self.low_priority);
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

/// A message for one entity's client-side class (`svc_EntityMessage`).
#[doc(alias("SVC_EntityMessage"))]
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
		EncodeError::check_bits("entity", self.entity.into(), MAX_EDICT_BITS)?;
		EncodeError::check_bits("class ID", self.class_id.into(), SERVER_CLASS_BITS)?;

		let mut body = BitWriter::new();

		body.write_ubits(self.entity.into(), MAX_EDICT_BITS);
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

/// Sets the client's view angles (`svc_FixAngle`), as teleports do.
#[doc(alias("SVC_FixAngle"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FixAngle {
	/// Adds the angles to the client's own instead of replacing them.
	pub relative: bool,

	/// The angles in degrees, each sent as a fraction of a turn in 16 bits.
	pub angles: QAngle,
}

impl NetMessage for FixAngle {
	fn id(&self) -> MessageId {
		MessageId::FIX_ANGLE
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		out.write_bit(self.relative);
		out.write_bit_angle(self.angles.pitch, ANGLE_BITS);
		out.write_bit_angle(self.angles.yaw, ANGLE_BITS);
		out.write_bit_angle(self.angles.roll, ANGLE_BITS);
		Ok(())
	}
}

/// A game event for this client alone (`svc_GameEvent`), as
/// [`GameEventManager::serialize_event`] encodes it.
///
/// [`GameEventManager::serialize_event`]: crate::interfaces::GameEventManager::serialize_event
#[doc(alias("SVC_GameEvent"))]
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

/// Asks the client for a console variable's value (`svc_GetCvarValue`).
///
/// The client answers with `clc_RespondCvarValue`, carrying the cookie.
/// [`PluginHelpers::start_query_cvar_value`] sends the same query with a
/// cookie the engine chooses, and reports the answer to server plugins.
///
/// [`PluginHelpers::start_query_cvar_value`]: crate::interfaces::PluginHelpers::start_query_cvar_value
#[doc(alias("SVC_GetCvarValue"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GetCvarValue<'a> {
	/// A number the client returns with its answer, to match it to the query.
	pub cookie: i32,

	/// The variable's name, at most [`MAX_QUERY_NAME_LEN`] bytes long.
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

/// Loads a precached sound ahead of its first use (`svc_Prefetch`).
#[doc(alias("SVC_Prefetch"))]
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

/// Prints text to the client's console (`svc_Print`).
#[doc(alias("SVC_Print"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Print<'a> {
	/// The text, at most [`MAX_PRINT_LEN`] bytes long.
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

/// Any message, from its type and encoded fields.
///
/// Nothing checks the fields, and a malformed message makes the client
/// disconnect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Raw<'a> {
	/// The message's type.
	pub id: MessageId,

	/// The message's encoded fields, which follow its type.
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

/// Sets the client's copies of replicated console variables
/// (`net_SetConVar`).
///
/// Clients only accept variables marked `FCVAR_REPLICATED`, and keep the
/// values until the server's own change or another message replaces them.
#[doc(alias("NET_SetConVar"))]
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

/// Shows or hides the client's paused screen (`svc_SetPause`).
///
/// Only the client's display changes: the server keeps simulating.
#[doc(alias("SVC_SetPause"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetPause {
	/// Shows the paused screen when true, and hides it when false.
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
#[doc(alias("SVC_SetView"))]
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
		EncodeError::check_bits("entity", self.entity.into(), MAX_EDICT_BITS)?;
		out.write_ubits(self.entity.into(), MAX_EDICT_BITS);
		Ok(())
	}
}

/// Runs a command on the client (`net_StringCmd`), as
/// [`PluginHelpers::client_command`] does.
///
/// Clients only run commands the server may execute, marked
/// `FCVAR_SERVER_CAN_EXECUTE`, and cheat commands only while their copy of
/// `sv_cheats` is set. They run them from their command buffer on their next
/// frame, not as the message arrives.
///
/// [`PluginHelpers::client_command`]: crate::interfaces::PluginHelpers::client_command
#[doc(alias("NET_StringCmd"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StringCmd<'a> {
	/// The command and its arguments, at most [`MAX_COMMAND_LEN`] bytes long.
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

/// A user message for this client alone (`svc_UserMessage`).
///
/// The type is the index of a user message the game registered, which
/// [`ServerGameDll::user_messages`] lists. Unlike
/// [`user_messages::send`], nothing checks the payload against the
/// size the game registered.
///
/// [`ServerGameDll::user_messages`]: crate::interfaces::ServerGameDll::user_messages
/// [`user_messages::send`]: crate::user_messages::send
#[doc(alias("SVC_UserMessage"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserMessage<'a> {
	/// The index of the user message the game registered.
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

/// Voice for the client to play as a player's (`svc_VoiceData`).
///
/// The engine relays the voice each player sends (`clc_VoiceData`) to every
/// client that hears them this way, with the speaker's slot. The voice is in
/// the codec the server named as the client connected (`svc_VoiceInit`); the
/// engine passes it along without decoding it.
///
/// Voice goes in the unreliable stream, as the engine sends it.
#[doc(alias("SVC_VoiceData"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceData<'a> {
	/// The speaker's player slot, from 0: its player's entity index less 1.
	pub speaker: u8,

	/// The engine's proximity flag, which it sets when the client hears the
	/// speaker only by proximity (`IClient::IsProximityHearingClient`).
	pub proximity: bool,

	/// The encoded voice, at most [`MAX_VOICE_DATA_BYTES`] bytes.
	pub data: &'a [u8],
}

impl NetMessage for VoiceData<'_> {
	fn id(&self) -> MessageId {
		MessageId::VOICE_DATA
	}

	fn reliability(&self) -> Reliability {
		Reliability::Unreliable
	}

	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError> {
		EncodeError::check_len("voice data", self.data.len(), MAX_VOICE_DATA_BYTES)?;
		out.write_u8(self.speaker);
		out.write_u8(self.proximity.into());
		out.write_u16((self.data.len() * 8) as u16);
		out.write_bytes(self.data);
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
