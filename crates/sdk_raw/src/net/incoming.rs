//! Hand-written layouts of the engine's own classes for the messages clients
//! send, which no public header declares, and of the clients that handle them.
//!
//! The engine passes each message a client sends to a `Process*` method of
//! the client's message handler, `IClientMessageHandler`, one per
//! [`MessageClass`]. A message is an object of the engine's class for its
//! kind, such as `CLC_RespondCvarValue`, which derives from `CNetMessage`,
//! then `INetMessage`. The public headers only declare those classes, so the
//! fields here are those the TF2 engine is observed to hold, and
//! [`read_message`] only copies them once the engine's reported sizes, and
//! pointers the engine keeps into its own messages, confirm where they lie.
//!
//! The engine's clients are `CGameClient`s, whose base `CBaseClient` derives
//! from `IGameEventListener2`, `IClient`, then `IClientMessageHandler`, each
//! only a vtable pointer. [`handler_of_client`] confirms that layout through
//! the clients' run-time type information, and [`client_of_handler`] relies
//! on it.

use crate::bitbuf::{BfRead, BfWrite, Bits};
use crate::tier0::MAX_PATH;
use crate::util::rtti;
use crate::{vcall, vtable_slot};
use std::ffi::{c_char, c_int};
use std::mem::offset_of;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicUsize, Ordering};

/// `bool IClientMessageHandler::Process*(T *message)`, the type of each
/// method the engine passes clients' messages to, with the message as the
/// `INetMessage` it derives from.
///
/// Every message class derives from `CNetMessage`, then `INetMessage`, each
/// its only base, so a message's address is its `INetMessage`'s, which C++
/// passes as it passes any pointer.
#[doc(alias("Process", "ProcessMessage"))]
pub type ProcessMessageFn = unsafe extern "C" fn(
	this: *mut sys::IClientMessageHandler,
	message: *mut sys::INetMessage,
) -> bool;

const _: () = {
	assert!(size_of::<ConVarEntry>() == 2 * MAX_OSPATH);

	assert!(size_of::<TickFields>() == 24);
	assert!(offset_of!(TickFields, tick) == 8);
	assert!(offset_of!(TickFields, host_frame_time) == 12);
	assert!(offset_of!(TickFields, host_frame_time_std_deviation) == 16);

	assert!(size_of::<StringCmdFields>() == 1040);
	assert!(offset_of!(StringCmdFields, command) == 8);
	assert!(offset_of!(StringCmdFields, command_buffer) == 16);

	assert!(size_of::<SetConVarFields>() == 40);
	assert!(offset_of!(SetConVarFields, convars) == 8);

	assert!(size_of::<SignonStateFields>() == 16);
	assert!(offset_of!(SignonStateFields, signon_state) == 8);
	assert!(offset_of!(SignonStateFields, spawn_count) == 12);

	assert!(size_of::<ClientInfoFields>() == 72);
	assert!(offset_of!(ClientInfoFields, send_table_crc) == 8);
	assert!(offset_of!(ClientInfoFields, server_count) == 12);
	assert!(offset_of!(ClientInfoFields, is_hltv) == 16);
	assert!(offset_of!(ClientInfoFields, is_replay) == 17);
	assert!(offset_of!(ClientInfoFields, friends_id) == 20);
	assert!(offset_of!(ClientInfoFields, friends_name) == 24);
	assert!(offset_of!(ClientInfoFields, custom_files) == 56);

	assert!(size_of::<MoveFields>() == 88);
	assert!(offset_of!(MoveFields, backup_commands) == 8);
	assert!(offset_of!(MoveFields, new_commands) == 12);
	assert!(offset_of!(MoveFields, length) == 16);
	assert!(offset_of!(MoveFields, data_in) == 24);
	assert!(offset_of!(MoveFields, data_out) == 56);

	assert!(size_of::<VoiceDataFields>() == 88);
	assert!(offset_of!(VoiceDataFields, length) == 8);
	assert!(offset_of!(VoiceDataFields, data_in) == 16);
	assert!(offset_of!(VoiceDataFields, data_out) == 48);
	assert!(offset_of!(VoiceDataFields, rest) == 80);

	assert!(size_of::<BaselineAckFields>() == 16);
	assert!(offset_of!(BaselineAckFields, baseline_tick) == 8);
	assert!(offset_of!(BaselineAckFields, baseline_number) == 12);

	assert!(size_of::<ListenEventsFields>() == 72);
	assert!(offset_of!(ListenEventsFields, events) == 8);

	assert!(size_of::<RespondCvarValueFields>() == 552);
	assert!(offset_of!(RespondCvarValueFields, cookie) == 8);
	assert!(offset_of!(RespondCvarValueFields, name) == 16);
	assert!(offset_of!(RespondCvarValueFields, value) == 24);
	assert!(offset_of!(RespondCvarValueFields, status) == 32);
	assert!(offset_of!(RespondCvarValueFields, name_buffer) == 36);
	assert!(offset_of!(RespondCvarValueFields, value_buffer) == 292);

	assert!(size_of::<FileCrcCheckFields>() == 568);
	assert!(offset_of!(FileCrcCheckFields, path_id) == 8);
	assert!(offset_of!(FileCrcCheckFields, file_name) == 268);
	assert!(offset_of!(FileCrcCheckFields, md5) == 528);
	assert!(offset_of!(FileCrcCheckFields, crc) == 544);
	assert!(offset_of!(FileCrcCheckFields, hash_type) == 548);
	assert!(offset_of!(FileCrcCheckFields, length) == 552);
	assert!(offset_of!(FileCrcCheckFields, pack_file_number) == 556);
	assert!(offset_of!(FileCrcCheckFields, pack_file_id) == 560);
	assert!(offset_of!(FileCrcCheckFields, fraction) == 564);

	assert!(size_of::<FileMd5CheckFields>() == 544);
	assert!(offset_of!(FileMd5CheckFields, path_id) == 8);
	assert!(offset_of!(FileMd5CheckFields, file_name) == 268);
	assert!(offset_of!(FileMd5CheckFields, md5) == 528);

	assert!(size_of::<SaveReplayFields>() == 280);
	assert!(offset_of!(SaveReplayFields, start_send_byte) == 8);
	assert!(offset_of!(SaveReplayFields, file_name) == 12);
	assert!(offset_of!(SaveReplayFields, post_death_record_time) == 272);

	assert!(size_of::<CmdKeyValuesFields>() == 16);
	assert!(offset_of!(CmdKeyValuesFields, handler) == 8);
};

