//! Generic low-level utilities: checked image readers, signature scans, a
//! cache of module resolutions, run-time type information, C strings, vtable
//! calls, and code patching.
//!
//! Snapshots own their bytes; they never borrow mutable engine memory. Returned
//! addresses describe the snapshot and can become stale when a module unloads.

mod bytes;
pub mod cstr;
pub mod elf;
mod image;
mod module_cache;
pub mod patch;
pub mod pe;
pub mod printf;
pub mod rtti;
pub mod signature;
pub mod vtable;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;

use std::path::{Path, PathBuf};

pub use crate::sig;
pub use bytes::{relative, u16_at, u32_at, word_at};
pub use image::{Image, Section};
pub use module_cache::{ModuleCache, ModuleKey};
pub use platform::{MemoryReader, Module, is_executable, loaded_symbol, pin_module};
pub use signature::{SignaturePattern, exact_u32, find_all, is_exact, pattern};

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

/// A module that [`pin_module`] keeps loaded until the process exits.
#[derive(Debug, Clone)]
pub struct PinnedModule {
	base: usize,
	path: PathBuf,
}

impl PinnedModule {
	/// The module's load address.
	pub fn base(&self) -> usize {
		self.base
	}

	/// The file the loader mapped the module from.
	pub fn path(&self) -> &Path {
		&self.path
	}
}

/// The size of the place `_place` points to, for compile-time assertions
/// about the generated layouts.
pub(crate) const fn pointee_size<T>(_place: *const T) -> usize {
	size_of::<T>()
}
