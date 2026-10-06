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
use sdk_raw::net::incoming::{self as raw, ClientLayoutError, ClientMessage, MessageClass};
use sdk_raw::util::cstr::{copy_cstr, cstring_from_buffer};
use sdk_raw::vcall;
use std::ffi::{CString, c_int};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;

/// What a plugin hooks to see clients' messages.
#[derive(Debug, Clone, Copy)]
pub struct HookTarget {
	/// One of the engine's client message handlers. Every handler of its class
	/// shares its vtable.
	pub handler: NonNull<sys::IClientMessageHandler>,

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

impl From<ClientLayoutError> for HookTargetError {
	fn from(_: ClientLayoutError) -> Self {
		Self::UnexpectedLayout
	}
}

/// A message's fields, as [`IncomingMessage::decode`] copies them.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
	/// The client's last received tick and frame times (`NET_Tick`).
	#[doc(alias("NET_Tick"))]
	Tick {
		/// The last server tick the client received.
		tick: c_int,

		/// The client's frame time, in seconds.
		host_frame_time: f32,

		/// The standard deviation of the client's frame time, in seconds.
		host_frame_time_std_deviation: f32,
	},

	/// A console command for the server (`NET_StringCmd`).
	#[doc(alias("NET_StringCmd"))]
	StringCmd {
		/// The command and its arguments.
		command: CString,
	},

	/// Changed user settings (`NET_SetConVar`).
	#[doc(alias("NET_SetConVar"))]
	SetConVar {
		/// Names and values; at most 255.
		convars: Vec<(CString, CString)>,
	},

	/// Progress through connecting (`NET_SignonState`).
	#[doc(alias("NET_SignonState"))]
	SignonState {
		/// The sign-on state the client reached, a `SIGNONSTATE_*` value.
		state: c_int,

		/// The server's spawn count the state refers to.
		spawn_count: c_int,
	},

	/// The client's identity and custom files (`CLC_ClientInfo`).
	#[doc(alias("CLC_ClientInfo"))]
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
	#[doc(alias("CLC_Move"))]
	Move {
		/// Commands sent before, repeated in case their packets were lost.
		backup_commands: c_int,

		/// Commands the client has not sent before.
		new_commands: c_int,

		/// The encoded user commands.
		data: BitWriter,
	},

	/// Encoded voice (`CLC_VoiceData`).
	#[doc(alias("CLC_VoiceData"))]
	VoiceData {
		/// The encoded voice.
		data: BitWriter,
	},

	/// Acknowledgement of an entity baseline (`CLC_BaselineAck`).
	#[doc(alias("CLC_BaselineAck"))]
	BaselineAck {
		/// The tick of the baseline the client acknowledges.
		tick: c_int,

		/// Which of the client's baselines it acknowledges.
		baseline: c_int,
	},

	/// The game events the client wants (`CLC_ListenEvents`).
	#[doc(alias("CLC_ListenEvents"))]
	ListenEvents {
		/// A bit per game event ID.
		events: [u32; 16],
	},

	/// The answer to a console variable query (`CLC_RespondCvarValue`).
	#[doc(alias("CLC_RespondCvarValue"))]
	RespondCvarValue {
		/// The cookie of the query answered.
		cookie: c_int,

		/// The `EQueryCvarValueStatus` of the answer:
		/// [`QUERY_CVAR_VALUE_INTACT`] when the value was found,
		/// [`QUERY_CVAR_NOT_FOUND`] when no variable has the name,
		/// [`QUERY_CVAR_NOT_A_CVAR`] when a command has it instead, and
		/// [`QUERY_CVAR_PROTECTED`] when the variable does not allow queries.
		///
		/// [`QUERY_CVAR_VALUE_INTACT`]: sdk_raw::interfaces::plugin_helpers::QUERY_CVAR_VALUE_INTACT
		/// [`QUERY_CVAR_NOT_FOUND`]: sdk_raw::interfaces::plugin_helpers::QUERY_CVAR_NOT_FOUND
		/// [`QUERY_CVAR_NOT_A_CVAR`]: sdk_raw::interfaces::plugin_helpers::QUERY_CVAR_NOT_A_CVAR
		/// [`QUERY_CVAR_PROTECTED`]: sdk_raw::interfaces::plugin_helpers::QUERY_CVAR_PROTECTED
		status: c_int,

		/// The variable's name.
		name: CString,

		/// The variable's value.
		value: CString,
	},