/// Where a `CGameClient`'s `IClient` base lies: after its
/// `IGameEventListener2` base, which is only a vtable pointer.
pub(super) const CLIENT_OFFSET: usize = size_of::<sys::IGameEventListener2>();

/// How far a client's `IClientMessageHandler` base lies past its `IClient`
/// base, which is only a vtable pointer.
const CLIENT_TO_HANDLER: usize = size_of::<sys::IClient>();

/// The class whose objects are the engine's clients.
pub(super) const GAME_CLIENT: &str = "CGameClient";

/// The largest size of `CNetMessage` [`read_message`] accepts. A larger one
/// is taken as a layout this module does not know.
const LARGEST_MESSAGE_BASE: usize = 256;

/// How many custom files, such as sprays, a client has.
///
/// This is `MAX_CUSTOM_FILES` from `public/const.h`.
const MAX_CUSTOM_FILES: usize = 4;

/// How many game events the engine can describe, one per index.
///
/// This is `MAX_EVENT_NUMBER` from `public/igameevents.h`.
const MAX_EVENT_NUMBER: usize = 1 << 9;

/// The size of a buffer for a file path, terminator included.
///
/// This is `MAX_OSPATH` from `common/qlimits.h`.
pub const MAX_OSPATH: usize = 260;

/// The size of a buffer for a player's name, terminator included.
///
/// This is `MAX_PLAYER_NAME_LENGTH` from `public/const.h`.
const MAX_PLAYER_NAME_LENGTH: usize = 32;

/// The most variables one `NET_SetConVar` holds, since the engine reads
/// their count as a byte.
pub const MAX_SET_CONVARS: usize = 255;

/// The size of `CNetMessage` in the SDK's 2013 release: a vtable pointer, a
/// flag, and a channel pointer. The TF2 engine may add fields after them, so
/// [`read_message`] accepts larger sizes too.
pub const SMALLEST_MESSAGE_BASE: usize = 24;

/// How far a client's `IClientMessageHandler` base lies past its `IClient`
/// base, once [`handler_of_client`] has confirmed it, or zero before.
static HANDLER_OFFSET: AtomicUsize = AtomicUsize::new(0);

/// The size of `CNetMessage`, which the TF2 engine's message classes extend.
/// It is learned from the first message whose fields confirm where they
/// start.
static MESSAGE_BASE: AtomicUsize = AtomicUsize::new(0);

/// The fields `CLC_BaselineAck` declares.
#[doc(alias("CLC_BaselineAck"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct BaselineAckFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::IClientMessageHandler,

	/// The tick of the baseline the client acknowledges (`m_nBaselineTick`).
	pub baseline_tick: c_int,

	/// Which of the client's baselines it acknowledges (`m_nBaselineNr`).
	pub baseline_number: c_int,
}

