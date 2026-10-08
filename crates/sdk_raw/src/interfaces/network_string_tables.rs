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

/// The most bytes of user data a string of a table can carry: what fits the
/// 14 bits the engine sends each string's user data size in, as its
/// `MAX_USERDATA_BITS` (`engine/networkstringtable.h`, which the SDK does not
/// ship) sets them.
#[doc(alias("MAX_USERDATA_BITS", "MAX_USERDATA_SIZE"))]
pub const MAX_USER_DATA_LEN: usize = (1 << 14) - 1;

/// The `length` to pass `INetworkStringTable::AddString` when adding a string
/// without user data: its default in `public/networkstringtabledefs.h`.
/// `length` is the size of the user data, not of the string.
#[doc(alias("AddString"))]
pub const UNKNOWN_STRING_LENGTH: c_int = -1;

/// The version string `INetworkStringTableContainer` is exported and requested
/// under.
///
/// This is `INTERFACENAME_NETWORKSTRINGTABLESERVER` from
/// `public/networkstringtabledefs.h`.
#[doc(alias("INTERFACENAME_NETWORKSTRINGTABLESERVER"))]
pub const VERSION: &CStr = c"VEngineServerStringTable001";