	/// A file's hash, for `sv_pure` (`CLC_FileCRCCheck`).
	#[doc(alias("CLC_FileCRCCheck"))]
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
	#[doc(alias("CLC_FileMD5Check"))]
	FileMd5Check {
		/// The search path ID the file was found under, such as `GAME`.
		path_id: CString,

		/// The file's path.
		file_name: CString,

		/// The file's MD5 hash.
		md5: [u8; 16],
	},

	/// A request to save a replay (`CLC_SaveReplay`).
	#[doc(alias("CLC_SaveReplay"))]
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
	#[doc(alias("CLC_CmdKeyValues"))]
	CmdKeyValues,
}

impl Incoming {
	/// The fields [`raw::read_message`] copied, as their types here.
	fn from_raw(message: ClientMessage) -> Self {
		match message {
			ClientMessage::Tick(fields) => Self::Tick {
				tick: fields.tick,
				host_frame_time: fields.host_frame_time,
				host_frame_time_std_deviation: fields.host_frame_time_std_deviation,
			},

			ClientMessage::StringCmd(fields) => Self::StringCmd {
				command: cstring_from_buffer(&fields.command_buffer),
			},

			ClientMessage::SetConVar(convars) => Self::SetConVar {
				convars: convars
					.iter()
					.map(|convar| {
						(
							cstring_from_buffer(&convar.name),
							cstring_from_buffer(&convar.value),
						)
					})
					.collect(),
			},

			ClientMessage::SignonState(fields) => Self::SignonState {
				state: fields.signon_state,
				spawn_count: fields.spawn_count,
			},

			ClientMessage::ClientInfo(fields) => Self::ClientInfo {
				send_table_crc: fields.send_table_crc,
				server_count: fields.server_count,
				is_hltv: fields.is_hltv != 0,
				is_replay: fields.is_replay != 0,
				friends_id: fields.friends_id,
				friends_name: cstring_from_buffer(&fields.friends_name),
				custom_files: fields.custom_files,
			},

			ClientMessage::Move { fields, data } => Self::Move {
				backup_commands: fields.backup_commands,
				new_commands: fields.new_commands,
				data: data.into(),
			},

			ClientMessage::VoiceData { data, .. } => Self::VoiceData { data: data.into() },

			ClientMessage::BaselineAck(fields) => Self::BaselineAck {
				tick: fields.baseline_tick,
				baseline: fields.baseline_number,
			},

			ClientMessage::ListenEvents(fields) => Self::ListenEvents {
				events: fields.events,
			},

			ClientMessage::RespondCvarValue(fields) => Self::RespondCvarValue {
				cookie: fields.cookie,
				status: fields.status,
				name: cstring_from_buffer(&fields.name_buffer),
				value: cstring_from_buffer(&fields.value_buffer),
			},

			ClientMessage::FileCrcCheck(fields) => Self::FileCrcCheck {
				path_id: cstring_from_buffer(&fields.path_id),
				file_name: cstring_from_buffer(&fields.file_name),
				md5: fields.md5,
				crc: fields.crc,
				hash_type: fields.hash_type,
				length: fields.length,
				pack_file_number: fields.pack_file_number,
				pack_file_id: fields.pack_file_id,
				fraction: fields.fraction,
			},

			ClientMessage::FileMd5Check(fields) => Self::FileMd5Check {
				path_id: cstring_from_buffer(&fields.path_id),
				file_name: cstring_from_buffer(&fields.file_name),
				md5: fields.md5,
			},

			ClientMessage::SaveReplay(fields) => Self::SaveReplay {
				start_send_byte: fields.start_send_byte,
				file_name: cstring_from_buffer(&fields.file_name),
				post_death_record_time: fields.post_death_record_time,
			},

			ClientMessage::CmdKeyValues(_) => Self::CmdKeyValues,
		}
	}
}

/// Decides about the messages clients send.
pub trait IncomingHandler: 'static {
	/// Called for each message a client sends, before the engine processes
	/// it, during the engine's processing of the client's packet.
	fn incoming(&self, server: Server<'_>, message: IncomingMessage<'_>) -> Verdict;
}

