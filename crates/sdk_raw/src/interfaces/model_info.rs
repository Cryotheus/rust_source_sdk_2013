//! Hand-written ABI of `IVModelInfo` that the generated bindings do not
//! describe: the version string it is requested by, from
//! `public/engine/ivmodelinfo.h`.

use std::ffi::CStr;

/// The version string `IVModelInfo` is exported and requested under.
///
/// This is `VMODELINFO_SERVER_INTERFACE_VERSION` from
/// `public/engine/ivmodelinfo.h`.
#[doc(alias = "VMODELINFO_SERVER_INTERFACE_VERSION")]
pub const VERSION: &CStr = c"VModelInfoServer004";