/// The fields `CLC_ClientInfo` declares.
#[doc(alias("CLC_ClientInfo"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ClientInfoFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::IClientMessageHandler,

	/// A CRC of the client's send tables (`m_nSendTableCRC`).
	pub send_table_crc: sys::CRC32_t,

	/// The server's spawn count the information is for (`m_nServerCount`).
	pub server_count: c_int,

	/// Whether the client is SourceTV (`m_bIsHLTV`), a C++ `bool`.
	pub is_hltv: u8,

	/// Whether the client is a replay client (`m_bIsReplay`), a C++ `bool`.
	pub is_replay: u8,

	/// The client's Steam friends ID (`m_nFriendsID`).
	pub friends_id: u32,

	/// The client's Steam friends name (`m_FriendsName`).
	pub friends_name: [c_char; MAX_PLAYER_NAME_LENGTH],

	/// CRCs of the client's custom files, such as its spray
	/// (`m_nCustomFiles`).
	pub custom_files: [sys::CRC32_t; MAX_CUSTOM_FILES],
}

/// The engine's clients are not laid out as [`handler_of_client`] or
/// [`listener_of_client`] expects.
///
/// [`listener_of_client`]: super::events::listener_of_client
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the engine's clients are not laid out as expected")]
pub struct ClientLayoutError;

/// A message's fields, as [`read_message`] copies them.
///
/// Pointers in the fields point into the engine's message, or the packet it
/// came in, which only last for the engine's call that passed the message.
#[derive(Debug)]
pub enum ClientMessage {
	/// `NET_Tick`'s fields.
	Tick(TickFields),

	/// `NET_StringCmd`'s fields.
	StringCmd(Box<StringCmdFields>),

	/// The variables of `NET_SetConVar`'s vector.
	SetConVar(Vec<ConVarEntry>),

	/// `NET_SignonState`'s fields.
	SignonState(SignonStateFields),

	/// `CLC_ClientInfo`'s fields.
	ClientInfo(ClientInfoFields),

	/// `CLC_Move`'s fields.
	Move {
		/// The fields.
		fields: MoveFields,

		/// The encoded user commands, from [`MoveFields::data_in`].
		data: Bits,
	},

	/// `CLC_VoiceData`'s fields.
	VoiceData {
		/// The fields.
		fields: VoiceDataFields,

		/// The encoded voice, from [`VoiceDataFields::data_in`].
		data: Bits,
	},

	/// `CLC_BaselineAck`'s fields.
	BaselineAck(BaselineAckFields),

	/// `CLC_ListenEvents`'s fields.
	ListenEvents(ListenEventsFields),

	/// `CLC_RespondCvarValue`'s fields.
	RespondCvarValue(RespondCvarValueFields),

	/// `CLC_FileCRCCheck`'s fields.
	FileCrcCheck(FileCrcCheckFields),

	/// `CLC_FileMD5Check`'s fields.
	FileMd5Check(FileMd5CheckFields),

	/// `CLC_SaveReplay`'s fields.
	SaveReplay(SaveReplayFields),

	/// `CLC_CmdKeyValues`'s fields.
	CmdKeyValues(CmdKeyValuesFields),
}

/// The fields `CLC_CmdKeyValues` holds.
#[doc(alias("CLC_CmdKeyValues"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct CmdKeyValuesFields {
	/// The key values, which its base, `Base_CmdKeyValues`, declares
	/// (`m_pKeyValues`).
	pub key_values: *mut sys::KeyValues,

	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::IClientMessageHandler,
}

/// A variable of `NET_SetConVar` (`cvar_t`).
#[doc(alias("cvar_t"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ConVarEntry {
	/// The variable's name (`name`).
	pub name: [c_char; MAX_OSPATH],

	/// The variable's value (`value`).
	pub value: [c_char; MAX_OSPATH],
}

