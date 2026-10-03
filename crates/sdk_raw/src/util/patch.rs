//! Checked patching of code in loaded modules.
//!
//! On Windows, a `BytePatch` replaces one byte of code in place, such as a
//! conditional branch's opcode, only while the code around it still holds the
//! bytes it was verified with, and puts the original back when restored or
//! dropped. Patching is not supported on Linux.

#[cfg(target_os = "windows")]
use super::platform::{PAGE_EXECUTE_READWRITE, flush_instruction_cache, protect};

use std::io;

#[cfg(target_os = "windows")]
use std::ptr::NonNull;

/// One byte of code, replaced in place, with the block of code around it
/// that must keep the bytes it was verified with.
///
/// The patch writes nothing until [enabled](Self::enable). Dropping it
/// attempts [`Self::restore`], ignoring failures; call that first to observe
/// and handle them.
///
/// If a Windows call fails after the byte was written, the patch still owns
/// the write and any pending page-protection restore or instruction cache
/// flush, which the next [`Self::enable`] or [`Self::restore`] retries.
#[cfg(target_os = "windows")]
#[cfg_attr(docsrs, doc(cfg(target_os = "windows")))]
#[derive(Debug)]
pub struct BytePatch {
	/// The first byte of the verified block.
	block: NonNull<u8>,

	/// The block as verified, holding the original byte at `offset`.
	verified: Box<[u8]>,

	/// The patched byte's offset in the block.
	offset: usize,

	/// The byte that replaces the original.
	replacement: u8,

	/// Whether `replacement` was written and not yet restored.
	active: bool,

	/// Whether the last write still needs an instruction cache flush.
	cache_dirty: bool,

	/// The page protection to put back after a write, while that is pending.
	protection: Option<u32>,
}

#[cfg(target_os = "windows")]
impl BytePatch {
	/// Prepares to replace the byte at `offset` in the code at `block`, which
	/// held `verified` when it was checked, with `replacement`. Nothing is
	/// written until [`Self::enable`].
	///
	/// Returns `None` if `offset` lies outside `verified`, or the byte there
	/// already is `replacement`.
	///
	/// # Safety
	///
	/// - `block` points to `verified.len()` bytes of code in a module that
	///   stays loaded for as long as the patch exists.
	/// - The patch is dropped, which may restore the original byte, only
	///   while no thread executes or modifies the block.
	pub unsafe fn new(
		block: NonNull<u8>,
		verified: Box<[u8]>,
		offset: usize,
		replacement: u8,
	) -> Option<Self> {
		(*verified.get(offset)? != replacement).then_some(Self {
			block,
			verified,
			offset,
			replacement,
			active: false,
			cache_dirty: false,
			protection: None,
		})
	}

	/// Writes the replacement byte, once the whole block still holds the
	/// verified bytes.
	///
	/// This is idempotent after successful enabling, while the replacement
	/// stays in place, retrying only pending cleanup. If the block changed,
	/// nothing is written and [`PatchError::InstructionChanged`] is returned.
	///
	/// # Safety
	///
	/// No thread may execute or modify the block during the call.
	pub unsafe fn enable(&mut self) -> Result<(), PatchError> {
		let expected = if self.active {
			self.replacement
		} else {
			self.original()
		};

		// SAFETY: The block stays loaded and the caller excludes concurrent
		// execution and modification of it.
		if !unsafe { self.matches_block(expected) } {
			return Err(PatchError::InstructionChanged);
		}

		if self.active {
			self.finish_write()
		} else {
			// SAFETY: As above.
			unsafe { self.replace(self.original(), self.replacement, true) }
		}
	}

	/// Attempts the pending instruction cache flush and protection restore.
	///
	/// A step that fails stays pending for the next call. The flush's error
	/// takes precedence when both fail.
	fn finish_write(&mut self) -> Result<(), PatchError> {
		let location = self.location().cast_const().cast();

		let cache_error = if self.cache_dirty {
			match flush_instruction_cache(location, 1) {
				Ok(()) => {
					self.cache_dirty = false;
					None
				}

				Err(source) => Some(PatchError::Os {
					operation: PatchOperation::FlushInstructionCache,
					source,
				}),
			}
		} else {
			None
		};

		let protection_error = if let Some(protection) = self.protection {
			// SAFETY: This restores the protection Windows reported for the
			// byte's page, in the module that stays loaded, which nothing
			// executes from while the patch is changed.
			match unsafe { protect(location, 1, protection) } {
				Ok(_) => {
					self.protection = None;
					None
				}

				Err(source) => Some(PatchError::Os {
					operation: PatchOperation::Reprotect,
					source,
				}),
			}
		} else {
			None
		};

		match cache_error.or(protection_error) {
			Some(error) => Err(error),
			None => Ok(()),
		}
	}

