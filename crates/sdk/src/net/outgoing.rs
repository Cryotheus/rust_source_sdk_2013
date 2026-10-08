//! What the server sends clients through their channels: the engine's own
//! message objects, and the encoded messages they become.
//!
//! The engine sends a client everything through the client's channel, in
//! three ways. `INetChannel::SendNetMsg` takes one of the engine's message
//! objects, an [`OutgoingMessage`], which encodes itself into one of the
//! channel's streams. `SendData` appends messages already encoded, such as a
//! client's sign-on data, or what
//! [`NetChannel::send_encoded`](super::NetChannel::send_encoded) sends.
//! `SendDatagram` sends a packet at once, with encoded messages for that
//! packet alone, such as a snapshot that updates the client's last.
//!
//! [`walk`] finds the messages in encoded bits as the client parses them: the
//! type and size of each, and the fields of the kinds that tell what a client
//! is sent: user messages, game events, entity messages, temporary entities,
//! sounds, entity updates, string table changes, console text and ticks.
//!
//! # Encodings
//!
//! Each message is its type in [`MESSAGE_TYPE_BITS`] bits, then its fields,
//! which the walk reads as TF2's engine writes them for its protocol 24.
//!
//! For the kinds [`messages`](super::messages) encodes, which clients accept
//! as it encodes them, the walk reads exactly what it writes: `net_SetConVar`,
//! `net_StringCmd`, `svc_BSPDecal`, `svc_EntityMessage`, `svc_FixAngle`,
//! `svc_GameEvent`, `svc_GetCvarValue`, `svc_Prefetch`, `svc_Print`,
//! `svc_SetPause`, `svc_SetView`, `svc_UserMessage` and `svc_VoiceData`.
//! Several of their widths are also constants of the public headers: the 11
//! bits of an entity's index (`MAX_EDICT_BITS` in `public/const.h`), the 9 of
//! a server class's ID (`MAX_SERVER_CLASS_BITS`, also there), the 13 of a
//! model's index (`SP_MODEL_INDEX_BITS`, also there), the 9 of a decal's
//! texture (`MAX_DECAL_INDEX_BITS` in `common/qlimits.h`) and the 14 of a
//! sound's index (`MAX_SOUND_INDEX_BITS` in `public/soundflags.h`). The 11
//! bits of a payload's length, which hold the 255 bytes of
//! `MAX_USER_MSG_DATA`, have no constant there.
//!
//! The layouts of the other kinds are inferred: no public header describes
//! them, and they are the layouts TF2's demos, which record what clients
//! receive, are read with. Where a header's constant applies, the width
//! agrees with it.
//!
//! - `net_Tick`: the tick in 32 bits, then the frame time and its deviation in
//!   16 bits each.
//! - `net_SignonState`: the state in 8 bits, then the spawn count in 32.
//! - `net_File`: the transfer's ID in 32 bits, the file's name, then a bit for
//!   a request.
//! - `svc_ServerInfo`: the protocol in 16 bits, the server count in 32, a bit
//!   each for SourceTV and a dedicated server, the client library's CRC in 32
//!   bits, the number of classes in 16, the map's MD5 in 16 bytes, the
//!   player's slot and the most players in 8 bits each, the tick interval as a
//!   32-bit float, the platform as a byte, the game's directory and the map's,
//!   sky's and host's names, then a bit for a replay server.
//! - `svc_ClassInfo`: the number of classes in 16 bits, then a bit telling the
//!   client to create them itself. When it is clear, the classes follow, in a
//!   layout not inferred here, and the walk stops.
//! - `svc_CreateStringTable`: the table's name, its most entries in 16 bits,
//!   its number of entries in one bit more than the base-2 logarithm of the
//!   most, the data's length in bits as a variable-length integer, a bit that
//!   flags a fixed size of user data in 12 and 4 more bits, a bit for
//!   compression, then the data.
//! - `svc_UpdateStringTable`: the table's ID in 5 bits, which hold the 32
//!   tables of `MAX_TABLES` in `public/networkstringtabledefs.h`, a bit that
//!   flags the number of entries changed in 16 bits, which is one otherwise,
//!   the data's length in bits in 20, then the data.
//! - `svc_VoiceInit`: the codec's name, the quality in 8 bits, and when that
//!   is 255, the sample rate in 16.
//! - `svc_Sounds`: a bit for a reliable sound, then the data's length in bits
//!   in 8 bits for one, or the number of sounds in 8 bits and the length in 16
//!   for others, then the data.
//! - `svc_PacketEntities`: the most entries in 11 bits, a bit that flags the
//!   tick of the snapshot the entities are updated from in 32 bits, the
//!   baseline in 1 bit, the number of entries in 11 bits, the data's length in
//!   bits in 20, a bit for updating the baseline, then the data.
//! - `svc_TempEntities`: the number of temporary entities in 8 bits, where 0
//!   stands for a single reliable one, the data's length in bits as a
//!   variable-length integer, then the data. `common/proto_version.h` notes
//!   that protocol 24 did away with the fixed-width lengths of
//!   `NET_MAX_PAYLOAD_BITS`.
//! - `svc_Menu`: the menu's kind in 16 bits, the data's length in bytes in 16,
//!   then the data.
//! - `svc_GameEventList`: the number of events in 9 bits (`MAX_EVENT_BITS` in
//!   `public/igameevents.h`), the data's length in bits in 20, then the data.
//! - `svc_CmdKeyValues`: the data's length in bytes in 32 bits, then the data.
//!
//! The walk never reads past the bits it is given, and does not guess: at a
//! type with no layout here, such as `net_Disconnect`, `svc_SendTable`,
//! `svc_CrosshairAngle` or a number no message has, or at a field that does
//! not fit in the bits left, it stops, and reports the rest of the bits as one
//! message of that type, with [`Details::Unparsed`].

