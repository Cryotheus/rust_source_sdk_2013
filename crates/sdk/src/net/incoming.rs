//! Messages clients send the server, seen before the engine processes them.
//!
//! The engine hands each message a client sends to the `Process*` method of
//! that client's message handler (`IClientMessageHandler`), one per
//! [`IncomingKind`]. A Metamod plugin hooks those methods and passes each call
//! to [`route_incoming`], which gives an [`IncomingHandler`] the message and
//! lets it block the message.
//!
//! Every message's type, name, and description come from the engine's
//! `INetMessage`. Its fields come from the engine's own message classes,
//! which no public header declares, so [`IncomingMessage::decode`] reads them
//! only once the engine's reported sizes, and pointers the engine keeps into
//! its own messages, confirm the expected layout.

use crate::NotThreadSafe;
use crate::bitbuf::BitWriter;
use crate::interfaces::game_server::GameClient;
use crate::server::{InterfaceError, Server, ServerBinding};
use sdk_raw::util::cstr::{copy_cstr, cstring_from_buffer};
use sdk_raw::util::rtti;
use sdk_raw::vcall;
use std::ffi::{CString, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::mem::offset_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Size of a vtable slot.
const SLOT: usize = size_of::<*const ()>();

/// The client message handler methods, one per kind of message.
#[doc(alias = "IClientMessageHandler")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IncomingKind {
	/// `net_Tick`: the client's last received tick and frame times.
	#[doc(alias = "ProcessTick")]
	Tick,
	/// `net_StringCmd`: a console command for the server.
	#[doc(alias = "ProcessStringCmd")]
	StringCmd,
	/// `net_SetConVar`: changed user settings.
	#[doc(alias = "ProcessSetConVar")]
	SetConVar,
	/// `net_SignonState`: progress through connecting.
	#[doc(alias = "ProcessSignonState")]
	SignonState,
	/// `clc_ClientInfo`: the client's identity and custom files.
	#[doc(alias = "ProcessClientInfo")]
	ClientInfo,
	/// `clc_Move`: user commands.
	#[doc(alias = "ProcessMove")]
	Move,
	/// `clc_VoiceData`: encoded voice.
	#[doc(alias = "ProcessVoiceData")]
	VoiceData,
	/// `clc_BaselineAck`: acknowledgement of an entity baseline.
	#[doc(alias = "ProcessBaselineAck")]
	BaselineAck,
	/// `clc_ListenEvents`: the game events the client wants.
	#[doc(alias = "ProcessListenEvents")]
	ListenEvents,
	/// `clc_RespondCvarValue`: the answer to a console variable query.
	#[doc(alias = "ProcessRespondCvarValue")]
	RespondCvarValue,
	/// `clc_FileCRCCheck`: a file's hash, for `sv_pure`.
	#[doc(alias = "ProcessFileCRCCheck")]
	FileCrcCheck,
	/// `clc_FileMD5Check`: a file's MD5 hash.
	#[doc(alias = "ProcessFileMD5Check")]
	FileMd5Check,
	/// `clc_SaveReplay`: a request to save a replay.
	#[doc(alias = "ProcessSaveReplay")]
	SaveReplay,
	/// `clc_CmdKeyValues`: a command with key values, such as TF2's Mann vs.
	/// Machine upgrades.
	#[doc(alias = "ProcessCmdKeyValues")]
	CmdKeyValues,
}

/// The vtable slot of an `IClientMessageHandler` method, for the target's ABI.
macro_rules! handler_slot {
	($method:ident) => {
		(offset_of!(sys::IClientMessageHandler__bindgen_vtable, $method) / SLOT) as c_int
	};
}

/// Bytes of a name or value in `net_SetConVar`'s `cvar_t` (`MAX_OSPATH`).
const CONVAR_TEXT: usize = 260;

/// The largest size of `CNetMessage` [`message_base`] accepts. A larger one
/// is taken as a layout this module does not know.
const LARGEST_MESSAGE_BASE: usize = 256;

/// The most variables a `net_SetConVar` holds, which is its count's range.
const MAX_CONVARS: usize = 255;

