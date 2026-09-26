//! Clients' net channels, and the messages the server sends through them.
//!
//! Each connected client has an `INetChannel`, which carries a reliable and
//! an unreliable stream of net messages. A message is a 6-bit type followed by
//! its fields, encoded with [`bitbuf`](crate::bitbuf). The engine sends the
//! streams at the end of each frame, reliable data first.
//!
//! [`ValveEngine::net_channel`](crate::interfaces::ValveEngine::net_channel)
//! returns a client's channel. Typed messages are in [`messages`]; any other
//! message can be sent as a [`messages::Raw`].

pub mod incoming;
pub mod messages;

use crate::bitbuf::{BitWriter, RawBfWrite};
use crate::ffi::{NotThreadSafe, copy_cstr, vcall};
use std::error::Error;
use std::ffi::{CString, c_int};
use std::fmt::{self, Display, Formatter};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// Bits in each message's type (`NETMSG_TYPE_BITS`).
pub const MESSAGE_TYPE_BITS: u32 = 6;

/// A net message's type, the first field of every message.
///
/// The constants are the types of TF2's protocol 24. `Disconnect`,
/// `SendTable`, `CrosshairAngle` and the unused 16 and 22 follow the engine's
/// numbering; the rest match TF2's demos.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MessageId(u8);

impl MessageId {
	pub const NOP: Self = Self(0);
	pub const DISCONNECT: Self = Self(1);
	pub const FILE: Self = Self(2);
	pub const TICK: Self = Self(3);
	pub const STRING_CMD: Self = Self(4);
	pub const SET_CONVAR: Self = Self(5);
	pub const SIGNON_STATE: Self = Self(6);
	pub const PRINT: Self = Self(7);
	pub const SERVER_INFO: Self = Self(8);
	pub const SEND_TABLE: Self = Self(9);
	pub const CLASS_INFO: Self = Self(10);
	pub const SET_PAUSE: Self = Self(11);
	pub const CREATE_STRING_TABLE: Self = Self(12);
	pub const UPDATE_STRING_TABLE: Self = Self(13);
	pub const VOICE_INIT: Self = Self(14);
	pub const VOICE_DATA: Self = Self(15);
	pub const SOUNDS: Self = Self(17);
	pub const SET_VIEW: Self = Self(18);
	pub const FIX_ANGLE: Self = Self(19);
	pub const CROSSHAIR_ANGLE: Self = Self(20);
	pub const BSP_DECAL: Self = Self(21);
	pub const USER_MESSAGE: Self = Self(23);
	pub const ENTITY_MESSAGE: Self = Self(24);
	pub const GAME_EVENT: Self = Self(25);
	pub const PACKET_ENTITIES: Self = Self(26);
	pub const TEMP_ENTITIES: Self = Self(27);
	pub const PREFETCH: Self = Self(28);
	pub const MENU: Self = Self(29);
	pub const GAME_EVENT_LIST: Self = Self(30);
	pub const GET_CVAR_VALUE: Self = Self(31);
	pub const CMD_KEY_VALUES: Self = Self(32);

	/// A type, which must fit in [`MESSAGE_TYPE_BITS`] bits.
	pub const fn new(id: u8) -> Option<Self> {
		match id < 1 << MESSAGE_TYPE_BITS {
			true => Some(Self(id)),
			false => None,
		}
	}

	pub const fn get(self) -> u8 {
		self.0
	}

	/// The engine's name for the type, such as `svc_Print`.
	pub const fn name(self) -> Option<&'static str> {
		Some(match self.0 {
			0 => "net_NOP",
			1 => "net_Disconnect",
			2 => "net_File",
			3 => "net_Tick",
			4 => "net_StringCmd",
			5 => "net_SetConVar",
			6 => "net_SignonState",
			7 => "svc_Print",
			8 => "svc_ServerInfo",
			9 => "svc_SendTable",
			10 => "svc_ClassInfo",
			11 => "svc_SetPause",
			12 => "svc_CreateStringTable",
			13 => "svc_UpdateStringTable",
			14 => "svc_VoiceInit",
			15 => "svc_VoiceData",
			17 => "svc_Sounds",
			18 => "svc_SetView",
			19 => "svc_FixAngle",
			20 => "svc_CrosshairAngle",
			21 => "svc_BSPDecal",
			23 => "svc_UserMessage",
			24 => "svc_EntityMessage",
			25 => "svc_GameEvent",
			26 => "svc_PacketEntities",
			27 => "svc_TempEntities",
			28 => "svc_Prefetch",
			29 => "svc_Menu",
			30 => "svc_GameEventList",
			31 => "svc_GetCvarValue",
			32 => "svc_CmdKeyValues",
			_ => return None,
		})
	}
}