#[cfg(test)]
#[path = "../tests/net/outgoing.rs"]
mod tests;

use super::messages::{
	ANGLE_BITS, DECAL_INDEX_BITS, MODEL_INDEX_BITS, PAYLOAD_LENGTH_BITS, SERVER_CLASS_BITS,
	SOUND_INDEX_BITS, VOICE_LENGTH_BITS,
};

use super::{MESSAGE_TYPE_BITS, MessageId};
use crate::NotThreadSafe;
use crate::bitbuf::{BitReader, BitWriter, Overflow};
use sdk_raw::bitbuf::BfWrite;
use sdk_raw::edicts::MAX_EDICT_BITS;
use sdk_raw::interfaces::game_event::MAX_EVENT_BITS;
use sdk_raw::util::cstr::copy_cstr;
use sdk_raw::vcall;
use std::ffi::{CString, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// Bits in the length, in bits, of a `svc_PacketEntities`'s data
/// (`DELTASIZE_BITS`).
const DELTA_SIZE_BITS: u32 = 20;

/// Bits in the length, in bits, of a `svc_GameEventList`'s data.
const EVENT_LIST_LENGTH_BITS: u32 = 20;

/// The words [`OutgoingMessage::encode`] lets a message take, 512 KiB.
const MAX_ENCODED_WORDS: usize = 1 << 17;

/// The words [`OutgoingMessage::encode`] first lets a message take, 16 KiB,
/// which all but the largest messages fit in.
const SMALL_ENCODED_WORDS: usize = 1 << 12;

/// Bits in the length, in bits, of a reliable sound's data.
const SOUND_LENGTH_BITS: u32 = 8;

/// Bits in the length, in bits, of unreliable sounds' data.
const SOUNDS_LENGTH_BITS: u32 = 16;

/// Bits in the length, in bits, of a `svc_UpdateStringTable`'s data.
const STRING_TABLE_LENGTH_BITS: u32 = 20;

/// Bits in a string table's ID, which hold the 32 tables of `MAX_TABLES`.
const TABLE_ID_BITS: u32 = 5;

/// What [`walk`] read of a message's fields.
#[derive(Debug, Clone, PartialEq)]
pub enum Details {
	/// A message whose fields the walk passed over, keeping none.
	Other,

	/// A message for one entity's client-side class (`svc_EntityMessage`).
	EntityMessage {
		/// The entity's index.
		entity: u16,

		/// The ID of the entity's server class.
		class_id: u16,

		/// The payload.
		data: BitWriter,
	},

	/// A game event (`svc_GameEvent`).
	GameEvent {
		/// The event's ID, then its fields, which
		/// [`GameEventManager::unserialize_event`] decodes.
		///
		/// [`GameEventManager::unserialize_event`]: crate::interfaces::GameEventManager::unserialize_event
		data: BitWriter,
	},

	/// Entities' states, for a snapshot (`svc_PacketEntities`).
	PacketEntities {
		/// The tick of the snapshot the client has, which these update, or
		/// `None` for a full update.
		delta_from: Option<i32>,

		/// The number of entities the data holds an entry for.
		updated: u16,
	},

	/// Console text (`svc_Print`).
	Print {
		/// The text.
		text: CString,
	},

	/// Sounds to play (`svc_Sounds`).
	Sounds {
		/// The number of sounds, one for a reliable sound.
		count: u8,

		/// Whether the message carries a single sound sent reliably, rather
		/// than the sounds of a snapshot.
		reliable: bool,
	},

	/// Temporary entities, such as effects (`svc_TempEntities`).
	TempEntities {
		/// The number of temporary entities, one for a reliable one.
		count: u8,

		/// Whether the message carries a single temporary entity sent
		/// reliably, rather than those of a snapshot.
		reliable: bool,
	},

	/// The server's tick (`net_Tick`), which starts each snapshot.
	Tick {
		/// The tick.
		tick: i32,
	},

	/// Changes to a string table's entries (`svc_UpdateStringTable`).
	UpdateStringTable {
		/// The table's ID, below 32.
		table: u8,

		/// The number of entries changed.
		changed: u16,
	},

	/// A user message (`svc_UserMessage`).
	UserMessage {
		/// The index the game registered the user message at, which
		/// [`ServerGameDll::user_message`] names.
		///
		/// [`ServerGameDll::user_message`]: crate::interfaces::ServerGameDll::user_message
		message_type: u8,

		/// The payload.
		data: BitWriter,
	},

	/// The walk stopped at this message, whose type has no layout here, or
	/// whose fields do not fit in the bits left. Its size is the rest of the
	/// bits walked.
	Unparsed,
}

/// A message [`walk`] found in encoded bits.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedMessage {
	/// The message's type.
	pub id: MessageId,

	/// The message's size in bits, its type included.
	pub bits: usize,

	/// What the walk read of the message's fields.
	pub details: Details,
}