/// `CNetMessage` holds a vtable pointer, a flag, and a channel pointer in the
/// SDK's 2013 release; the TF2 engine may add fields after them.
const SMALLEST_MESSAGE_BASE: usize = 24;

/// How far a client's message handler sits past its `IClient`, once
/// [`hook_target`] has confirmed it.
static HANDLER_OFFSET: AtomicUsize = AtomicUsize::new(0);

/// The size of `CNetMessage`, which the TF2 engine's message classes extend.
/// It is learned from the first message whose own pointers confirm it.
static MESSAGE_BASE: AtomicUsize = AtomicUsize::new(0);

/// What a plugin hooks to see clients' messages.
#[derive(Debug, Clone, Copy)]
pub struct HookTarget {
	/// One of the engine's client message handlers. Every handler of its class
	/// shares its vtable.
	pub handler: NonNull<c_void>,

	/// The vtable slot of each kind's handler method, in
	/// [`IncomingKind::ALL`]'s order.
	pub slots: [c_int; 14],
}

/// Why clients' messages cannot be hooked.
#[derive(Debug, thiserror::Error)]
pub enum HookTargetError {
	/// [`Server::valve_engine`] failed.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The engine returned no game server.
	#[error("the engine has no game server")]
	NoServer,

	/// The server has no client in its first slot. The engine creates its
	/// client objects as players first connect.
	#[error("the server has no client objects yet")]
	NotReady,

	/// The client's run-time type information does not confirm the class and
	/// base layout the hooks expect.
	#[error("the engine's clients are not laid out as expected")]
	UnexpectedLayout,
}

