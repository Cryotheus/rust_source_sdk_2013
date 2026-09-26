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

use crate::bitbuf::BitWriter;
use crate::ffi::{NotThreadSafe, copy_cstr, cstring_from_buffer, vcall};
use crate::interfaces::game_server::GameClient;
use crate::rtti;
use crate::server::{InterfaceError, Server, ServerBinding};
use std::ffi::{CString, c_char, c_int, c_void};
use std::marker::PhantomData;
use std::mem::offset_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The client message handler methods, one per kind of message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IncomingKind {
	/// `net_Tick`: the client's last received tick and frame times.
	Tick,
	/// `net_StringCmd`: a console command for the server.
	StringCmd,
	/// `net_SetConVar`: changed user settings.
	SetConVar,
	/// `net_SignonState`: progress through connecting.
	SignonState,
	/// `clc_ClientInfo`: the client's identity and custom files.
	ClientInfo,
	/// `clc_Move`: user commands.
	Move,
	/// `clc_VoiceData`: encoded voice.
	VoiceData,
	/// `clc_BaselineAck`: acknowledgement of an entity baseline.
	BaselineAck,
	/// `clc_ListenEvents`: the game events the client wants.
	ListenEvents,
	/// `clc_RespondCvarValue`: the answer to a console variable query.
	RespondCvarValue,
	/// `clc_FileCRCCheck`: a file's hash, for `sv_pure`.
	FileCrcCheck,
	/// `clc_FileMD5Check`: a file's MD5 hash.
	FileMd5Check,
	/// `clc_SaveReplay`: a request to save a replay.
	SaveReplay,
	/// `clc_CmdKeyValues`: a command with key values, such as TF2's Mann vs.
	/// Machine upgrades.
	CmdKeyValues,
}

/// Size of a vtable slot.
const SLOT: usize = size_of::<*const ()>();

macro_rules! handler_slot {
	($method:ident) => {
		(offset_of!(sys::IClientMessageHandler__bindgen_vtable, $method) / SLOT) as c_int
	};
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

/// What the handler decides about a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Verdict {
	/// Lets the engine process the message.
	#[default]
	Continue,

	/// Drops the message. The engine carries on as though it processed it.
	Block,
}

/// Decides about the messages clients send.
pub trait IncomingHandler: 'static {
	/// Called for each message a client sends, before the engine processes
	/// it, during the engine's processing of the client's packet.
	fn incoming(&self, server: Server<'_>, message: IncomingMessage<'_>) -> Verdict;
}

/// A message a client sent, before the engine processed it.
#[derive(Debug, Clone, Copy)]
pub struct IncomingMessage<'s> {
	kind: IncomingKind,
	raw: NonNull<sys::INetMessage>,
	client: GameClient<'s>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> IncomingMessage<'s> {
	pub const fn kind(self) -> IncomingKind {
		self.kind
	}

	/// The client that sent the message.
	pub const fn client(self) -> GameClient<'s> {
		self.client
	}

	/// The engine's message, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::INetMessage {
		self.raw.as_ptr()
	}

	const fn as_const(self) -> *const sys::INetMessage {
		self.raw.as_ptr().cast_const()
	}

	/// The message's type, as the client numbers its messages.
	#[doc(alias = "GetType")]
	pub fn id(self) -> c_int {
		// SAFETY: The engine keeps the message alive while it is processed.
		unsafe { vcall!(self.as_const() => INetMessage_GetType()) }
	}

	/// The engine's name for the message, such as `clc_VoiceData`.
	#[doc(alias = "GetName")]
	pub fn name(self) -> Option<CString> {
		// SAFETY: As for `id`, and the name is copied at once.
		unsafe { copy_cstr(vcall!(self.as_const() => INetMessage_GetName())) }
	}

	/// The group the engine counts the message's traffic in.
	#[doc(alias = "GetGroup")]
	pub fn group(self) -> c_int {
		// SAFETY: As for `id`.
		unsafe { vcall!(self.as_const() => INetMessage_GetGroup()) }
	}

	/// Whether the client sent it in its reliable stream.
	#[doc(alias = "IsReliable")]
	pub fn is_reliable(self) -> bool {
		// SAFETY: As for `id`.
		unsafe { vcall!(self.as_const() => INetMessage_IsReliable()) }
	}

	/// The engine's own description of the message and its fields.
	#[doc(alias = "ToString")]
	pub fn describe(self) -> Option<CString> {
		// SAFETY: As for `id`. The engine formats into a buffer it reuses, so
		// the text is copied at once.
		unsafe { copy_cstr(vcall!(self.as_const() => INetMessage_ToString())) }
	}

	/// The size of the engine's message object.
	fn object_size(self) -> usize {
		// SAFETY: As for `id`.
		unsafe { vcall!(self.as_const() => INetMessage_GetSize()) }
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
}