impl Display for MessageId {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self.name() {
			Some(name) => f.write_str(name),
			None => write!(f, "message {}", self.0),
		}
	}
}

/// Which stream of a channel a message goes in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Reliability {
	/// Sent until acknowledged, in order with the channel's other reliable
	/// messages.
	#[default]
	Reliable,

	/// Sent once, with the next packet, and dropped if it does not fit.
	Unreliable,
}

/// A direction of a channel's traffic, for its statistics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Flow {
	/// From the server to the client (`FLOW_OUTGOING`).
	Outgoing,

	/// From the client to the server (`FLOW_INCOMING`).
	Incoming,
}

impl Flow {
	const fn raw(self) -> c_int {
		match self {
			Self::Outgoing => 0,
			Self::Incoming => 1,
		}
	}
}

/// A channel's packet sequence numbers, as `GetSequenceData` reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SequenceData {
	/// The sequence number the next outgoing packet will have.
	pub outgoing: c_int,

	/// The sequence number of the last packet received.
	pub incoming: c_int,

	/// The last outgoing sequence number the client acknowledged.
	pub outgoing_acknowledged: c_int,
}

/// A message that can be sent through a [`NetChannel`].
///
/// The engine's client checks each message's fields as it parses them, and
/// disconnects from the server if one is malformed, so implementations must
/// write exactly the fields their type has.
pub trait NetMessage {
	/// The message's type.
	fn id(&self) -> MessageId;

	/// Writes the message's fields, which follow its type.
	fn write_body(&self, out: &mut BitWriter) -> Result<(), EncodeError>;

	/// The stream the message goes in unless the sender chooses.
	fn reliability(&self) -> Reliability {
		Reliability::Reliable
	}

	/// The whole message: its type, then its fields.
	fn encode(&self) -> Result<BitWriter, EncodeError> {
		let mut out = BitWriter::new();

		out.write_ubits(self.id().get().into(), MESSAGE_TYPE_BITS);
		self.write_body(&mut out)?;

		Ok(out)
	}
}

/// Why a message could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
	#[error("{field} is {len} bytes long, but at most {max} fit")]
	TooLong {
		field: &'static str,
		len: usize,
		max: usize,
	},

	#[error("{field} is {value}, which exceeds its maximum of {max}")]
	OutOfRange {
		field: &'static str,
		value: u64,
		max: u64,
	},
}

impl EncodeError {
	/// Checks a length, such as a string's without its terminator.
	pub(crate) fn check_len(field: &'static str, len: usize, max: usize) -> Result<(), Self> {
		match len <= max {
			true => Ok(()),
			false => Err(Self::TooLong { field, len, max }),
		}
	}

	/// Checks that a value fits in `bits` bits.
	pub(crate) fn check_bits(field: &'static str, value: u64, bits: u32) -> Result<(), Self> {
		let max = (1u64 << bits) - 1;

		match value <= max {
			true => Ok(()),
			false => Err(Self::OutOfRange { field, value, max }),
		}
	}
}

/// Why a message could not be sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
	/// The message could not be encoded.
	Encode(EncodeError),

	/// The channel's reliable stream has no room for the message. The engine
	/// frees room as the client acknowledges what was sent.
	TooLarge { bits: usize },
}

impl Display for SendError {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		match self {
			Self::Encode(error) => Display::fmt(error, f),
			Self::TooLarge { bits } => {
				write!(f, "the channel has no room for a message of {bits} bits")
			}
		}
	}
}

impl Error for SendError {
	fn source(&self) -> Option<&(dyn Error + 'static)> {
		match self {
			Self::Encode(error) => Some(error),
			Self::TooLarge { .. } => None,
		}
	}
}

impl From<EncodeError> for SendError {
	fn from(error: EncodeError) -> Self {
		Self::Encode(error)
	}
}