/// The fields `CLC_FileCRCCheck` declares.
#[doc(alias("CLC_FileCRCCheck"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FileCrcCheckFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::IClientMessageHandler,

	/// The search path ID the file was found under, such as `GAME`
	/// (`m_szPathID`).
	pub path_id: [c_char; MAX_PATH],

	/// The file's path (`m_szFilename`).
	pub file_name: [c_char; MAX_PATH],

	/// The file's MD5 hash (`m_MD5`).
	pub md5: [u8; 16],

	/// The CRC the client reports with the hash (`m_CRCIOs`).
	pub crc: sys::CRC32_t,

	/// The kind of hash in [`md5`](Self::md5) (`m_eFileHashType`).
	pub hash_type: c_int,

	/// The file's size, in bytes (`m_cbFileLen`).
	pub length: c_int,

	/// The number of the pack file holding the file (`m_nPackFileNumber`).
	pub pack_file_number: c_int,

	/// The ID of the pack file holding the file (`m_PackFileID`).
	pub pack_file_id: c_int,

	/// The file fraction the client reports with the hash
	/// (`m_nFileFraction`).
	pub fraction: c_int,
}

/// The fields `CLC_FileMD5Check` declares.
#[doc(alias("CLC_FileMD5Check"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct FileMd5CheckFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::IClientMessageHandler,

	/// The search path ID the file was found under, such as `GAME`
	/// (`m_szPathID`).
	pub path_id: [c_char; MAX_PATH],

	/// The file's path (`m_szFilename`).
	pub file_name: [c_char; MAX_PATH],

	/// The file's MD5 hash (`m_MD5`).
	pub md5: [u8; 16],
}

/// The fields `CLC_ListenEvents` declares.
#[doc(alias("CLC_ListenEvents"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ListenEventsFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::IClientMessageHandler,

	/// A bit per game event ID, the client's wanted events (`m_EventArray`).
	pub events: [u32; MAX_EVENT_NUMBER / u32::BITS as usize],
}

/// A kind of message clients send, by the `IClientMessageHandler` method the
/// engine passes it to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageClass {
	/// `NET_Tick`, for `ProcessTick`.
	#[doc(alias("NET_Tick"))]
	Tick,

	/// `NET_StringCmd`, for `ProcessStringCmd`.
	#[doc(alias("NET_StringCmd"))]
	StringCmd,

	/// `NET_SetConVar`, for `ProcessSetConVar`.
	#[doc(alias("NET_SetConVar"))]
	SetConVar,

	/// `NET_SignonState`, for `ProcessSignonState`.
	#[doc(alias("NET_SignonState"))]
	SignonState,

	/// `CLC_ClientInfo`, for `ProcessClientInfo`.
	#[doc(alias("CLC_ClientInfo"))]
	ClientInfo,

	/// `CLC_Move`, for `ProcessMove`.
	#[doc(alias("CLC_Move"))]
	Move,

	/// `CLC_VoiceData`, for `ProcessVoiceData`.
	#[doc(alias("CLC_VoiceData"))]
	VoiceData,

	/// `CLC_BaselineAck`, for `ProcessBaselineAck`.
	#[doc(alias("CLC_BaselineAck"))]
	BaselineAck,

	/// `CLC_ListenEvents`, for `ProcessListenEvents`.
	#[doc(alias("CLC_ListenEvents"))]
	ListenEvents,

	/// `CLC_RespondCvarValue`, for `ProcessRespondCvarValue`.
	#[doc(alias("CLC_RespondCvarValue"))]
	RespondCvarValue,

	/// `CLC_FileCRCCheck`, for `ProcessFileCRCCheck`.
	#[doc(alias("CLC_FileCRCCheck"))]
	FileCrcCheck,

	/// `CLC_FileMD5Check`, for `ProcessFileMD5Check`.
	#[doc(alias("CLC_FileMD5Check"))]
	FileMd5Check,

	/// `CLC_SaveReplay`, for `ProcessSaveReplay`.
	#[doc(alias("CLC_SaveReplay"))]
	SaveReplay,

	/// `CLC_CmdKeyValues`, for `ProcessCmdKeyValues`.
	#[doc(alias("CLC_CmdKeyValues"))]
	CmdKeyValues,
}

impl MessageClass {
	/// Every class, in the order of their handler methods.
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

	/// The size of the fields the class declares, and so the size of its
	/// objects past their `CNetMessage`.
	const fn fields_size(self) -> usize {
		match self {
			Self::Tick => size_of::<TickFields>(),
			Self::StringCmd => size_of::<StringCmdFields>(),
			Self::SetConVar => size_of::<SetConVarFields>(),
			Self::SignonState => size_of::<SignonStateFields>(),
			Self::ClientInfo => size_of::<ClientInfoFields>(),
			Self::Move => size_of::<MoveFields>(),
			Self::VoiceData => size_of::<VoiceDataFields>(),
			Self::BaselineAck => size_of::<BaselineAckFields>(),
			Self::ListenEvents => size_of::<ListenEventsFields>(),
			Self::RespondCvarValue => size_of::<RespondCvarValueFields>(),
			Self::FileCrcCheck => size_of::<FileCrcCheckFields>(),
			Self::FileMd5Check => size_of::<FileMd5CheckFields>(),
			Self::SaveReplay => size_of::<SaveReplayFields>(),
			Self::CmdKeyValues => size_of::<CmdKeyValuesFields>(),
		}
	}