	/// Whether the replacement was written and not yet restored.
	pub fn is_active(&self) -> bool {
		self.active
	}

	/// The patched byte.
	fn location(&self) -> *mut u8 {
		self.block.as_ptr().wrapping_add(self.offset)
	}

	/// Whether the live block equals the verified one with `byte` at the
	/// patched offset.
	///
	/// # Safety
	///
	/// The block must stay loaded, and nothing may modify it during the call.
	unsafe fn matches_block(&self, byte: u8) -> bool {
		self.verified.iter().enumerate().all(|(index, verified)| {
			let expected = if index == self.offset {
				byte
			} else {
				*verified
			};

			// SAFETY: The block of `verified.len()` bytes is live, as the
			// caller promises, and nothing modifies it.
			(unsafe { self.block.as_ptr().add(index).read_volatile() }) == expected
		})
	}

	/// The byte the block held at the patched offset when it was verified.
	fn original(&self) -> u8 {
		self.verified[self.offset]
	}

	/// Writes `value` over the patched byte, records `active`, and finishes
	/// the write.
	///
	/// Writes nothing and returns [`PatchError::InstructionChanged`] unless
	/// the byte is `expected`.
	///
	/// # Safety
	///
	/// The block must stay loaded, and no thread may execute or modify it
	/// during the call.
	unsafe fn replace(&mut self, expected: u8, value: u8, active: bool) -> Result<(), PatchError> {
		let location = self.location();

		// SAFETY: The byte is live, and nothing else modifies it.
		if unsafe { location.read_volatile() } != expected {
			return Err(PatchError::InstructionChanged);
		}

		// SAFETY: The byte's page belongs to the loaded module, and nothing
		// executes from it while its protection changes.
		let old = unsafe { protect(location.cast_const().cast(), 1, PAGE_EXECUTE_READWRITE) }
			.map_err(|source| PatchError::Os {
				operation: PatchOperation::Unprotect,
				source,
			})?;

		self.protection.get_or_insert(old);

		// SAFETY: The byte is writable, and nothing executes or modifies it.
		unsafe { location.write_volatile(value) };

		self.active = active;
		self.cache_dirty = true;
		self.finish_write()
	}

	/// Restores the original byte if [`Self::enable`] wrote it, with any
	/// pending page-protection restore and instruction cache flush.
	///
	/// This is idempotent after successful restoration. If another component
	/// already put the original byte back, only the pending cleanup runs. If
	/// it changed the block otherwise, the block is left alone and
	/// [`PatchError::InstructionChanged`] is returned. A failed Windows call
	/// can be retried.
	///
	/// # Safety
	///
	/// As for [`Self::enable`].
	pub unsafe fn restore(&mut self) -> Result<(), PatchError> {
		if !self.active {
			return self.finish_write();
		}

		// SAFETY: The block stays loaded and the caller excludes concurrent
		// execution and modification of it.
		if unsafe { self.location().read_volatile() } == self.original() {
			self.active = false;
			return self.finish_write();
		}

		// SAFETY: As above; refuse to change the byte if another component
		// modified the surrounding block.
		if !unsafe { self.matches_block(self.replacement) } {
			return Err(PatchError::InstructionChanged);
		}

		// SAFETY: As above.
		unsafe { self.replace(self.replacement, self.original(), false) }
	}
}

#[cfg(target_os = "windows")]
impl Drop for BytePatch {
	fn drop(&mut self) {
		// SAFETY: The constructor's caller promised that dropping happens
		// while nothing executes or modifies the block, which stays loaded.
		let _ = unsafe { self.restore() };
	}
}

/// Why a code patch could not be applied or undone.
#[derive(Debug, thiserror::Error)]
pub enum PatchError {
	/// The code no longer matches what the patch verified or wrote. It is
	/// left unchanged.
	#[error("the patched code no longer holds the verified instructions")]
	InstructionChanged,

	/// An OS call failed. The OS error is both in the message and the error's
	/// [`source`](std::error::Error::source).
	#[error("{operation:?} failed: {source}")]
	Os {
		/// The call that failed.
		operation: PatchOperation,

		/// The error the OS reported for the call.
		#[source]
		source: io::Error,
	},
}