/// The client message handler methods, one per kind of message.
#[doc(alias("IClientMessageHandler"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IncomingKind {
	/// `net_Tick`: the client's last received tick and frame times.
	#[doc(alias("ProcessTick"))]
	Tick,
	/// `net_StringCmd`: a console command for the server.
	#[doc(alias("ProcessStringCmd"))]
	StringCmd,
	/// `net_SetConVar`: changed user settings.
	#[doc(alias("ProcessSetConVar"))]
	SetConVar,
	/// `net_SignonState`: progress through connecting.
	#[doc(alias("ProcessSignonState"))]
	SignonState,
	/// `clc_ClientInfo`: the client's identity and custom files.
	#[doc(alias("ProcessClientInfo"))]
	ClientInfo,
	/// `clc_Move`: user commands.
	#[doc(alias("ProcessMove"))]
	Move,
	/// `clc_VoiceData`: encoded voice.
	#[doc(alias("ProcessVoiceData"))]
	VoiceData,
	/// `clc_BaselineAck`: acknowledgement of an entity baseline.
	#[doc(alias("ProcessBaselineAck"))]
	BaselineAck,
	/// `clc_ListenEvents`: the game events the client wants.
	#[doc(alias("ProcessListenEvents"))]
	ListenEvents,
	/// `clc_RespondCvarValue`: the answer to a console variable query.
	#[doc(alias("ProcessRespondCvarValue"))]
	RespondCvarValue,
	/// `clc_FileCRCCheck`: a file's hash, for `sv_pure`.
	#[doc(alias("ProcessFileCRCCheck"))]
	FileCrcCheck,
	/// `clc_FileMD5Check`: a file's MD5 hash.
	#[doc(alias("ProcessFileMD5Check"))]
	FileMd5Check,
	/// `clc_SaveReplay`: a request to save a replay.
	#[doc(alias("ProcessSaveReplay"))]
	SaveReplay,
	/// `clc_CmdKeyValues`: a command with key values, such as TF2's Mann vs.
	/// Machine upgrades.
	#[doc(alias("ProcessCmdKeyValues"))]
	CmdKeyValues,
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
	pub const HANDLER_SLOTS: [c_int; 14] = {
		let mut slots = [0; 14];
		let mut index = 0;

		while index < slots.len() {
			let slot = Self::ALL[index].raw().process_slot();

			assert!(slot <= c_int::MAX as usize);
			slots[index] = slot as c_int;
			index += 1;
		}

		slots
	};

	/// The kind at `index` in [`Self::ALL`], or `None` past its end.
	fn from_index(index: c_int) -> Option<Self> {
		Self::ALL.get(usize::try_from(index).ok()?).copied()
	}

	/// The engine's class of the kind's messages.
	const fn raw(self) -> MessageClass {
		match self {
			Self::Tick => MessageClass::Tick,
			Self::StringCmd => MessageClass::StringCmd,
			Self::SetConVar => MessageClass::SetConVar,
			Self::SignonState => MessageClass::SignonState,
			Self::ClientInfo => MessageClass::ClientInfo,
			Self::Move => MessageClass::Move,
			Self::VoiceData => MessageClass::VoiceData,
			Self::BaselineAck => MessageClass::BaselineAck,
			Self::ListenEvents => MessageClass::ListenEvents,
			Self::RespondCvarValue => MessageClass::RespondCvarValue,
			Self::FileCrcCheck => MessageClass::FileCrcCheck,
			Self::FileMd5Check => MessageClass::FileMd5Check,
			Self::SaveReplay => MessageClass::SaveReplay,
			Self::CmdKeyValues => MessageClass::CmdKeyValues,
		}
	}
}

