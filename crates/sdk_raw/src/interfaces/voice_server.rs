//! Hand-written ABI of `IVoiceServer` that the generated bindings do not
//! describe: the version string it is requested by, from
//! `public/ivoiceserver.h`.

use std::ffi::CStr;

/// The version string `IVoiceServer` is exported and requested under.
///
/// This is `INTERFACEVERSION_VOICESERVER` from `public/ivoiceserver.h`.
#[doc(alias("INTERFACEVERSION_VOICESERVER"))]
pub const VERSION: &CStr = c"VoiceServer002";