/// An OS call that patching code makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatchOperation {
	/// Publishing a written byte to instruction fetch, with
	/// `FlushInstructionCache`.
	FlushInstructionCache,

	/// Restoring the original protection of the patched byte's page, with
	/// `VirtualProtect`.
	Reprotect,

	/// Making the patched byte's page writable, with `VirtualProtect`.
	Unprotect,
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
	use super::*;
	use crate::util::is_executable;
	use std::alloc::{Layout, alloc_zeroed, dealloc};

	/// The layout of a page the test owns alone.
	const PAGE: Layout = match Layout::from_size_align(4096, 4096) {
		Ok(layout) => layout,
		Err(_) => panic!("a page is a valid layout"),
	};

	/// The block the tests patch at offset 2.
	const VERIFIED: [u8; 4] = [0x48, 0x85, 0x75, 0x1c];

	/// A page of memory only this test uses, so changing its protection
	/// affects nothing else.
	struct Page(NonNull<u8>);

	impl Page {
		fn new() -> Self {
			// SAFETY: The layout has a nonzero size.
			let page = Self(NonNull::new(unsafe { alloc_zeroed(PAGE) }).unwrap());

			for (index, byte) in VERIFIED.into_iter().enumerate() {
				page.write(index, byte);
			}

			page
		}

		fn read(&self, index: usize) -> u8 {
			// SAFETY: The page is live, and the tests read within it.
			unsafe { self.0.as_ptr().add(index).read_volatile() }
		}

		fn write(&self, index: usize, byte: u8) {
			// SAFETY: The page is live and writable, and the tests write
			// within it.
			unsafe { self.0.as_ptr().add(index).write_volatile(byte) }
		}
	}

	impl Drop for Page {
		fn drop(&mut self) {
			// SAFETY: The page was allocated with this layout.
			unsafe { dealloc(self.0.as_ptr(), PAGE) }
		}
	}

	#[test]
	fn changed_blocks_are_left_alone() {
		let page = Page::new();
		// SAFETY: The page outlives the patch, and nothing executes it.
		let mut patch = unsafe { BytePatch::new(page.0, VERIFIED.into(), 2, 0xeb) }.unwrap();
		page.write(0, 0x90);
		// SAFETY: Nothing executes the page.
		assert!(matches!(
			unsafe { patch.enable() },
			Err(PatchError::InstructionChanged)
		));
		assert!(!patch.is_active());
		assert_eq!(page.read(2), 0x75);
		page.write(0, 0x48);
		// SAFETY: As above.
		unsafe { patch.enable() }.unwrap();
		page.write(3, 0x90);
		// SAFETY: As above.
		assert!(matches!(
			unsafe { patch.restore() },
			Err(PatchError::InstructionChanged)
		));
		assert!(patch.is_active());
		assert_eq!(page.read(2), 0xeb);
		page.write(3, 0x1c);
		// Another component put the original back, leaving only cleanup.
		page.write(2, 0x75);
		// SAFETY: As above.
		unsafe { patch.restore() }.unwrap();
		assert!(!patch.is_active());
	}

	#[test]
	fn enabling_and_restoring_are_idempotent_and_dropping_restores() {
		let page = Page::new();
		// SAFETY: The page outlives the patch, and nothing executes it.
		let mut patch = unsafe { BytePatch::new(page.0, VERIFIED.into(), 2, 0xeb) }.unwrap();

		// Writes make the page executable only until they finish.
		let address = page.0.as_ptr().addr();
		assert!(!is_executable(address));

		for _ in 0..2 {
			// SAFETY: Nothing executes the page.
			unsafe { patch.enable() }.unwrap();
			assert!(patch.is_active());
			assert_eq!(page.read(2), 0xeb);
			assert!(!is_executable(address));
		}

		for _ in 0..2 {
			// SAFETY: As above.
			unsafe { patch.restore() }.unwrap();
			assert!(!patch.is_active());
			assert_eq!(page.read(2), 0x75);
			assert!(!is_executable(address));
		}

		// SAFETY: As above.
		unsafe { patch.enable() }.unwrap();
		drop(patch);
		assert_eq!(page.read(2), 0x75);
		assert!(!is_executable(address));
	}

	#[test]
	fn offsets_must_be_in_the_block_and_change_the_byte() {
		let page = Page::new();
		// SAFETY: The page outlives the patches, and nothing executes it.
		unsafe {
			assert!(BytePatch::new(page.0, VERIFIED.into(), 4, 0xeb).is_none());
			assert!(BytePatch::new(page.0, VERIFIED.into(), 2, 0x75).is_none());
		}
	}
}