/// A message's fields, as [`IncomingMessage::decode`] copies them.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
	/// The client's last received tick and frame times (`NET_Tick`).
	#[doc(alias = "NET_Tick")]
	Tick {
		/// The last server tick the client received.
		tick: c_int,

		/// The client's frame time, in seconds.
		host_frame_time: f32,

		/// The standard deviation of the client's frame time, in seconds.
		host_frame_time_std_deviation: f32,
	},

	/// A console command for the server (`NET_StringCmd`).
	#[doc(alias = "NET_StringCmd")]
	StringCmd {
		/// The command and its arguments.
		command: CString,
	},

	/// Changed user settings (`NET_SetConVar`).
	#[doc(alias = "NET_SetConVar")]
	SetConVar {
		/// Names and values; at most 255.
		convars: Vec<(CString, CString)>,
	},

	/// Progress through connecting (`NET_SignonState`).
	#[doc(alias = "NET_SignonState")]
	SignonState {
		/// The sign-on state the client reached, a `SIGNONSTATE_*` value.
		state: c_int,

		/// The server's spawn count the state refers to.
		spawn_count: c_int,
	},

	/// The client's identity and custom files (`CLC_ClientInfo`).
	#[doc(alias = "CLC_ClientInfo")]
	ClientInfo {
		/// A CRC of the client's send tables.
		send_table_crc: u32,

		/// The server's spawn count the information is for.
		server_count: c_int,

		/// Whether the client is SourceTV.
		is_hltv: bool,

		/// Whether the client is a replay client.
		is_replay: bool,

		/// The client's Steam friends ID.
		friends_id: u32,

		/// The client's Steam friends name.
		friends_name: CString,

		/// CRCs of the client's custom files, such as its spray.
		custom_files: [u32; 4],
	},

	/// User commands (`CLC_Move`).
	#[doc(alias = "CLC_Move")]
	Move {
		/// Commands sent before, repeated in case their packets were lost.
		backup_commands: c_int,

		/// Commands the client has not sent before.
		new_commands: c_int,

		/// The encoded user commands.
		data: BitWriter,
	},

	/// Encoded voice (`CLC_VoiceData`).
	#[doc(alias = "CLC_VoiceData")]
	VoiceData {
		/// The encoded voice.
		data: BitWriter,
	},

	/// Acknowledgement of an entity baseline (`CLC_BaselineAck`).
	#[doc(alias = "CLC_BaselineAck")]
	BaselineAck {
		/// The tick of the baseline the client acknowledges.
		tick: c_int,

		/// Which of the client's baselines it acknowledges.
		baseline: c_int,
	},

	/// The game events the client wants (`CLC_ListenEvents`).
	#[doc(alias = "CLC_ListenEvents")]
	ListenEvents {
		/// A bit per game event ID.
		events: [u32; 16],
	},

	/// The answer to a console variable query (`CLC_RespondCvarValue`).
	#[doc(alias = "CLC_RespondCvarValue")]
	RespondCvarValue {
		/// The cookie of the query answered.
		cookie: c_int,

		/// `EQueryCvarValueStatus`: 0 when the value was found, 1 when no
		/// variable has the name, 2 when a command has it instead, and 3 when
		/// the variable does not allow queries.
		status: c_int,

		/// The variable's name.
		name: CString,

		/// The variable's value.
		value: CString,
	},

	/// A file's hash, for `sv_pure` (`CLC_FileCRCCheck`).
	#[doc(alias = "CLC_FileCRCCheck")]
	FileCrcCheck {
		/// The search path ID the file was found under, such as `GAME`.
		path_id: CString,

		/// The file's path.
		file_name: CString,

		/// The file's MD5 hash.
		md5: [u8; 16],

		/// The CRC the client reports with the hash.
		crc: u32,

		/// The kind of hash in `md5`, as the client reports it.
		hash_type: c_int,

		/// The file's size, in bytes.
		length: c_int,

		/// The number of the pack file holding the file.
		pack_file_number: c_int,

		/// The ID of the pack file holding the file.
		pack_file_id: c_int,

		/// The file fraction the client reports with the hash.
		fraction: c_int,
	},

	/// A file's MD5 hash (`CLC_FileMD5Check`).
	#[doc(alias = "CLC_FileMD5Check")]
	FileMd5Check {
		/// The search path ID the file was found under, such as `GAME`.
		path_id: CString,

		/// The file's path.
		file_name: CString,

		/// The file's MD5 hash.
		md5: [u8; 16],
	},

	/// A request to save a replay (`CLC_SaveReplay`).
	#[doc(alias = "CLC_SaveReplay")]
	SaveReplay {
		/// The byte of the replay's data to start sending from.
		start_send_byte: c_int,

		/// The name to save the replay under.
		file_name: CString,

		/// How long to keep recording after the player's death, in seconds.
		post_death_record_time: f32,
	},

	/// A command with key values (`CLC_CmdKeyValues`). The key values are not
	/// decoded.
	#[doc(alias = "CLC_CmdKeyValues")]
	CmdKeyValues,
}

/// Decides about the messages clients send.
pub trait IncomingHandler: 'static {
	/// Called for each message a client sends, before the engine processes
	/// it, during the engine's processing of the client's packet.
	fn incoming(&self, server: Server<'_>, message: IncomingMessage<'_>) -> Verdict;
}