/// A message a client sent, before the engine processed it.
#[doc(alias("INetMessage"))]
#[derive(Debug, Clone, Copy)]
pub struct IncomingMessage<'s> {
	kind: IncomingKind,
	raw: NonNull<sys::INetMessage>,
	client: GameClient<'s>,

	/// The client's handler the engine passed the message to.
	handler: NonNull<sys::IClientMessageHandler>,

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
		// SAFETY: The engine passed the message to the client's handler method
		// for its kind, and keeps it alive and unchanged while it is processed,
		// which the scope `'s` lies within, during a callback from the engine's
		// module.
		let message = unsafe { raw::read_message(self.kind.raw(), self.handler, self.raw) }?;

		Some(Incoming::from_raw(message))
	}

	/// The engine's own description of the message and its fields, or `None`
	/// if the engine returns none.
	#[doc(alias("ToString"))]
	pub fn describe(self) -> Option<CString> {
		// SAFETY: As for `id`. The engine formats into a buffer it reuses, so
		// the text is copied at once.
		unsafe { copy_cstr(vcall!(self.as_const() => INetMessage_ToString())) }
	}

	/// The group the engine counts the message's traffic in.
	#[doc(alias("GetGroup"))]
	pub fn group(self) -> c_int {
		// SAFETY: As for `id`.
		unsafe { vcall!(self.as_const() => INetMessage_GetGroup()) }
	}

	/// The message's type, as the client numbers its messages.
	#[doc(alias("GetType"))]
	pub fn id(self) -> c_int {
		// SAFETY: The engine keeps the message alive while it is processed.
		unsafe { vcall!(self.as_const() => INetMessage_GetType()) }
	}

	/// Whether the client sent it in its reliable stream.
	#[doc(alias("IsReliable"))]
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
	#[doc(alias("GetName"))]
	pub fn name(self) -> Option<CString> {
		// SAFETY: As for `id`, and the name is copied at once.
		unsafe { copy_cstr(vcall!(self.as_const() => INetMessage_GetName())) }
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

/// Finds a client message handler of the engine's to hook, after checking its
/// class and place through the engine's run-time type information.
pub fn hook_target(server: Server<'_>) -> Result<HookTarget, HookTargetError> {
	let client = server
		.valve_engine()?
		.game_server()
		.ok_or(HookTargetError::NoServer)?
		.client(0)
		.ok_or(HookTargetError::NotReady)?;

	let client = NonNull::new(client.as_ptr()).ok_or(HookTargetError::NotReady)?;

	// SAFETY: The client is one of the engine's live clients, which the engine
	// built with run-time type information, like each of its bases. The
	// engine's module stays loaded during the server's callback.
	let handler = unsafe { raw::handler_of_client(client) }?;

	Ok(HookTarget {
		handler,
		slots: IncomingKind::HANDLER_SLOTS,
	})
}

/// A message of `kind` from `client`, as [`route_incoming`] passes one, to a
/// handler that no mock message names.
///
/// A test seam: the message's fields are private, so the shared test
/// support cannot build one.
///
/// # Safety
///
/// `raw` must answer every virtual call the test makes, and stay alive for
/// `'s`, as the test support's mock messages do.
#[cfg(test)]
pub(crate) const unsafe fn mock_incoming_message<'s>(
	kind: IncomingKind,
	raw: NonNull<sys::INetMessage>,
	client: GameClient<'s>,
) -> IncomingMessage<'s> {
	IncomingMessage {
		kind,
		raw,
		client,
		handler: NonNull::dangling(),
		_scope: PhantomData,
		_not_thread_safe: PhantomData,
	}
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
	this: NonNull<sys::IClientMessageHandler>,
	message: NonNull<sys::INetMessage>,
) -> Verdict {
	let Some(kind) = IncomingKind::from_index(kind) else {
		return Verdict::Continue;
	};

	// SAFETY: As the caller promises, `this` is a handler of the class whose
	// vtable `hook_target` found, a base of one of the engine's clients.
	let Some(client) = (unsafe { raw::client_of_handler(this) }) else {
		return Verdict::Continue;
	};

	let scope = ();

	// SAFETY: The hook runs during the engine's call into the handler, on the
	// main thread.
	let server = unsafe { binding.server(&scope) };

	// SAFETY: The engine keeps its clients while it processes their messages.
	let client = unsafe { GameClient::from_raw(client) };

	let message = IncomingMessage {
		kind,
		raw: message,
		client,
		handler: this,
		_scope: PhantomData,
		_not_thread_safe: PhantomData,
	};

	catch_unwind(AssertUnwindSafe(|| handler.incoming(server, message)))
		.unwrap_or(Verdict::Continue)
}
