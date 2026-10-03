//! Hand-written ABI of the engine's network string tables that the generated
//! bindings do not describe.

use std::ffi::{CStr, c_int};

/// The index a string table reports for a string it does not hold, or refused
/// to add: `(unsigned short)-1`.
///
/// This is `INVALID_STRING_INDEX` from `public/networkstringtabledefs.h`.
/// `INetworkStringTable::AddString` and `FindStringIndex` return it widened to
/// `int`, as 65535.
pub const INVALID_STRING_INDEX: u16 = u16::MAX;

/// The `length` to pass `INetworkStringTable::AddString` when adding a string
/// without user data: its default in `public/networkstringtabledefs.h`.
/// `length` is the size of the user data, not of the string.
#[doc(alias = "AddString")]
pub const UNKNOWN_STRING_LENGTH: c_int = -1;

/// The version string `INetworkStringTableContainer` is exported and requested
/// under.
///
/// This is `INTERFACENAME_NETWORKSTRINGTABLESERVER` from
/// `public/networkstringtabledefs.h`.
#[doc(alias = "INTERFACENAME_NETWORKSTRINGTABLESERVER")]
pub const VERSION: &CStr = c"VEngineServerStringTable001";
