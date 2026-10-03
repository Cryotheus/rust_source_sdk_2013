//! Hand-written ABI of `IServerGameClients` that the generated bindings do not
//! describe.

use crate::vtable_slot;
use std::ffi::CStr;

/// `void IServerGameClients::ClientCommand(edict_t *, const CCommand &)`,
/// which the engine calls to run a command a client sent, and plugins hook to
/// run their own client commands.
#[doc(alias("ClientCommand"))]
pub type ClientCommandFn = unsafe extern "C" fn(
	this: *mut sys::IServerGameClients,
	edict: *mut sys::edict_t,
	args: *const sys::CCommand,
);

const _: () = assert!(
	vtable_slot!(
		sys::IServerGameClients__bindgen_vtable,
		IServerGameClients_ClientCommand
	) == CLIENT_COMMAND_SLOT
);

// The generated binding has this signature.
const _: fn(&sys::IServerGameClients__bindgen_vtable) -> ClientCommandFn =
	|vtable| vtable.IServerGameClients_ClientCommand;

/// The vtable slot of `IServerGameClients::ClientCommand`.
///
/// The interface declares no virtual destructor, so the slot is the same under
/// the MSVC and Itanium ABIs.
pub const CLIENT_COMMAND_SLOT: usize = 5;

/// The version string `IServerGameClients` is exported and requested under.
///
/// This is `INTERFACEVERSION_SERVERGAMECLIENTS` from `public/eiface.h`.
#[doc(alias("INTERFACEVERSION_SERVERGAMECLIENTS"))]
pub const VERSION: &CStr = c"ServerGameClients005";