/// A message a client sent, before the engine processed it.
#[doc(alias = "INetMessage")]
#[derive(Debug, Clone, Copy)]
pub struct IncomingMessage<'s> {
	kind: IncomingKind,
	raw: NonNull<sys::INetMessage>,
	client: GameClient<'s>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> IncomingMessage<'s> {
	const fn as_const(self) -> *const sys::INetMessage {
		self.raw.as_ptr().cast_const()
	}

	/// The engine's message, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::INetMessage {
		self.raw.as_ptr()
	}

	/// The client that sent the message.
	pub const fn client(self) -> GameClient<'s> {
		self.client
	}

	/// The message's fields, or `None` if the engine's layout is not the one
	/// expected, or the fields are inconsistent.
	pub fn decode(self) -> Option<Incoming> {
		let base = message_base(self)?;

		// SAFETY: `message_base` checked that the object is as large as the
		// kind's layout, so every field read below lies within it, and the
		// engine keeps it alive and unchanged while it is processed.
		unsafe { decode(self.kind, self.raw.cast::<u8>().as_ptr().add(base)) }
	}

	/// The engine's own description of the message and its fields, or `None`
	/// if the engine returns none.
	#[doc(alias = "ToString")]
	pub fn describe(self) -> Option<CString> {
		// SAFETY: As for `id`. The engine formats into a buffer it reuses, so
		// the text is copied at once.
		unsafe { copy_cstr(vcall!(self.as_const() => INetMessage_ToString())) }
	}

	/// The group the engine counts the message's traffic in.
	#[doc(alias = "GetGroup")]
	pub fn group(self) -> c_int {
		// SAFETY: As for `id`.
		unsafe { vcall!(self.as_const() => INetMessage_GetGroup()) }
	}

	/// The message's type, as the client numbers its messages.
	#[doc(alias = "GetType")]
	pub fn id(self) -> c_int {
		// SAFETY: The engine keeps the message alive while it is processed.
		unsafe { vcall!(self.as_const() => INetMessage_GetType()) }
	}

	/// Whether the client sent it in its reliable stream.
	#[doc(alias = "IsReliable")]
	pub fn is_reliable(self) -> bool {
		// SAFETY: As for `id`.
		unsafe { vcall!(self.as_const() => INetMessage_IsReliable()) }
	}

	/// The handler method the engine passed the message to.
	pub const fn kind(self) -> IncomingKind {
		self.kind
	}

	/// The engine's name for the message, such as `clc_VoiceData`, or `None`
	/// if the engine returns none.
	#[doc(alias = "GetName")]
	pub fn name(self) -> Option<CString> {
		// SAFETY: As for `id`, and the name is copied at once.
		unsafe { copy_cstr(vcall!(self.as_const() => INetMessage_GetName())) }
	}

	/// The size of the engine's message object, in bytes (`GetSize`).
	fn object_size(self) -> usize {
		// SAFETY: As for `id`.
		unsafe { vcall!(self.as_const() => INetMessage_GetSize()) }
	}
}

/// What the handler decides about a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Verdict {
	/// Lets the engine process the message.
	#[default]
	Continue,

	/// Drops the message. The engine carries on as though it processed it.
	Block,
}

impl IncomingKind {
	/// Every kind, in the order of their handler methods.
	pub const ALL: [Self; 14] = [
		Self::Tick,
		Self::StringCmd,
		Self::SetConVar,
		Self::SignonState,
		Self::ClientInfo,
		Self::Move,
		Self::VoiceData,
		Self::BaselineAck,
		Self::ListenEvents,
		Self::RespondCvarValue,
		Self::FileCrcCheck,
		Self::FileMd5Check,
		Self::SaveReplay,
		Self::CmdKeyValues,
	];

	/// The vtable slot of each kind's handler method, in [`Self::ALL`]'s
	/// order, for the target's ABI.
	pub const HANDLER_SLOTS: [c_int; 14] = [
		handler_slot!(IClientMessageHandler_ProcessTick),
		handler_slot!(IClientMessageHandler_ProcessStringCmd),
		handler_slot!(IClientMessageHandler_ProcessSetConVar),
		handler_slot!(IClientMessageHandler_ProcessSignonState),
		handler_slot!(IClientMessageHandler_ProcessClientInfo),
		handler_slot!(IClientMessageHandler_ProcessMove),
		handler_slot!(IClientMessageHandler_ProcessVoiceData),
		handler_slot!(IClientMessageHandler_ProcessBaselineAck),
		handler_slot!(IClientMessageHandler_ProcessListenEvents),
		handler_slot!(IClientMessageHandler_ProcessRespondCvarValue),
		handler_slot!(IClientMessageHandler_ProcessFileCRCCheck),
		handler_slot!(IClientMessageHandler_ProcessFileMD5Check),
		handler_slot!(IClientMessageHandler_ProcessSaveReplay),
		handler_slot!(IClientMessageHandler_ProcessCmdKeyValues),
	];

	/// The kind at `index` in [`Self::ALL`], or `None` past its end.
	fn from_index(index: c_int) -> Option<Self> {
		Self::ALL.get(usize::try_from(index).ok()?).copied()
	}

