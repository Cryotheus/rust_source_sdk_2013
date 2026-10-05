//! Hand-written ABI of `IServerGameClients` that the generated bindings do not
//! describe.

use crate::vtable_slot;
use std::ffi::{CStr, c_char, c_int};

/// `void IServerGameClients::ClientCommand(edict_t *, const CCommand &)`,
/// which the engine calls to run a command a client sent, and plugins hook to
/// run their own client commands.
#[doc(alias("ClientCommand"))]
pub type ClientCommandFn = unsafe extern "C" fn(
	this: *mut sys::IServerGameClients,
	edict: *mut sys::edict_t,
	args: *const sys::CCommand,
);

/// `bool IServerGameClients::ClientConnect(edict_t *, const char *name, const
/// char *address, char *reject, int maxrejectlen)`, which the engine calls as
/// a remote client connects, before it joins, and which refuses the client by
/// returning false after writing the reason the client is shown to `reject`,
/// a buffer of `maxrejectlen` bytes. Fake clients, such as bots, join without
/// it.
#[doc(alias("ClientConnect"))]
pub type ClientConnectFn = unsafe extern "C" fn(
	this: *mut sys::IServerGameClients,
	edict: *mut sys::edict_t,
	name: *const c_char,
	address: *const c_char,
	reject: *mut c_char,
	reject_capacity: c_int,
) -> bool;

const _: () = assert!(
	vtable_slot!(
		sys::IServerGameClients__bindgen_vtable,
		IServerGameClients_ClientCommand
	) == CLIENT_COMMAND_SLOT
);

const _: () = assert!(
	vtable_slot!(
		sys::IServerGameClients__bindgen_vtable,
		IServerGameClients_ClientConnect
	) == CLIENT_CONNECT_SLOT
);

// The generated binding has this signature.
const _: fn(&sys::IServerGameClients__bindgen_vtable) -> ClientCommandFn =
	|vtable| vtable.IServerGameClients_ClientCommand;

// The generated binding has this signature.
const _: fn(&sys::IServerGameClients__bindgen_vtable) -> ClientConnectFn =
	|vtable| vtable.IServerGameClients_ClientConnect;

/// The vtable slot of `IServerGameClients::ClientCommand`.
///
/// The interface declares no virtual destructor, so the slot is the same under
/// the MSVC and Itanium ABIs.
pub const CLIENT_COMMAND_SLOT: usize = 5;

/// The vtable slot of `IServerGameClients::ClientConnect`, as for
/// [`CLIENT_COMMAND_SLOT`].
pub const CLIENT_CONNECT_SLOT: usize = 1;

/// The version string `IServerGameClients` is exported and requested under.
///
/// This is `INTERFACEVERSION_SERVERGAMECLIENTS` from `public/eiface.h`.
#[doc(alias("INTERFACEVERSION_SERVERGAMECLIENTS"))]
pub const VERSION: &CStr = c"ServerGameClients005";
