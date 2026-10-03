//! Values from `public/const.h` about the clients the engine hosts, which the
//! generated bindings omit.

use std::ffi::c_int;

/// The most clients the engine can host at once.
///
/// This is `ABSOLUTE_PLAYER_LIMIT` from `public/const.h`. The player of each
/// client uses the edict one past the client's slot, so no player's edict
/// index exceeds this limit.
pub const ABSOLUTE_PLAYER_LIMIT: c_int = 255;