	/// The vtable slot of the `IClientMessageHandler` method the engine
	/// passes the class's messages to, under the target's C++ ABI.
	pub const fn process_slot(self) -> usize {
		use sys::IClientMessageHandler__bindgen_vtable as Vtable;

		match self {
			Self::Tick => vtable_slot!(Vtable, IClientMessageHandler_ProcessTick),
			Self::StringCmd => vtable_slot!(Vtable, IClientMessageHandler_ProcessStringCmd),
			Self::SetConVar => vtable_slot!(Vtable, IClientMessageHandler_ProcessSetConVar),
			Self::SignonState => vtable_slot!(Vtable, IClientMessageHandler_ProcessSignonState),
			Self::ClientInfo => vtable_slot!(Vtable, IClientMessageHandler_ProcessClientInfo),
			Self::Move => vtable_slot!(Vtable, IClientMessageHandler_ProcessMove),
			Self::VoiceData => vtable_slot!(Vtable, IClientMessageHandler_ProcessVoiceData),
			Self::BaselineAck => vtable_slot!(Vtable, IClientMessageHandler_ProcessBaselineAck),
			Self::ListenEvents => vtable_slot!(Vtable, IClientMessageHandler_ProcessListenEvents),

			Self::RespondCvarValue => {
				vtable_slot!(Vtable, IClientMessageHandler_ProcessRespondCvarValue)
			}

			Self::FileCrcCheck => vtable_slot!(Vtable, IClientMessageHandler_ProcessFileCRCCheck),
			Self::FileMd5Check => vtable_slot!(Vtable, IClientMessageHandler_ProcessFileMD5Check),
			Self::SaveReplay => vtable_slot!(Vtable, IClientMessageHandler_ProcessSaveReplay),
			Self::CmdKeyValues => vtable_slot!(Vtable, IClientMessageHandler_ProcessCmdKeyValues),
		}
	}
}

/// The fields `CLC_Move` declares.
#[doc(alias("CLC_Move"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct MoveFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::IClientMessageHandler,

	/// Commands sent before, repeated in case their packets were lost
	/// (`m_nBackupCommands`).
	pub backup_commands: c_int,

	/// Commands the client has not sent before (`m_nNewCommands`).
	pub new_commands: c_int,

	/// The size of the encoded commands, in bits (`m_nLength`).
	pub length: c_int,

	/// A reader positioned at the encoded commands, over the packet the
	/// message came in (`m_DataIn`).
	pub data_in: BfRead,

	/// The buffer the engine encodes the commands into when it sends the
	/// message (`m_DataOut`).
	pub data_out: BfWrite,
}

/// The fields `CLC_RespondCvarValue` declares.
#[doc(alias("CLC_RespondCvarValue"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct RespondCvarValueFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::IClientMessageHandler,

	/// The cookie of the query answered (`m_iCookie`).
	pub cookie: c_int,

	/// The variable's name, which the engine points at
	/// [`name_buffer`](Self::name_buffer) as it reads the message
	/// (`m_szCvarName`).
	pub name: *const c_char,

	/// The variable's value, which the engine points at
	/// [`value_buffer`](Self::value_buffer) as it reads the message
	/// (`m_szCvarValue`).
	pub value: *const c_char,

	/// The `EQueryCvarValueStatus` of the answer (`m_eStatusCode`):
	/// [`QUERY_CVAR_VALUE_INTACT`] when the value was found,
	/// [`QUERY_CVAR_NOT_FOUND`] when no variable has the name,
	/// [`QUERY_CVAR_NOT_A_CVAR`] when a command has it instead, and
	/// [`QUERY_CVAR_PROTECTED`] when the variable does not allow queries.
	///
	/// [`QUERY_CVAR_VALUE_INTACT`]: crate::interfaces::plugin_helpers::QUERY_CVAR_VALUE_INTACT
	/// [`QUERY_CVAR_NOT_FOUND`]: crate::interfaces::plugin_helpers::QUERY_CVAR_NOT_FOUND
	/// [`QUERY_CVAR_NOT_A_CVAR`]: crate::interfaces::plugin_helpers::QUERY_CVAR_NOT_A_CVAR
	/// [`QUERY_CVAR_PROTECTED`]: crate::interfaces::plugin_helpers::QUERY_CVAR_PROTECTED
	pub status: c_int,

	/// The storage of the variable's name (`m_szCvarNameBuffer`).
	pub name_buffer: [c_char; 256],

	/// The storage of the variable's value (`m_szCvarValueBuffer`).
	pub value_buffer: [c_char; 256],
}