	/// The size of the kind's own fields, after the message handler pointer
	/// each message class declares first, and so the size of its class past
	/// `CNetMessage`'s.
	const fn own_size(self) -> usize {
		match self {
			Self::Tick => 24,
			Self::StringCmd => 1040,
			Self::SetConVar => 40,
			Self::SignonState | Self::BaselineAck | Self::CmdKeyValues => 16,
			Self::ClientInfo | Self::ListenEvents => 72,
			Self::Move | Self::VoiceData => 88,
			Self::RespondCvarValue => 552,
			Self::FileCrcCheck => 568,
			Self::FileMd5Check => 544,
			Self::SaveReplay => 280,
		}
	}
}

/// Whether pointers the engine keeps into the message's own buffers confirm
/// that its fields start at `base`.
fn confirms_base(message: IncomingMessage<'_>, base: usize) -> bool {
	let this = message.raw.cast::<u8>().as_ptr();

	// SAFETY: `message_base` checked the object holds the kind's fields past
	// `base`. The engine points these at the message's own buffers when it
	// reads the message.
	unsafe {
		let points_at = |pointer: usize, buffer: usize| {
			this.add(base + pointer)
				.cast::<*const u8>()
				.read_unaligned()
				== this.add(base + buffer).cast_const()
		};

		match message.kind {
			IncomingKind::StringCmd => points_at(8, 16),
			IncomingKind::RespondCvarValue => points_at(16, 36) && points_at(24, 292),
			_ => false,
		}
	}
}

/// Reads `net_SetConVar`'s `CUtlVector<cvar_t>`.
///
/// # Safety
///
/// As for [`decode`].
unsafe fn convars(base: *const u8) -> Option<Vec<(CString, CString)>> {
	// SAFETY: As the caller promises, per `CUtlVector`'s layout in
	// `public/tier1/utlvector.h`: its memory's pointer, capacity, and growth,
	// then its count, then a pointer to its elements.
	let (memory, capacity, count, elements) = unsafe {
		(
			read::<*const u8>(base, 8),
			read::<c_int>(base, 16),
			read::<c_int>(base, 24),
			read::<*const u8>(base, 32),
		)
	};

	let count = usize::try_from(count).ok()?;

	if count > MAX_CONVARS
		|| count > usize::try_from(capacity).ok()?
		|| (count > 0 && (memory.is_null() || memory != elements))
	{
		return None;
	}

	Some(
		(0..count)
			.map(|index| {
				let convar = index * 2 * CONVAR_TEXT;

				// SAFETY: The vector holds `count` elements of two buffers each.
				unsafe {
					(
						string::<CONVAR_TEXT>(memory, convar),
						string::<CONVAR_TEXT>(memory, convar + CONVAR_TEXT),
					)
				}
			})
			.collect(),
	)
}

