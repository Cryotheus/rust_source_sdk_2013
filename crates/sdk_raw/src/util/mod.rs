//! Checked image readers, signature scans, and primary C++ vtable discovery.
//!
//! Snapshots own their bytes; they never borrow mutable engine memory. Returned
//! addresses describe the snapshot and can become stale when a module unloads.

mod bytes;
pub mod elf;
mod image;
pub mod pe;
mod rtti;
mod signature;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;

pub use crate::sig;
pub use bytes::{relative, u16_at, u32_at, word_at};
pub use image::{Image, Section};
pub use platform::{MemoryReader, Module, is_executable};
pub use signature::{SignaturePattern, pattern};

/// Upper bound on an individual snapshot or on-disk image allocation.
pub const MAX_IMAGE_BYTES: usize = 0x40000000;

/// Failure to read or validate an executable image.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("unsupported or malformed executable image")]
	InvalidImage,

	#[error("could not inspect executable image: {0}")]
	Io(#[from] std::io::Error),
}