/// The fields `CLC_SaveReplay` declares.
#[doc(alias("CLC_SaveReplay"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct SaveReplayFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::IClientMessageHandler,

	/// The byte of the replay's data to start sending from
	/// (`m_nStartSendByte`).
	pub start_send_byte: c_int,

	/// The name to save the replay under (`m_szFilename`).
	pub file_name: [c_char; MAX_OSPATH],

	/// How long to keep recording after the player's death, in seconds
	/// (`m_flPostDeathRecordTime`).
	pub post_death_record_time: f32,
}

/// The fields `NET_SetConVar` declares.
#[doc(alias("NET_SetConVar"))]
#[derive(Debug)]
#[repr(C)]
pub struct SetConVarFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::INetMessageHandler,

	/// The variables (`m_ConVars`).
	pub convars: sys::CUtlVector<ConVarEntry, sys::CUtlMemory<ConVarEntry>>,
}

/// The fields `NET_SignonState` declares.
#[doc(alias("NET_SignonState"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct SignonStateFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::INetMessageHandler,

	/// The sign-on state the client reached, a `SIGNONSTATE_*` value
	/// (`m_nSignonState`).
	pub signon_state: c_int,

	/// The server's spawn count the state refers to (`m_nSpawnCount`).
	pub spawn_count: c_int,
}

/// The fields `NET_StringCmd` declares.
#[doc(alias("NET_StringCmd"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct StringCmdFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::INetMessageHandler,

	/// The command, which the engine points at
	/// [`command_buffer`](Self::command_buffer) as it reads the message
	/// (`m_szCommand`).
	pub command: *const c_char,

	/// The storage of the command and its arguments (`m_szCommandBuffer`).
	pub command_buffer: [c_char; 1024],
}

/// The fields `NET_Tick` declares.
#[doc(alias("NET_Tick"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct TickFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::INetMessageHandler,

	/// The last server tick the client received (`m_nTick`).
	pub tick: c_int,

	/// The client's frame time, in seconds (`m_flHostFrameTime`).
	pub host_frame_time: f32,

	/// The standard deviation of the client's frame time, in seconds
	/// (`m_flHostFrameTimeStdDeviation`).
	pub host_frame_time_std_deviation: f32,
}

/// The fields `CLC_VoiceData` holds.
#[doc(alias("CLC_VoiceData"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct VoiceDataFields {
	/// The client's handler (`m_pMessageHandler`).
	pub handler: *mut sys::IClientMessageHandler,

	/// The size of the encoded voice, in bits (`m_nLength`).
	pub length: c_int,

	/// A reader positioned at the encoded voice, over the packet the message
	/// came in (`m_DataIn`).
	pub data_in: BfRead,

	/// The buffer the engine encodes the voice into when it sends the
	/// message (`m_DataOut`).
	pub data_out: BfWrite,

	/// The bytes the engine's class holds after its buffers, which this
	/// module does not interpret.
	pub rest: [u8; 8],
}

/// The client whose `IClientMessageHandler` base `handler` points to, or
/// `None` until [`handler_of_client`] has confirmed the engine's clients'
/// layout.
///
/// # Safety
///
/// `handler` must point to the `IClientMessageHandler` base of one of the
/// engine's clients, such as the `this` the engine passes to a method of the
/// vtable of a handler [`handler_of_client`] returned.
pub unsafe fn client_of_handler(
	handler: NonNull<sys::IClientMessageHandler>,
) -> Option<NonNull<sys::IClient>> {
	match HANDLER_OFFSET.load(Ordering::Relaxed) {
		0 => None,

		// SAFETY: The handler is a client's base, which `handler_of_client`
		// confirmed lies `offset` past the client's `IClient` base in every
		// client, all of one class.
		offset => Some(unsafe { handler.byte_sub(offset) }.cast()),
	}
}