/// Copies the fields of a message of `kind`, or returns `None` if they are
/// inconsistent.
///
/// # Safety
///
/// `base` must be where the fields of a live message of `kind` start.
unsafe fn decode(kind: IncomingKind, base: *const u8) -> Option<Incoming> {
	// SAFETY: As the caller promises, per the message classes' layouts. Each
	// starts with its message handler pointer.
	unsafe {
		Some(match kind {
			IncomingKind::Tick => Incoming::Tick {
				tick: read(base, 8),
				host_frame_time: read(base, 12),
				host_frame_time_std_deviation: read(base, 16),
			},

			IncomingKind::StringCmd => Incoming::StringCmd {
				command: string::<1024>(base, 16),
			},

			IncomingKind::SetConVar => Incoming::SetConVar {
				convars: convars(base)?,
			},

			IncomingKind::SignonState => Incoming::SignonState {
				state: read(base, 8),
				spawn_count: read(base, 12),
			},

			IncomingKind::ClientInfo => Incoming::ClientInfo {
				send_table_crc: read(base, 8),
				server_count: read(base, 12),
				is_hltv: read::<u8>(base, 16) != 0,
				is_replay: read::<u8>(base, 17) != 0,
				friends_id: read(base, 20),
				friends_name: string::<32>(base, 24),
				custom_files: read(base, 56),
			},

			IncomingKind::Move => Incoming::Move {
				backup_commands: read(base, 8),
				new_commands: read(base, 12),
				data: payload(base.add(24), read(base, 16))?,
			},

			IncomingKind::VoiceData => Incoming::VoiceData {
				data: payload(base.add(16), read(base, 8))?,
			},

			IncomingKind::BaselineAck => Incoming::BaselineAck {
				tick: read(base, 8),
				baseline: read(base, 12),
			},

			IncomingKind::ListenEvents => Incoming::ListenEvents {
				events: read(base, 8),
			},

			IncomingKind::RespondCvarValue => Incoming::RespondCvarValue {
				cookie: read(base, 8),
				status: read(base, 32),
				name: string::<256>(base, 36),
				value: string::<256>(base, 292),
			},

			IncomingKind::FileCrcCheck => Incoming::FileCrcCheck {
				path_id: string::<260>(base, 8),
				file_name: string::<260>(base, 268),
				md5: read(base, 528),
				crc: read(base, 544),
				hash_type: read(base, 548),
				length: read(base, 552),
				pack_file_number: read(base, 556),
				pack_file_id: read(base, 560),
				fraction: read(base, 564),
			},

			IncomingKind::FileMd5Check => Incoming::FileMd5Check {
				path_id: string::<260>(base, 8),
				file_name: string::<260>(base, 268),
				md5: read(base, 528),
			},

			IncomingKind::SaveReplay => Incoming::SaveReplay {
				start_send_byte: read(base, 8),
				file_name: string::<260>(base, 12),
				post_death_record_time: read(base, 272),
			},

			IncomingKind::CmdKeyValues => Incoming::CmdKeyValues,
		})
	}
}

/// Finds a client message handler of the engine's to hook, after checking its
/// class and place through the engine's run-time type information.
pub fn hook_target(server: Server<'_>) -> Result<HookTarget, HookTargetError> {
	let client = server
		.valve_engine()?
		.game_server()
		.ok_or(HookTargetError::NoServer)?
		.client(0)
		.ok_or(HookTargetError::NotReady)?;

	let client = client.as_ptr().cast::<c_void>().cast_const();

	// `CBaseClient` derives from `IGameEventListener2`, `IClient`, then
	// `IClientMessageHandler`, each only a vtable pointer.
	//
	// SAFETY: The client is one of the engine's live clients, which the
	// engine built with run-time type information, like each of its bases.
	let client_offset = unsafe { rtti::subobject_offset(client, "CGameClient") };

	if client_offset != Some(SLOT as isize) {
		return Err(HookTargetError::UnexpectedLayout);
	}

	// SAFETY: The complete object is a `CGameClient`, whose bases confirmed
	// above place the handler right after the client.
	let handler = unsafe { client.byte_add(SLOT) };

	// SAFETY: As above, `handler` is the client's `IClientMessageHandler`
	// base, a polymorphic subobject of the same live `CGameClient`.
	let handler_offset = unsafe { rtti::subobject_offset(handler, "CGameClient") };

	if handler_offset != Some(2 * SLOT as isize) {
		return Err(HookTargetError::UnexpectedLayout);
	}

	HANDLER_OFFSET.store(SLOT, Ordering::Relaxed);

	Ok(HookTarget {
		handler: NonNull::new(handler.cast_mut()).ok_or(HookTargetError::UnexpectedLayout)?,
		slots: IncomingKind::HANDLER_SLOTS,
	})
}

/// Where a message's own fields start, if its layout is the one expected.
fn message_base(message: IncomingMessage<'_>) -> Option<usize> {
	let size = message.object_size();
	let base = size.checked_sub(message.kind.own_size())?;

	if !(SMALLEST_MESSAGE_BASE..=LARGEST_MESSAGE_BASE).contains(&base) || !base.is_multiple_of(SLOT)
	{
		return None;
	}

	match MESSAGE_BASE.load(Ordering::Relaxed) {
		0 if confirms_base(message, base) => {
			MESSAGE_BASE.store(base, Ordering::Relaxed);
			Some(base)
		}

		0 => None,
		known => (known == base).then_some(base),
	}
}

