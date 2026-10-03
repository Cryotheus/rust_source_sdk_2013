//! Hand-written ABI of the engine's network string tables that the generated
//! bindings do not describe.

/// The index a string table reports for a string it does not hold, or refused
/// to add: `(unsigned short)-1`.
///
/// This is `INVALID_STRING_INDEX` from `public/networkstringtabledefs.h`.
/// `INetworkStringTable::AddString` and `FindStringIndex` return it widened to
/// `int`, as 65535.
pub const INVALID_STRING_INDEX: u16 = u16::MAX;
