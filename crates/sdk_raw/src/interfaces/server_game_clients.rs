//! Hand-written ABI of `IServerGameClients` that the generated bindings do not
//! describe.

use crate::vtable_slot;
use std::ffi::{CStr, c_char};

/// `void IServerGameClients::ClientActive(edict_t *, bool bLoadGame)`, which
/// the engine calls once a client finished connecting, for its player to
/// enter the game. TF2's spawns the player there, the first time with its
/// `InitialSpawn`.
#[doc(alias("ClientActive"))]
pub type ClientActiveFn = unsafe extern "C" fn(
	this: *mut sys::IServerGameClients,
	edict: *mut sys::edict_t,
	load_game: bool,
);

/// `void IServerGameClients::ClientCommand(edict_t *, const CCommand &)`,
/// which the engine calls to run a command a client sent, and plugins hook to
/// run their own client commands.
#[doc(alias("ClientCommand"))]
pub type ClientCommandFn = unsafe extern "C" fn(
	this: *mut sys::IServerGameClients,
	edict: *mut sys::edict_t,
	args: *const sys::CCommand,
);

/// `void IServerGameClients::ClientPutInServer(edict_t *, const char *name)`,
/// which the engine calls as a client's player is put in the server, before
/// it [enters the game](ClientActiveFn). The game creates the client's player
/// entity there, with its final class, and does not spawn it yet.
#[doc(alias("ClientPutInServer"))]
pub type ClientPutInServerFn = unsafe extern "C" fn(
	this: *mut sys::IServerGameClients,
	edict: *mut sys::edict_t,
	name: *const c_char,
);

const _: () = {
	use sys::IServerGameClients__bindgen_vtable as Vtable;

	assert!(vtable_slot!(Vtable, IServerGameClients_ClientActive) == CLIENT_ACTIVE_SLOT);
	assert!(vtable_slot!(Vtable, IServerGameClients_ClientCommand) == CLIENT_COMMAND_SLOT);
	assert!(
		vtable_slot!(Vtable, IServerGameClients_ClientPutInServer) == CLIENT_PUT_IN_SERVER_SLOT
	);
};

// The generated bindings have these signatures.
const _: fn(&sys::IServerGameClients__bindgen_vtable) -> ClientActiveFn =
	|vtable| vtable.IServerGameClients_ClientActive;

const _: fn(&sys::IServerGameClients__bindgen_vtable) -> ClientCommandFn =
	|vtable| vtable.IServerGameClients_ClientCommand;

const _: fn(&sys::IServerGameClients__bindgen_vtable) -> ClientPutInServerFn =
	|vtable| vtable.IServerGameClients_ClientPutInServer;

/// The vtable slot of `IServerGameClients::ClientActive`, the same under both
/// ABIs as [`CLIENT_COMMAND_SLOT`].
pub const CLIENT_ACTIVE_SLOT: usize = 2;

/// The vtable slot of `IServerGameClients::ClientCommand`.
///
/// The interface declares no virtual destructor, so the slot is the same under
/// the MSVC and Itanium ABIs.
pub const CLIENT_COMMAND_SLOT: usize = 5;

/// The vtable slot of `IServerGameClients::ClientPutInServer`, the same under
/// both ABIs as [`CLIENT_COMMAND_SLOT`].
pub const CLIENT_PUT_IN_SERVER_SLOT: usize = 4;

/// The version string `IServerGameClients` is exported and requested under.
///
/// This is `INTERFACEVERSION_SERVERGAMECLIENTS` from `public/eiface.h`.
#[doc(alias("INTERFACEVERSION_SERVERGAMECLIENTS"))]
pub const VERSION: &CStr = c"ServerGameClients005";