/// The bits of a `bf_read` from its read position, as a message the engine
/// read keeps its payload.
///
/// # Safety
///
/// `reader` must point to a `bf_read` within the live message, whose buffer
/// is the packet the engine is processing.
unsafe fn payload(reader: *const u8, bits: c_int) -> Option<BitWriter> {
	// SAFETY: As the caller promises, per `bf_read`'s layout in
	// `public/tier1/bitbuf.h`.
	let (data, bytes, limit, position) = unsafe {
		(
			read::<*const u8>(reader, 0),
			read::<c_int>(reader, 8),
			read::<c_int>(reader, 12),
			read::<c_int>(reader, 16),
		)
	};

	let bits = usize::try_from(bits).ok()?;
	let position = usize::try_from(position).ok()?;
	let end = position.checked_add(bits)?;

	if data.is_null()
		|| end > usize::try_from(limit).ok()?
		|| end > usize::try_from(bytes).ok()? * 8
	{
		return None;
	}

	let mut out = BitWriter::with_capacity(bits);

	for bit in position..end {
		// SAFETY: `end` is within the buffer's bytes, checked above.
		let byte = unsafe { data.add(bit / 8).read() };

		out.write_bit(byte >> (bit % 8) & 1 != 0);
	}

	Some(out)
}

/// Reads a value at `offset` past `base`.
///
/// # Safety
///
/// The value must lie within the live message.
unsafe fn read<T: Copy>(base: *const u8, offset: usize) -> T {
	// SAFETY: As the caller promises.
	unsafe { base.add(offset).cast::<T>().read_unaligned() }
}

/// Passes a message to `handler`, returning what it decided.
///
/// # Safety
///
/// Call it from a hook on a handler method of the vtable [`hook_target`]
/// found, before the method runs, on the server's main thread: `kind` indexes
/// [`IncomingKind::ALL`], `this` is the handler the engine called, and
/// `message` its argument. Panics are caught, and let the message through.
pub unsafe fn route_incoming(
	binding: &ServerBinding,
	handler: &dyn IncomingHandler,
	kind: c_int,
	this: NonNull<c_void>,
	message: NonNull<c_void>,
) -> Verdict {
	let (Some(kind), offset @ 1..) = (
		IncomingKind::from_index(kind),
		HANDLER_OFFSET.load(Ordering::Relaxed),
	) else {
		return Verdict::Continue;
	};

	let scope = ();

	// SAFETY: The hook runs during the engine's call into the handler, on the
	// main thread.
	let server = unsafe { binding.server(&scope) };

	// SAFETY: Every handler of the hooked class sits `offset` past its client,
	// which `hook_target` confirmed, and the engine keeps its clients.
	let client = unsafe { GameClient::from_raw(this.byte_sub(offset).cast()) };

	let message = IncomingMessage {
		kind,
		raw: message.cast(),
		client,
		_scope: PhantomData,
		_not_thread_safe: PhantomData,
	};

	catch_unwind(AssertUnwindSafe(|| handler.incoming(server, message)))
		.unwrap_or(Verdict::Continue)
}

/// Copies a string from a fixed buffer, up to its terminator or its end.
///
/// # Safety
///
/// As for [`read`], for the whole buffer.
unsafe fn string<const N: usize>(base: *const u8, offset: usize) -> CString {
	// SAFETY: As the caller promises.
	cstring_from_buffer(&unsafe { read::<[c_char; N]>(base, offset) })
}

#[cfg(test)]
pub(crate) mod test_support {
	use super::*;
	use sdk_raw::util::mock::{mock_vtable, unexpected_call};
	use std::ffi::CStr;

	/// The size of `CNetMessage` in mock messages. Every mock that decodes
	/// shares it, since the first message decoded fixes [`MESSAGE_BASE`] for
	/// the process.
	const BASE: usize = SMALLEST_MESSAGE_BASE;

