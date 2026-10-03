//! Console commands and variables Rust implements for the engine: objects
//! laid out as tier1's `ConCommand` and `ConVar`, with their vtables,
//! run-time type information, and C++ thunks, and the header values and
//! argument handling around them.
//!
//! # Objects and their owners
//!
//! A [`ConCommandObject`] or [`ConVarObject`] is the first field of its
//! owner's `#[repr(C)]` container, which holds what the owner keeps alongside
//! it. The owner hands the engine a pointer to the whole container, cast to
//! the object, through [`ConCommandObject::as_base`] or
//! [`ConVarObject::as_base`]. The engine passes that pointer back to every
//! slot of the object's vtables, and the slots that run the owner's code pass
//! it on to the hooks the object was created with, so the hooks can reach the
//! rest of the container.
//!
//! Each object kind has one vtable, shared by every object of the kind, so
//! the hooks are what tell objects apart: an owner gives its objects hooks in
//! a `static` of its own, whose address identifies its objects.
//!
//! The `FCVAR_*` flags are those of `public/tier1/iconvar.h`, which the
//! engine stores in `ConCommandBase::m_nFlags`.

mod args;
mod object;
mod variable;

use std::ffi::c_int;
use std::ptr::NonNull;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub use args::tokenized;

pub use args::{CommandLine, MalformedCommand};
pub use object::{ConCommandHooks, ConCommandObject};

pub use variable::{
	ConVarHooks, ConVarObject, ICONVAR_OFFSET, ReplacedString, format_float, format_int,
	parse_float, parse_int,
};

/// The names the engine runs for clients itself, through `Dispatch` as
/// though the server had run them, before the game or any hook sees them: the
/// list `CGameClient::ExecuteStringCommand` checks in TF2's 64-bit engine.
/// The `rpt` names are not registered, so the registry cannot report them
/// taken.
pub const ENGINE_CLIENT_COMMANDS: [&[u8]; 12] = [
	b"status",
	b"pause",
	b"setpause",
	b"unpause",
	b"ping",
	b"rpt",
	b"rpt_server_enable",
	b"rpt_client_enable",
	b"rpt_connect",
	b"rpt_password",
	b"rpt_screenshot",
	b"rpt_download_log",
];

/// A variable the engine saves when it writes its configuration.
pub const FCVAR_ARCHIVE: c_int = 1 << 7;

/// Runnable only while `sv_cheats` is set.
pub const FCVAR_CHEAT: c_int = 1 << 14;

/// A client command or variable the client library's
/// `IVEngineClient::ClientCmd` may run.
pub const FCVAR_CLIENTCMD_CAN_EXECUTE: c_int = 1 << 30;

/// Declared by the client library.
pub const FCVAR_CLIENTDLL: c_int = 1 << 3;

/// A variable recorded when a demo recording starts.
pub const FCVAR_DEMO: c_int = 1 << 16;

/// Hidden from the console in released builds of the engine.
pub const FCVAR_DEVELOPMENTONLY: c_int = 1 << 1;

/// Left out of demo recordings.
pub const FCVAR_DONTRECORD: c_int = 1 << 17;

/// Declared by the game server library. The engine dispatches a client's
/// invocation of a command carrying it directly, without saying which client
/// ran it, so a [`ConCommandObject`] never carries or reports it.
pub const FCVAR_GAMEDLL: c_int = 1 << 2;

/// Left out of `find`, `cvarlist`, and completion.
pub const FCVAR_HIDDEN: c_int = 1 << 4;

/// A variable whose string is never updated from its value.
pub const FCVAR_NEVER_AS_STRING: c_int = 1 << 12;

/// No flags.
pub const FCVAR_NONE: c_int = 0;

/// A client variable clients cannot change while connected to a server.
pub const FCVAR_NOT_CONNECTED: c_int = 1 << 22;

/// Changes to a variable are announced to players and written to the server
/// log.
pub const FCVAR_NOTIFY: c_int = 1 << 8;

/// A variable whose string may only contain printable characters.
pub const FCVAR_PRINTABLEONLY: c_int = 1 << 10;

/// A server variable whose value is withheld from queries of the server's
/// rules.
pub const FCVAR_PROTECTED: c_int = 1 << 5;

/// A variable whose server value is sent to every client.
pub const FCVAR_REPLICATED: c_int = 1 << 13;

/// A client command clients run when the server sends it to them.
pub const FCVAR_SERVER_CAN_EXECUTE: c_int = 1 << 28;

/// A client variable whose value clients refuse to report to the server.
pub const FCVAR_SERVER_CANNOT_QUERY: c_int = 1 << 29;

/// Meant for single-player games.
pub const FCVAR_SPONLY: c_int = 1 << 6;

/// Changes to a variable are not written to the server log.
pub const FCVAR_UNLOGGED: c_int = 1 << 11;

/// A client variable whose value clients send to the server.
pub const FCVAR_USERINFO: c_int = 1 << 9;

/// Whether the engine has marked a command or variable registered.
///
/// # Safety
///
/// `base` must point to a live `ConCommandBase`.
#[doc(alias = "IsRegistered")]
pub unsafe fn is_registered(base: NonNull<sys::ConCommandBase>) -> bool {
	// SAFETY: The caller guarantees the object is live. The field is read
	// without forming a reference, since C++ writes it too.
	unsafe { (&raw const (*base.as_ptr()).m_bRegistered).read() }
}