/// Whether pointers the engine keeps into a message's own buffers confirm
/// that the fields of its class start at `fields`.
///
/// # Safety
///
/// `fields` must point to [`MessageClass::fields_size`] readable bytes of a
/// live message of `class`.
unsafe fn confirms_fields(class: MessageClass, fields: *const u8) -> bool {
	/// Whether the pointer at `pointer` points to `buffer`.
	///
	/// # Safety
	///
	/// `pointer` must be readable.
	unsafe fn points_to<T>(pointer: *const *const c_char, buffer: *const T) -> bool {
		// SAFETY: As the caller promises.
		unsafe { pointer.read_unaligned() }.cast::<T>() == buffer
	}

	// SAFETY: As the caller promises. Each projection stays within the fields,
	// and the engine points these at the message's own buffers when it reads
	// the message.
	unsafe {
		match class {
			MessageClass::StringCmd => {
				let fields = fields.cast::<StringCmdFields>();

				points_to(
					&raw const (*fields).command,
					&raw const (*fields).command_buffer,
				)
			}

			MessageClass::RespondCvarValue => {
				let fields = fields.cast::<RespondCvarValueFields>();

				points_to(&raw const (*fields).name, &raw const (*fields).name_buffer)
					&& points_to(
						&raw const (*fields).value,
						&raw const (*fields).value_buffer,
					)
			}

			_ => false,
		}
	}
}

/// Copies `NET_SetConVar`'s variables, or returns `None` if its vector is
/// inconsistent.
///
/// # Safety
///
/// `fields` must point to the fields of a live `NET_SetConVar`.
unsafe fn convars(fields: *const SetConVarFields) -> Option<Vec<ConVarEntry>> {
	// SAFETY: As the caller promises, per `CUtlVector`'s generated layout.
	let (memory, capacity, count, elements) = unsafe {
		let vector = &raw const (*fields).convars;

		(
			(&raw const (*vector).m_Memory.m_pMemory).read_unaligned(),
			(&raw const (*vector).m_Memory.m_nAllocationCount).read_unaligned(),
			(&raw const (*vector).m_Size).read_unaligned(),
			(&raw const (*vector).m_pElements).read_unaligned(),
		)
	};

	let count = usize::try_from(count).ok()?;

	if count > MAX_SET_CONVARS
		|| count > usize::try_from(capacity).ok()?
		|| (count > 0 && (memory.is_null() || memory != elements))
	{
		return None;
	}

	Some(
		(0..count)
			// SAFETY: The vector holds `count` elements in its memory, within its
			// capacity, checked above.
			.map(|index| unsafe { memory.add(index).read_unaligned() })
			.collect(),
	)
}

/// Copies a message's fields as a `T`.
///
/// # Safety
///
/// `fields` must point to a `T`'s size of readable bytes. `T` must be one of
/// this module's fields structs, which hold integers, floats, raw pointers,
/// and arrays of them, so that any bytes are a valid `T`.
unsafe fn copy<T>(fields: *const u8) -> T {
	// SAFETY: As the caller promises.
	unsafe { fields.cast::<T>().read_unaligned() }
}

/// Finds the `IClientMessageHandler` base of one of the engine's clients,
/// after checking through run-time type information that the client is a
/// `CGameClient`, with its bases where this module expects. Records that the
/// engine's clients are laid out so, for [`client_of_handler`].
///
/// # Safety
///
/// `client` must point to the `IClient` base of a live client the engine
/// made, a polymorphic subobject whose vtable the engine's module emitted with
/// run-time type information, and which stays loaded for the call.
#[doc(alias("CGameClient"))]
pub unsafe fn handler_of_client(
	client: NonNull<sys::IClient>,
) -> Result<NonNull<sys::IClientMessageHandler>, ClientLayoutError> {
	let client = client.as_ptr().cast_const();

	// SAFETY: As the caller promises.
	let client_offset = unsafe { rtti::subobject_offset(client.cast(), GAME_CLIENT) };

	if client_offset.and_then(|offset| usize::try_from(offset).ok()) != Some(CLIENT_OFFSET) {
		return Err(ClientLayoutError);
	}

	// SAFETY: The complete object is a `CGameClient`, confirmed above with its
	// `IClient` base at `CLIENT_OFFSET`. `CBaseClient` declares
	// `IClientMessageHandler`, a polymorphic base of one vtable pointer, right
	// after `IClient`, so `handler` lies within the object at that base's
	// vtable pointer.
	let handler = unsafe { client.byte_add(CLIENT_TO_HANDLER) };

	// SAFETY: As above, `handler` is the client's `IClientMessageHandler`
	// base, a polymorphic subobject of the same live `CGameClient`, by the
	// engine's declared base order; this run-time type read confirms that
	// placement before it is relied on.
	let handler_offset = unsafe { rtti::subobject_offset(handler.cast(), GAME_CLIENT) };

	if handler_offset.and_then(|offset| usize::try_from(offset).ok())
		!= CLIENT_OFFSET.checked_add(CLIENT_TO_HANDLER)
	{
		return Err(ClientLayoutError);
	}

	let handler = NonNull::new(handler.cast::<sys::IClientMessageHandler>().cast_mut())
		.ok_or(ClientLayoutError)?;

	HANDLER_OFFSET.store(CLIENT_TO_HANDLER, Ordering::Relaxed);
	Ok(handler)
}

