//! Hand-written ABI of `IBotManager` that the generated bindings do not
//! describe: the version string it is requested by, from
//! `public/game/server/iplayerinfo.h`.

use std::ffi::CStr;

/// The version string `IBotManager` is exported and requested under.
///
/// This is `INTERFACEVERSION_PLAYERBOTMANAGER` from
/// `public/game/server/iplayerinfo.h`.
#[doc(alias = "INTERFACEVERSION_PLAYERBOTMANAGER")]
pub const VERSION: &CStr = c"BotManager001";
