//! Hand-written ABI of `IServerGameEnts` that the generated bindings do not
//! describe: the version string it is requested by, from `public/eiface.h`.

use std::ffi::CStr;

/// The version string `IServerGameEnts` is exported and requested under.
///
/// This is `INTERFACEVERSION_SERVERGAMEENTS` from `public/eiface.h`.
#[doc(alias = "INTERFACEVERSION_SERVERGAMEENTS")]
pub const VERSION: &CStr = c"ServerGameEnts001";