/// Whether a message's first field, the handler it is processed by
/// (`m_pMessageHandler`), is `handler`, the one the engine passed it to.
/// Every message class declares that field first, and the engine sets it to
/// the client that registered the message.
///
/// # Safety
///
/// `fields` must point to a pointer's size of readable bytes.
unsafe fn names_handler(fields: *const u8, handler: NonNull<sys::IClientMessageHandler>) -> bool {
	// SAFETY: As the caller promises.
	let first = unsafe { fields.cast::<*const ()>().read_unaligned() };

	first == handler.as_ptr().cast_const().cast()
}

/// Copies the fields of a message a client sent, after checking that the
/// engine's message is laid out as this module expects. Returns `None` if it
/// is not, or the fields are inconsistent.
///
/// The fields follow the engine's `CNetMessage`, whose size is learned once
/// per process, from the first message whose fields confirm where they start:
/// one whose first field is `handler`, the client's handler the engine passed
/// it to, or a `NET_StringCmd` or `CLC_RespondCvarValue` whose pointers into
/// its own buffers do. Until then, no message is read.
///
/// # Safety
///
/// `message` must point to a live message of `class` that the engine passed
/// to `handler`'s method for `class`, and that nothing changes during the
/// call, in a module that stays loaded for the call.
pub unsafe fn read_message(
	class: MessageClass,
	handler: NonNull<sys::IClientMessageHandler>,
	message: NonNull<sys::INetMessage>,
) -> Option<ClientMessage> {
	let this = message.as_ptr().cast_const();

	// SAFETY: As the caller promises.
	let size = unsafe { vcall!(this => INetMessage_GetSize()) };
	let base = size.checked_sub(class.fields_size())?;

	// Every class's fields start with a pointer, so must be aligned for one.
	if !(SMALLEST_MESSAGE_BASE..=LARGEST_MESSAGE_BASE).contains(&base)
		|| !base.is_multiple_of(align_of::<*const ()>())
	{
		return None;
	}

	// SAFETY: The engine's object is `size` bytes, so its fields lie within
	// it, past `base`.
	let fields = unsafe { this.byte_add(base) }.cast::<u8>();

	match MESSAGE_BASE.load(Ordering::Relaxed) {
		// SAFETY: As above. Every class's fields are at least a pointer.
		0 if unsafe { names_handler(fields, handler) || confirms_fields(class, fields) } => {
			MESSAGE_BASE.store(base, Ordering::Relaxed);
		}

		known if known == base => {}
		_ => return None,
	}

	// SAFETY: As above, and the message's base confirms where the fields
	// start. A message the engine is processing has its readers positioned
	// over the packet it came in, which lasts for the call.
	unsafe {
		Some(match class {
			MessageClass::Tick => ClientMessage::Tick(copy(fields)),
			MessageClass::StringCmd => ClientMessage::StringCmd(Box::new(copy(fields))),
			MessageClass::SetConVar => ClientMessage::SetConVar(convars(fields.cast())?),
			MessageClass::SignonState => ClientMessage::SignonState(copy(fields)),
			MessageClass::ClientInfo => ClientMessage::ClientInfo(copy(fields)),

			MessageClass::Move => {
				let fields = copy::<MoveFields>(fields);

				ClientMessage::Move {
					data: fields.data_in.unread_bits(fields.length)?,
					fields,
				}
			}

			MessageClass::VoiceData => {
				let fields = copy::<VoiceDataFields>(fields);

				ClientMessage::VoiceData {
					data: fields.data_in.unread_bits(fields.length)?,
					fields,
				}
			}

			MessageClass::BaselineAck => ClientMessage::BaselineAck(copy(fields)),
			MessageClass::ListenEvents => ClientMessage::ListenEvents(copy(fields)),
			MessageClass::RespondCvarValue => ClientMessage::RespondCvarValue(copy(fields)),
			MessageClass::FileCrcCheck => ClientMessage::FileCrcCheck(copy(fields)),
			MessageClass::FileMd5Check => ClientMessage::FileMd5Check(copy(fields)),
			MessageClass::SaveReplay => ClientMessage::SaveReplay(copy(fields)),
			MessageClass::CmdKeyValues => ClientMessage::CmdKeyValues(copy(fields)),
		})
	}
}