/// A connected client's net channel (`INetChannel`).
///
/// The engine frees a channel when its client disconnects, which none of this
/// crate's safe functions causes, so a channel stays valid for the scope `'s`.
/// Fake clients, such as bots and SourceTV, have none.
#[doc(alias = "INetChannel")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetChannel<'s> {
	raw: NonNull<sys::INetChannel>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> NetChannel<'s> {
	/// # Safety
	///
	/// `raw` must be a client's channel, which stays allocated for `'s`.
	pub(crate) const unsafe fn from_raw(raw: NonNull<sys::INetChannel>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	pub const fn as_ptr(self) -> *mut sys::INetChannel {
		self.raw.as_ptr()
	}

	const fn as_const(self) -> *const sys::INetChannel {
		self.raw.as_ptr().cast_const()
	}

	/// The channel's name, which is the client's name.
	#[doc(alias = "GetName")]
	pub fn name(self) -> Option<CString> {
		// SAFETY: The channel is live for `'s`, and the string is copied at once.
		unsafe { copy_cstr(vcall!(self.as_const() => INetChannel_GetName())) }
	}

	/// The client's address, such as `203.0.113.7:27005`.
	#[doc(alias = "GetAddress")]
	pub fn address(self) -> Option<CString> {
		// SAFETY: As for `name`.
		unsafe { copy_cstr(vcall!(self.as_const() => INetChannel_GetAddress())) }
	}

	/// Seconds since the client connected.
	#[doc(alias = "GetTimeConnected")]
	pub fn time_connected(self) -> f32 {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_GetTimeConnected()) }
	}

	/// The rate, in bytes per second, the channel sends at most.
	#[doc(alias = "GetDataRate")]
	pub fn data_rate(self) -> c_int {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_GetDataRate()) }
	}

	/// Whether the client runs in the server's own process, as a listen
	/// server's host does.
	#[doc(alias = "IsLoopback")]
	pub fn is_loopback(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_IsLoopback()) }
	}

	/// Whether nothing has arrived from the client for a while.
	#[doc(alias = "IsTimingOut")]
	pub fn is_timing_out(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_IsTimingOut()) }
	}

	/// The latest round-trip time, in seconds.
	#[doc(alias = "GetLatency")]
	pub fn latency(self, flow: Flow) -> f32 {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_GetLatency(flow.raw())) }
	}

	/// The average round-trip time, in seconds.
	#[doc(alias = "GetAvgLatency")]
	pub fn average_latency(self, flow: Flow) -> f32 {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_GetAvgLatency(flow.raw())) }
	}

	/// The average share of packets lost, from 0 to 1.
	#[doc(alias = "GetAvgLoss")]
	pub fn average_loss(self, flow: Flow) -> f32 {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_GetAvgLoss(flow.raw())) }
	}

	/// The average share of packets held back by the rate, from 0 to 1.
	#[doc(alias = "GetAvgChoke")]
	pub fn average_choke(self, flow: Flow) -> f32 {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_GetAvgChoke(flow.raw())) }
	}

	/// The average traffic, in bytes per second.
	#[doc(alias = "GetAvgData")]
	pub fn average_data(self, flow: Flow) -> f32 {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_GetAvgData(flow.raw())) }
	}

	/// The average packets per second.
	#[doc(alias = "GetAvgPackets")]
	pub fn average_packets(self, flow: Flow) -> f32 {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_GetAvgPackets(flow.raw())) }
	}

	/// The bytes sent or received since the client connected.
	#[doc(alias = "GetTotalData")]
	pub fn total_data(self, flow: Flow) -> c_int {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_GetTotalData(flow.raw())) }
	}

	/// Seconds since a packet last arrived from the client.
	#[doc(alias = "GetTimeSinceLastReceived")]
	pub fn time_since_last_received(self) -> f32 {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_GetTimeSinceLastReceived()) }
	}

	/// Seconds without a packet from the client before it times out.
	#[doc(alias = "GetTimeoutSeconds")]
	pub fn timeout_seconds(self) -> f32 {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_GetTimeoutSeconds()) }
	}

	/// The channel's packet sequence numbers.
	#[doc(alias = "GetSequenceData")]
	pub fn sequence_data(self) -> SequenceData {
		let mut data = SequenceData {
			outgoing: 0,
			incoming: 0,
			outgoing_acknowledged: 0,
		};

		// SAFETY: As for `name`, and the engine writes the three integers.
		unsafe {
			vcall!(self.as_ptr() => INetChannel_GetSequenceData(
				&raw mut data.outgoing,
				&raw mut data.incoming,
				&raw mut data.outgoing_acknowledged,
			))
		};

		data
	}

	/// Whether reliable data is waiting to be sent, or sent but not yet
	/// acknowledged by the client.
	///
	/// Once this turns false after a reliable message was sent, the client has
	/// received it.
	#[doc(alias = "HasPendingReliableData")]
	pub fn has_pending_reliable_data(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => INetChannel_HasPendingReliableData()) }
	}

	/// Whether the reliable stream overflowed, which disconnects the client.
	#[doc(alias = "IsOverflowed")]
	pub fn is_overflowed(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_const() => INetChannel_IsOverflowed()) }
	}

	/// The bits waiting in a stream for the next packet.
	#[doc(alias = "GetNumBitsWritten")]
	pub fn bits_waiting(self, reliability: Reliability) -> c_int {
		let reliable = reliability == Reliability::Reliable;

		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => INetChannel_GetNumBitsWritten(reliable)) }
	}

	/// The protocol version the client connected with.
	#[doc(alias = "GetProtocolVersion")]
	pub fn protocol_version(self) -> c_int {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => INetChannel_GetProtocolVersion()) }
	}

	/// Queues a message in its default stream. The engine sends it at the end
	/// of the frame, after the messages queued before it.
	pub fn send(self, message: &impl NetMessage) -> Result<(), SendError> {
		self.send_with(message, message.reliability())
	}

	/// Queues a message in a chosen stream.
	pub fn send_with(
		self,
		message: &impl NetMessage,
		reliability: Reliability,
	) -> Result<(), SendError> {
		self.send_encoded(&message.encode()?, reliability)
	}

	/// Queues already encoded messages, each a type followed by its fields.
	///
	/// A malformed message makes the client disconnect.
	#[doc(alias = "SendData")]
	pub fn send_encoded(self, bits: &BitWriter, reliability: Reliability) -> Result<(), SendError> {
		let mut buffer = RawBfWrite::written(bits);
		let reliable = reliability == Reliability::Reliable;

		// SAFETY: As for `name`. The engine only reads the buffer, which
		// describes the writer's storage and outlives the call.
		let sent = unsafe {
			vcall!(self.as_ptr() => INetChannel_SendData(
				(&raw mut buffer).cast::<sys::bf_write>(),
				reliable,
			))
		};

		match sent {
			true => Ok(()),
			false => Err(SendError::TooLarge { bits: bits.len() }),
		}
	}
}