/// One of the engine's message objects, which the engine is sending a client
/// through the client's channel.
#[doc(alias("INetMessage"))]
#[derive(Debug, Clone, Copy)]
pub struct OutgoingMessage<'s> {
	raw: NonNull<sys::INetMessage>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> OutgoingMessage<'s> {
	/// Wraps a message the engine is sending, such as the one a hook on
	/// `INetChannel::SendNetMsg` was passed.
	///
	/// # Safety
	///
	/// `raw` must be a live message of the engine's, which stays alive and
	/// unchanged for `'s`, and the handle must be made on the server's main
	/// thread.
	pub const unsafe fn from_raw(raw: NonNull<sys::INetMessage>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	const fn as_const(self) -> *const sys::INetMessage {
		self.raw.as_ptr().cast_const()
	}

	/// The engine's message, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::INetMessage {
		self.raw.as_ptr()
	}

	/// The engine's own description of the message and its fields, or `None`
	/// if the engine returns none.
	#[doc(alias("ToString"))]
	pub fn describe(self) -> Option<CString> {
		// SAFETY: As for `id`. The engine formats into a buffer it reuses, so
		// the text is copied at once.
		unsafe { copy_cstr(vcall!(self.as_const() => INetMessage_ToString())) }
	}

	/// Encodes the message as the channel will, with the message's own
	/// `WriteToBuffer`: its type, then its fields, which [`walk`] reads.
	///
	/// Returns `None` if the message reports that it could not be written, or
	/// takes more than 512 KiB. A message larger than 16 KiB is written again
	/// into a larger buffer. The engine's messages write the same bits each
	/// time, so encoding one leaves what the engine sends unchanged.
	#[doc(alias("WriteToBuffer"))]
	pub fn encode(self) -> Option<BitWriter> {
		let mut storage = [0; SMALL_ENCODED_WORDS];

		match self.encode_into(&mut storage) {
			Err(Overflow) => self
				.encode_into(&mut vec![0; MAX_ENCODED_WORDS])
				.ok()
				.flatten(),

			Ok(bits) => bits,
		}
	}

	/// Encodes the message into `storage`, returning [`Overflow`] if it does
	/// not fit, and `None` if the message reports another failure.
	fn encode_into(self, storage: &mut [u32]) -> Result<Option<BitWriter>, Overflow> {
		let mut buffer = BfWrite::empty(storage);

		// SAFETY: As for `id`. The message writes through the buffer, within
		// the storage it describes, which outlives the call, and marks it
		// overflowed rather than write past it.
		let written =
			unsafe { vcall!(self.as_ptr() => INetMessage_WriteToBuffer(buffer.as_raw())) };

		if buffer.overflow != 0 {
			return Err(Overflow);
		}

		// SAFETY: The buffer describes `storage`, which the call has finished
		// writing.
		Ok(written
			.then(|| unsafe { BfWrite::read_back(NonNull::from(&mut buffer)) })
			.flatten()
			.map(BitWriter::from))
	}

	/// The group the engine counts the message's traffic in, one of
	/// `INetChannelInfo`'s, such as
	/// [`INetChannelInfo_USERMESSAGES`](sys::INetChannelInfo_USERMESSAGES).
	#[doc(alias("GetGroup"))]
	pub fn group(self) -> c_int {
		// SAFETY: As for `id`.
		unsafe { vcall!(self.as_const() => INetMessage_GetGroup()) }
	}

	/// The message's type, or `None` if the engine reports a number that does
	/// not fit in [`MESSAGE_TYPE_BITS`] bits.
	#[doc(alias("GetType"))]
	pub fn id(self) -> Option<MessageId> {
		// SAFETY: The message is live for `'s`, on the main thread.
		let id = unsafe { vcall!(self.as_const() => INetMessage_GetType()) };

		u8::try_from(id).ok().and_then(MessageId::new)
	}

	/// Whether the message goes in the channel's reliable stream, unless the
	/// sender chooses it.
	#[doc(alias("IsReliable"))]
	pub fn is_reliable(self) -> bool {
		// SAFETY: As for `id`.
		unsafe { vcall!(self.as_const() => INetMessage_IsReliable()) }
	}

	/// The engine's name for the message, such as `svc_UserMessage`, or `None`
	/// if the engine returns none.
	#[doc(alias("GetName"))]
	pub fn name(self) -> Option<CString> {
		// SAFETY: As for `id`, and the name is copied at once.
		unsafe { copy_cstr(vcall!(self.as_const() => INetMessage_GetName())) }
	}
}

