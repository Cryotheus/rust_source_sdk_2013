//! Hand-written ABI of `IEngineTrace` that the generated bindings do not
//! describe: the version string it is requested by, from
//! `public/engine/IEngineTrace.h`.

use std::ffi::CStr;

/// The version string `IEngineTrace` is exported and requested under.
///
/// This is `INTERFACEVERSION_ENGINETRACE_SERVER` from
/// `public/engine/IEngineTrace.h`.
#[doc(alias("INTERFACEVERSION_ENGINETRACE_SERVER"))]
pub const VERSION: &CStr = c"EngineTraceServer003";
