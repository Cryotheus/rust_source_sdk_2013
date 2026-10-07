//! Hand-written ABI of `IVEngineServer` that the generated bindings do not
//! describe.
//!
//! `IVEngineServer` declares no virtual destructor, so its methods occupy the
//! same slots under both the MSVC and Itanium ABIs. The slots of `ChangeLevel`
//! (0), `GetPlayerUserId` (15), `PEntityOfEntIndex` (19), `ServerCommand` (36),
//! `ServerExecute` (37), `EntityMessageBegin` (42), `UserMessageBegin` (43),
//! `MessageEnd` (44), `ClientPrintf` (45), `LockNetworkStringTables` (53),
//! `CreateFakeClient` (54) and `CreateFakeClientEx` (116), counted from
//! `public/eiface.h`, are checked against the generated vtable, so a
//! regenerated binding cannot silently dispatch to another method.

use crate::vtable_slot;
use std::ffi::{CStr, c_char, c_int};

const _: () = {
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_ChangeLevel
		) == 0
	);
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_GetPlayerUserId
		) == 15
	);
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_PEntityOfEntIndex
		) == 19
	);
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_ServerCommand
		) == 36
	);
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_ServerExecute
		) == 37
	);
	assert!(ENTITY_MESSAGE_BEGIN_SLOT == 42);
	assert!(USER_MESSAGE_BEGIN_SLOT == 43);
	assert!(MESSAGE_END_SLOT == 44);
	assert!(CLIENT_PRINTF_SLOT == 45);
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_LockNetworkStringTables
		) == 53
	);
	assert!(CREATE_FAKE_CLIENT_SLOT == 54);
	assert!(CREATE_FAKE_CLIENT_EX_SLOT == 116);
};

// The generated methods take and return what the aliases below do.
const _: fn(&sys::IVEngineServer__bindgen_vtable) -> CreateFakeClientFn =
	|vtable| vtable.IVEngineServer_CreateFakeClient;
const _: fn(&sys::IVEngineServer__bindgen_vtable) -> CreateFakeClientExFn =
	|vtable| vtable.IVEngineServer_CreateFakeClientEx;
const _: fn(&sys::IVEngineServer__bindgen_vtable) -> EntityMessageBeginFn =
	|vtable| vtable.IVEngineServer_EntityMessageBegin;
const _: fn(&sys::IVEngineServer__bindgen_vtable) -> UserMessageBeginFn =
	|vtable| vtable.IVEngineServer_UserMessageBegin;
const _: fn(&sys::IVEngineServer__bindgen_vtable) -> MessageEndFn =
	|vtable| vtable.IVEngineServer_MessageEnd;
const _: fn(&sys::IVEngineServer__bindgen_vtable) -> ClientPrintfFn =
	|vtable| vtable.IVEngineServer_ClientPrintf;
const _: fn(&sys::IVEngineServer__bindgen_vtable) -> ServerCommandFn =
	|vtable| vtable.IVEngineServer_ServerCommand;

/// The slot of `IVEngineServer::CreateFakeClient`, from the generated binding.
#[doc(alias("CreateFakeClient"))]
pub const CREATE_FAKE_CLIENT_SLOT: usize = vtable_slot!(
	sys::IVEngineServer__bindgen_vtable,
	IVEngineServer_CreateFakeClient
);

/// The slot of `IVEngineServer::CreateFakeClientEx`, from the generated
/// binding.
#[doc(alias("CreateFakeClientEx"))]
pub const CREATE_FAKE_CLIENT_EX_SLOT: usize = vtable_slot!(
	sys::IVEngineServer__bindgen_vtable,
	IVEngineServer_CreateFakeClientEx
);

/// The slot of `IVEngineServer::ClientPrintf`, from the generated binding.
#[doc(alias("ClientPrintf"))]
pub const CLIENT_PRINTF_SLOT: usize = vtable_slot!(
	sys::IVEngineServer__bindgen_vtable,
	IVEngineServer_ClientPrintf
);

/// The slot of `IVEngineServer::EntityMessageBegin`, from the generated
/// binding.
#[doc(alias("EntityMessageBegin"))]
pub const ENTITY_MESSAGE_BEGIN_SLOT: usize = vtable_slot!(
	sys::IVEngineServer__bindgen_vtable,
	IVEngineServer_EntityMessageBegin
);

/// The slot of `IVEngineServer::MessageEnd`, from the generated binding.
#[doc(alias("MessageEnd"))]
pub const MESSAGE_END_SLOT: usize = vtable_slot!(
	sys::IVEngineServer__bindgen_vtable,
	IVEngineServer_MessageEnd
);