/// The messages in encoded bits, in order, which [`walk`] returns.
#[derive(Debug, Clone)]
pub struct Walk<'a> {
	reader: BitReader<'a>,
}

impl Walk<'_> {
	/// The bits the walk has not reached: none after an unparsed message, and
	/// fewer than [`MESSAGE_TYPE_BITS`] once it ends otherwise.
	pub const fn remaining(&self) -> usize {
		self.reader.remaining()
	}
}

impl Iterator for Walk<'_> {
	type Item = EncodedMessage;

	fn next(&mut self) -> Option<EncodedMessage> {
		if self.reader.remaining() < MESSAGE_TYPE_BITS as usize {
			return None;
		}

		let start = self.reader.position();
		let id = MessageId::new(self.reader.read_ubits(MESSAGE_TYPE_BITS).ok()? as u8)?;

		// The fields are read from a copy, which is kept only if they all fit.
		let mut fields = self.reader.clone();

		let details = match read_fields(id, &mut fields) {
			Ok(Some(details)) => {
				self.reader = fields;
				details
			}

			Ok(None) | Err(Overflow) => {
				self.reader.skip(self.reader.remaining()).ok()?;
				Details::Unparsed
			}
		};

		Some(EncodedMessage {
			id,
			bits: self.reader.position() - start,
			details,
		})
	}
}

