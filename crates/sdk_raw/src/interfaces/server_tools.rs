//! Hand-written ABI of `IServerTools` that the generated bindings do not
//! describe: the version string it is requested by, from
//! `public/toolframework/itoolentity.h`.

use std::ffi::CStr;

/// The version string `IServerTools` is exported and requested under.
///
/// This is `VSERVERTOOLS_INTERFACE_VERSION` from
/// `public/toolframework/itoolentity.h`.
#[doc(alias = "VSERVERTOOLS_INTERFACE_VERSION")]
pub const VERSION: &CStr = c"VSERVERTOOLS003";