/// The slot of `IVEngineServer::UserMessageBegin`, from the generated binding.
#[doc(alias("UserMessageBegin"))]
pub const USER_MESSAGE_BEGIN_SLOT: usize = vtable_slot!(
	sys::IVEngineServer__bindgen_vtable,
	IVEngineServer_UserMessageBegin
);

/// The slot of `IVEngineServer::ServerCommand`, from the generated binding.
#[doc(alias("ServerCommand"))]
pub const SERVER_COMMAND_SLOT: usize = vtable_slot!(
	sys::IVEngineServer__bindgen_vtable,
	IVEngineServer_ServerCommand
);

/// `IVEngineServer::ClientPrintf`: prints `message` to the console of the
/// client owning `client`, as is rather than as a format.
#[doc(alias("ClientPrintf"))]
pub type ClientPrintfFn = unsafe extern "C" fn(
	this: *mut sys::IVEngineServer,
	client: *mut sys::edict_t,
	message: *const c_char,
);

/// `IVEngineServer::EntityMessageBegin`: begins a message to the client-side
/// object of the entity at `entity_index`, whose server class is `class`, sent
/// reliably if `reliable` is set, and returns the buffer to write its payload
/// into, until [`MessageEndFn`] sends it.
#[doc(alias("EntityMessageBegin"))]
pub type EntityMessageBeginFn = unsafe extern "C" fn(
	this: *mut sys::IVEngineServer,
	entity_index: c_int,
	class: *mut sys::ServerClass,
	reliable: bool,
) -> *mut sys::bf_write;

/// `IVEngineServer::MessageEnd`: sends the message that
/// [`UserMessageBeginFn`] or [`EntityMessageBeginFn`] began.
#[doc(alias("MessageEnd"))]
pub type MessageEndFn = unsafe extern "C" fn(this: *mut sys::IVEngineServer);

/// `IVEngineServer::UserMessageBegin`: begins a user message of the type
/// `message_type`, the index the game registered it at, to the clients
/// `filter` lists, and returns the buffer to write its payload into, until
/// [`MessageEndFn`] sends it.
///
/// One message, user or entity, can be open at a time, and `filter` must
/// outlive it.
#[doc(alias("UserMessageBegin"))]
pub type UserMessageBeginFn = unsafe extern "C" fn(
	this: *mut sys::IVEngineServer,
	filter: *mut sys::IRecipientFilter,
	message_type: c_int,
) -> *mut sys::bf_write;

/// `IVEngineServer::ServerCommand`: appends `command`, which must end in a line
/// break, to the server's command buffer, which the main thread runs on its
/// next frame or [`ServerExecute`](sys::IVEngineServer__bindgen_vtable). The
/// engine locks the buffer while it appends.
#[doc(alias("ServerCommand"))]
pub type ServerCommandFn =
	unsafe extern "C" fn(this: *mut sys::IVEngineServer, command: *const c_char);

/// `IVEngineServer::CreateFakeClient`: connects a fake client named `name`,
/// returning its edict, or null if the server has no free slot.
#[doc(alias("CreateFakeClient"))]
pub type CreateFakeClientFn =
	unsafe extern "C" fn(this: *mut sys::IVEngineServer, name: *const c_char) -> *mut sys::edict_t;

/// `IVEngineServer::CreateFakeClientEx`: as [`CreateFakeClientFn`], with the
/// engine's `bReportFakeClient` as `report`, which the header defaults to true.
/// The game passes false only for Mann vs. Machine's robots, through
/// `NextBotCreatePlayerBot`.
///
/// The engine is not public. TF2's sets its choice for new fake clients,
/// calls `CreateFakeClient` through the vtable, and sets the choice back to
/// true. It tells Steam of an unreported client as it does of SourceTV: as
/// neither a bot nor a slot, and not as a player.
#[doc(alias("CreateFakeClientEx"))]
pub type CreateFakeClientExFn = unsafe extern "C" fn(
	this: *mut sys::IVEngineServer,
	name: *const c_char,
	report: bool,
) -> *mut sys::edict_t;

/// The version string `IVEngineServer` is exported and requested under.
///
/// This is `INTERFACEVERSION_VENGINESERVER` from `public/eiface.h`.
#[doc(alias("INTERFACEVERSION_VENGINESERVER"))]
pub const VERSION: &CStr = c"VEngineServer023";
