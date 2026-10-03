//! Generic low-level utilities: checked image readers, signature scans,
//! run-time type information, C strings, vtable calls, and code patching.
//!
//! Snapshots own their bytes; they never borrow mutable engine memory. Returned
//! addresses describe the snapshot and can become stale when a module unloads.

mod bytes;
pub mod cstr;
pub mod elf;
mod image;
pub mod patch;
pub mod pe;
pub mod printf;
pub mod rtti;
pub mod signature;
pub mod vtable;

#[cfg(any(test, feature = "test-support"))]
#[doc(hidden)]
pub mod mock;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;

pub use crate::sig;
pub use bytes::{relative, u16_at, u32_at, word_at};
pub use image::{Image, Section};
pub use platform::{MemoryReader, Module, is_executable, loaded_symbol};
pub use signature::{SignaturePattern, find_all, pattern};

/// Upper bound on an individual snapshot or on-disk image allocation.
pub const MAX_IMAGE_BYTES: usize = 0x40000000;

/// Failure to read or validate an executable image.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	/// The image is not a supported 64-bit module, or its headers or regions
	/// are inconsistent.
	#[error("unsupported or malformed executable image")]
	InvalidImage,

	/// The OS refused to describe or read the module.
	#[error("could not inspect executable image: {0}")]
	Io(#[from] std::io::Error),
}
