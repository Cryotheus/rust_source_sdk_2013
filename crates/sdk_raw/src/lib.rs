//! Low-level runtime support for inspecting Source engine modules.
//!
//! Utilities operate on owned byte snapshots. Resolving an address does not
//! establish its C++ type, ABI, lifetime, or suitability for a native call.

#[cfg(not(any(
	all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"),
	all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")
)))]
compile_error!("source_sdk_2013_raw requires Windows x64 MSVC or Linux x64 GNU");

pub mod util;