	unsafe extern "C" fn get_size(this: *const sys::INetMessage) -> usize {
		// SAFETY: Every mock message is at least `BASE` bytes, and keeps its
		// reported size in `CNetMessage`'s fields, after its vtable.
		unsafe { this.cast::<u8>().add(SLOT).cast::<usize>().read() }
	}

	/// A message of `kind` from `client`, as [`route_incoming`] passes one.
	///
	/// # Safety
	///
	/// `raw` must answer every virtual call the test makes, and stay alive
	/// for `'s`, as this module's mocks do.
	pub(crate) const unsafe fn message<'s>(
		kind: IncomingKind,
		raw: NonNull<sys::INetMessage>,
		client: GameClient<'s>,
	) -> IncomingMessage<'s> {
		IncomingMessage {
			kind,
			raw,
			client,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// A leaked, zeroed message object of `size` bytes, which reports that
	/// size and answers no other virtual call.
	fn mock_message(size: usize) -> NonNull<u8> {
		assert!(size >= BASE);

		// SAFETY: The vtable holds only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patch only writes a slot
		// of the vtable being built.
		let vtable = Box::leak(unsafe {
			mock_vtable::<sys::INetMessage__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).INetMessage_GetSize).write(get_size);
				},
			)
		});
		let object = Box::leak(vec![0_u64; size.div_ceil(8)].into_boxed_slice());
		let this = object.as_mut_ptr().cast::<u8>();

		// SAFETY: The object is at least `BASE` bytes and aligned for
		// pointers, so the vtable pointer and the size fit before its fields.
		unsafe {
			this.cast::<sys::INetMessage>()
				.write(sys::INetMessage { vtable_: vtable });
			this.add(SLOT).cast::<usize>().write(size);
		}

		NonNull::new(this).unwrap()
	}

	/// A `clc_RespondCvarValue` laid out as [`IncomingMessage::decode`]
	/// expects, answering the query carrying `cookie`.
	pub(crate) fn respond_cvar_value(
		cookie: c_int,
		status: c_int,
		name: &CStr,
		value: &CStr,
	) -> NonNull<sys::INetMessage> {
		let message = mock_message(BASE + IncomingKind::RespondCvarValue.own_size());
		let (name, value) = (name.to_bytes_with_nul(), value.to_bytes_with_nul());

		assert!(name.len() <= 256 && value.len() <= 256);

		// SAFETY: The object holds the kind's fields past `BASE`, at the
		// offsets `decode` and `confirms_base` read, and both strings fit
		// their 256-byte buffers.
		unsafe {
			let fields = message.as_ptr().add(BASE);

			fields.add(8).cast::<c_int>().write_unaligned(cookie);
			fields
				.add(16)
				.cast::<*const u8>()
				.write_unaligned(fields.add(36));
			fields
				.add(24)
				.cast::<*const u8>()
				.write_unaligned(fields.add(292));
			fields.add(32).cast::<c_int>().write_unaligned(status);
			fields
				.add(36)
				.copy_from_nonoverlapping(name.as_ptr(), name.len());
			fields
				.add(292)
				.copy_from_nonoverlapping(value.as_ptr(), value.len());
		}

		message.cast()
	}

	/// A message whose reported size is too small for any kind's fields, as
	/// an engine laid out otherwise might report, which nothing can decode.
	pub(crate) fn unreadable() -> NonNull<sys::INetMessage> {
		mock_message(BASE).cast()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn handler_slots_follow_the_abi() {
		let destructors = if cfg!(target_os = "windows") { 1 } else { 2 };

		assert_eq!(IncomingKind::HANDLER_SLOTS[0], destructors);
		assert_eq!(IncomingKind::HANDLER_SLOTS[6], destructors + 6);
		assert_eq!(IncomingKind::HANDLER_SLOTS[13], destructors + 13);
	}

	#[test]
	fn kinds_map_to_indices() {
		assert_eq!(IncomingKind::from_index(1), Some(IncomingKind::StringCmd));
		assert_eq!(IncomingKind::from_index(14), None);
		assert_eq!(IncomingKind::from_index(-1), None);
	}
}
