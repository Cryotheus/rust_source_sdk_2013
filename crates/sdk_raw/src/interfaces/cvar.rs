//! Hand-written ABI of `ICvar` that the generated bindings do not
//! describe: the version string it is requested by, from `public/icvar.h`.

use std::ffi::CStr;

/// The version string `ICvar` is exported and requested under.
///
/// This is `CVAR_INTERFACE_VERSION` from `public/icvar.h`.
#[doc(alias = "CVAR_INTERFACE_VERSION")]
pub const VERSION: &CStr = c"VEngineCvar004";