/// Reads the fields of a message of type `id`, which follow its type, or
/// returns `None` for a type with no layout here.
fn read_fields(id: MessageId, reader: &mut BitReader<'_>) -> Result<Option<Details>, Overflow> {
	let details = match id {
		MessageId::NOP => Details::Other,

		MessageId::FILE => {
			reader.skip(32)?;
			skip_string(reader)?;
			reader.skip(1)?;
			Details::Other
		}

		MessageId::TICK => {
			let tick = reader.read_i32()?;

			// The frame time and its deviation.
			reader.skip(16 + 16)?;
			Details::Tick { tick }
		}

		MessageId::STRING_CMD => {
			skip_string(reader)?;
			Details::Other
		}

		MessageId::SET_CONVAR => {
			for _ in 0..reader.read_u8()? {
				skip_string(reader)?;
				skip_string(reader)?;
			}

			Details::Other
		}

		MessageId::SIGNON_STATE => {
			reader.skip(8 + 32)?;
			Details::Other
		}

		MessageId::PRINT => Details::Print {
			text: reader.read_cstring()?,
		},

		MessageId::SERVER_INFO => {
			// From the protocol to the number of classes, the map's MD5, then from
			// the player's slot to the platform.
			reader.skip(16 + 32 + 1 + 1 + 32 + 16)?;
			reader.skip(16 * 8)?;
			reader.skip(8 + 8 + 32 + 8)?;

			// The game's directory, and the map's, sky's and host's names.
			for _ in 0..4 {
				skip_string(reader)?;
			}

			// Whether the server is a replay server.
			reader.skip(1)?;
			Details::Other
		}

		MessageId::CLASS_INFO => {
			reader.skip(16)?;

			// Unless the client creates the classes itself, they follow.
			if !reader.read_bit()? {
				return Ok(None);
			}

			Details::Other
		}

		MessageId::SET_PAUSE => {
			reader.skip(1)?;
			Details::Other
		}

		MessageId::CREATE_STRING_TABLE => {
			skip_string(reader)?;

			let most = reader.read_u16()?;

			// The number of entries.
			reader.skip(most.checked_ilog2().unwrap_or(0) as usize + 1)?;

			let length = reader.read_var_u32()?;

			// A fixed size of user data, in bytes and in bits.
			if reader.read_bit()? {
				reader.skip(12 + 4)?;
			}

			// Whether the data is compressed.
			reader.skip(1)?;
			reader.skip(length as usize)?;
			Details::Other
		}

		MessageId::UPDATE_STRING_TABLE => {
			let table = reader.read_ubits(TABLE_ID_BITS)? as u8;

			let changed = match reader.read_bit()? {
				true => reader.read_u16()?,
				false => 1,
			};

			let length = reader.read_ubits(STRING_TABLE_LENGTH_BITS)?;

			reader.skip(length as usize)?;
			Details::UpdateStringTable { table, changed }
		}

		MessageId::VOICE_INIT => {
			skip_string(reader)?;

			// A quality of 255 flags the sample rate.
			if reader.read_u8()? == 255 {
				reader.skip(16)?;
			}

			Details::Other
		}

		MessageId::VOICE_DATA => {
			// The speaker, and the proximity flag.
			reader.skip(8 + 8)?;

			let length = reader.read_ubits(VOICE_LENGTH_BITS)?;

			reader.skip(length as usize)?;
			Details::Other
		}

		MessageId::SOUNDS => {
			let reliable = reader.read_bit()?;

			let (count, length) = match reliable {
				true => (1, reader.read_ubits(SOUND_LENGTH_BITS)?),
				false => (reader.read_u8()?, reader.read_ubits(SOUNDS_LENGTH_BITS)?),
			};

			reader.skip(length as usize)?;
			Details::Sounds { count, reliable }
		}

		MessageId::SET_VIEW => {
			reader.skip(MAX_EDICT_BITS as usize)?;
			Details::Other
		}

		MessageId::FIX_ANGLE => {
			// Whether the angles are relative, then the angles.
			reader.skip(1 + 3 * ANGLE_BITS as usize)?;
			Details::Other
		}

		MessageId::BSP_DECAL => {
			reader.read_bit_vec3_coord()?;
			reader.skip(DECAL_INDEX_BITS as usize)?;

			// The entity and its model, if the decal is not on the world.
			if reader.read_bit()? {
				reader.skip((MAX_EDICT_BITS + MODEL_INDEX_BITS) as usize)?;
			}

			// Whether the decal has a low priority.
			reader.skip(1)?;
			Details::Other
		}

		MessageId::USER_MESSAGE => {
			let message_type = reader.read_u8()?;

			Details::UserMessage {
				message_type,
				data: read_payload(reader)?,
			}
		}

		MessageId::ENTITY_MESSAGE => {
			let entity = reader.read_ubits(MAX_EDICT_BITS)? as u16;
			let class_id = reader.read_ubits(SERVER_CLASS_BITS)? as u16;

			Details::EntityMessage {
				entity,
				class_id,
				data: read_payload(reader)?,
			}
		}

		MessageId::GAME_EVENT => Details::GameEvent {
			data: read_payload(reader)?,
		},

		MessageId::PACKET_ENTITIES => {
			// The most entries.
			reader.skip(MAX_EDICT_BITS as usize)?;

			let delta_from = match reader.read_bit()? {
				true => Some(reader.read_i32()?),
				false => None,
			};

			// The baseline.
			reader.skip(1)?;

			let updated = reader.read_ubits(MAX_EDICT_BITS)? as u16;
			let length = reader.read_ubits(DELTA_SIZE_BITS)?;

			// Whether the client updates its baseline.
			reader.skip(1)?;
			reader.skip(length as usize)?;
			Details::PacketEntities {
				delta_from,
				updated,
			}
		}

		MessageId::TEMP_ENTITIES => {
			let count = reader.read_u8()?;
			let length = reader.read_var_u32()?;

			reader.skip(length as usize)?;

			Details::TempEntities {
				count: count.max(1),
				reliable: count == 0,
			}
		}

		MessageId::PREFETCH => {
			reader.skip(SOUND_INDEX_BITS as usize)?;
			Details::Other
		}

		MessageId::MENU => {
			// The menu's kind.
			reader.skip(16)?;

			let length = reader.read_u16()?;

			reader.skip(usize::from(length) * 8)?;
			Details::Other
		}

		MessageId::GAME_EVENT_LIST => {
			// The number of events.
			reader.skip(MAX_EVENT_BITS as usize)?;

			let length = reader.read_ubits(EVENT_LIST_LENGTH_BITS)?;

			reader.skip(length as usize)?;
			Details::Other
		}

		MessageId::GET_CVAR_VALUE => {
			// The cookie.
			reader.skip(32)?;
			skip_string(reader)?;
			Details::Other
		}

		MessageId::CMD_KEY_VALUES => {
			let length = reader.read_u32()? as usize;

			reader.skip(length.checked_mul(8).ok_or(Overflow)?)?;
			Details::Other
		}

		_ => return Ok(None),
	};

	Ok(Some(details))
}

/// Reads a payload's length in bits, then the payload, as
/// [`messages`](super::messages) writes them.
fn read_payload(reader: &mut BitReader<'_>) -> Result<BitWriter, Overflow> {
	let length = reader.read_ubits(PAYLOAD_LENGTH_BITS)?;

	reader.read_bits(length as usize)
}

/// Passes over a string and its terminator.
fn skip_string(reader: &mut BitReader<'_>) -> Result<(), Overflow> {
	while reader.read_u8()? != 0 {}

	Ok(())
}

/// Walks the encoded messages `reader` holds from its position, as the client
/// parses them: each one's type and size, and the fields of the kinds that
/// tell what a client is sent; see the
/// [module documentation](self#encodings).
///
/// The walk ends where fewer bits than a type's remain, as the client's
/// parsing does, since packets are padded to whole bytes, or after a message
/// it cannot read, which [`Details::Unparsed`] marks.
pub fn walk(reader: BitReader<'_>) -> Walk<'_> {
	Walk { reader }
}