/// The size of `CNetMessage`, which the TF2 engine's message classes extend.
/// It is learned from the first message whose own pointers confirm it.
static MESSAGE_BASE: AtomicUsize = AtomicUsize::new(0);

/// `CNetMessage` holds a vtable pointer, a flag, and a channel pointer in the
/// SDK's 2013 release; the TF2 engine may add fields after them.
const SMALLEST_MESSAGE_BASE: usize = 24;

/// Where a message's own fields start, if its layout is the one expected.
fn message_base(message: IncomingMessage<'_>) -> Option<usize> {
	let size = message.object_size();
	let base = size.checked_sub(message.kind.own_size())?;

	if !(SMALLEST_MESSAGE_BASE..=256).contains(&base) || !base.is_multiple_of(SLOT) {
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

/// A message's fields.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
	Tick {
		tick: c_int,
		host_frame_time: f32,
		host_frame_time_std_deviation: f32,
	},
	StringCmd {
		command: CString,
	},
	SetConVar {
		convars: Vec<(CString, CString)>,
	},
	SignonState {
		state: c_int,
		spawn_count: c_int,
	},
	ClientInfo {
		send_table_crc: u32,
		server_count: c_int,
		is_hltv: bool,
		is_replay: bool,
		friends_id: u32,
		friends_name: CString,
		custom_files: [u32; 4],
	},
	Move {
		backup_commands: c_int,
		new_commands: c_int,
		/// The encoded user commands.
		data: BitWriter,
	},
	VoiceData {
		/// The encoded voice.
		data: BitWriter,
	},
	BaselineAck {
		tick: c_int,
		baseline: c_int,
	},
	ListenEvents {
		/// A bit per game event ID.
		events: [u32; 16],
	},
	RespondCvarValue {
		cookie: c_int,
		/// `EQueryCvarValueStatus`: 0 when the value was found.
		status: c_int,
		name: CString,
		value: CString,
	},
	FileCrcCheck {
		path_id: CString,
		file_name: CString,
		md5: [u8; 16],
		crc: u32,
		hash_type: c_int,
		length: c_int,
		pack_file_number: c_int,
		pack_file_id: c_int,
		fraction: c_int,
	},
	FileMd5Check {
		path_id: CString,
		file_name: CString,
		md5: [u8; 16],
	},
	SaveReplay {
		start_send_byte: c_int,
		file_name: CString,
		post_death_record_time: f32,
	},
	/// The key values are not decoded.
	CmdKeyValues,
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

/// Copies a string from a fixed buffer, up to its terminator or its end.
///
/// # Safety
///
/// As for [`read`], for the whole buffer.
unsafe fn string<const N: usize>(base: *const u8, offset: usize) -> CString {
	// SAFETY: As the caller promises.
	cstring_from_buffer(&unsafe { read::<[c_char; N]>(base, offset) })
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

/// The most variables a `net_SetConVar` holds, which is its count's range.
const MAX_CONVARS: usize = 255;

/// Bytes of a name or value in `net_SetConVar`'s `cvar_t` (`MAX_OSPATH`).
const CONVAR_TEXT: usize = 260;

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

/// How far a client's message handler sits past its `IClient`, once
/// [`hook_target`] has confirmed it.
static HANDLER_OFFSET: AtomicUsize = AtomicUsize::new(0);

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
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	#[error("the engine has no game server")]
	NoServer,

	#[error("the server has no client objects yet")]
	NotReady,

	#[error("the engine's clients are not laid out as expected")]
	UnexpectedLayout,
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
