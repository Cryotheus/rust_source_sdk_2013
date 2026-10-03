//! Hand-written ABI of `IPlayerInfoManager` that the generated bindings do not
//! describe: the version string it is requested by, from
//! `public/game/server/iplayerinfo.h`.

use std::ffi::CStr;

/// The version string `IPlayerInfoManager` is exported and requested under.
///
/// This is `INTERFACEVERSION_PLAYERINFOMANAGER` from
/// `public/game/server/iplayerinfo.h`.
#[doc(alias = "INTERFACEVERSION_PLAYERINFOMANAGER")]
pub const VERSION: &CStr = c"PlayerInfoManager002";