#[cfg(test)]
pub(crate) mod test_support {
	use super::*;
	use crate::ffi::test_support::{mock_vtable, unexpected_call};
	use std::cell::RefCell;

	thread_local! {
		static SENT: RefCell<Vec<(BitWriter, bool)>> = const { RefCell::new(Vec::new()) };
		static ACCEPTS: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
	}

	unsafe extern "C" fn record_send_data(
		_: *mut sys::INetChannel,
		buffer: *mut sys::bf_write,
		reliable: bool,
	) -> bool {
		let bits = unsafe { RawBfWrite::read_back(NonNull::new(buffer.cast()).unwrap()) }
			.expect("a readable buffer");

		SENT.with_borrow_mut(|sent| sent.push((bits, reliable)));
		ACCEPTS.get()
	}

	/// A channel whose `SendData` records what it is given.
	pub(crate) struct MockChannel {
		channel: *mut sys::INetChannel,
	}

	impl MockChannel {
		pub(crate) fn new() -> Self {
			let vtable = unsafe {
				mock_vtable::<sys::INetChannel__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).INetChannel_SendData).write(record_send_data);
					},
				)
			};

			// Leaked, so the raw pointers stay valid for the test.
			let channel = Box::into_raw(Box::new(sys::INetChannel {
				vtable_: Box::into_raw(vtable),
			}));

			SENT.set(Vec::new());
			ACCEPTS.set(true);

			Self { channel }
		}

		pub(crate) fn channel(&self) -> NetChannel<'_> {
			unsafe { NetChannel::from_raw(NonNull::new(self.channel).unwrap()) }
		}

		/// Makes `SendData` report that the stream had no room.
		pub(crate) fn refuse(&self) {
			ACCEPTS.set(false);
		}

		pub(crate) fn take_sent(&self) -> Vec<(BitWriter, bool)> {
			SENT.take()
		}
	}
}

#[cfg(test)]
mod tests {
	use super::test_support::MockChannel;
	use super::*;
	use crate::net::messages::{Print, Raw};

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
	fn types_fit_in_six_bits() {
		assert_eq!(MessageId::new(63).map(MessageId::get), Some(63));
		assert_eq!(MessageId::new(64), None);
		assert_eq!(MessageId::SET_CONVAR.to_string(), "net_SetConVar");
		assert_eq!(MessageId::new(40).unwrap().to_string(), "message 40");
	}
}
