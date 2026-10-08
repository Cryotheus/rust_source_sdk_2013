#![cfg(target_os = "windows")]
//! Tests of patching code in place on Windows.

use source_sdk_2013_raw::util::is_executable;
use source_sdk_2013_raw::util::patch::{BytePatch, PatchError};
use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::ptr::NonNull;

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
	// SAFETY: Nothing executes the page.
	unsafe { patch.enable() }.unwrap();
	page.write(3, 0x90);
	// SAFETY: Nothing executes the page.
	assert!(matches!(
		unsafe { patch.restore() },
		Err(PatchError::InstructionChanged)
	));
	assert!(patch.is_active());
	assert_eq!(page.read(2), 0xeb);
	page.write(3, 0x1c);
	// Another component put the original back, leaving only cleanup.
	page.write(2, 0x75);
	// SAFETY: Nothing executes the page.
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
		// SAFETY: Nothing executes the page.
		unsafe { patch.restore() }.unwrap();
		assert!(!patch.is_active());
		assert_eq!(page.read(2), 0x75);
		assert!(!is_executable(address));
	}

	// SAFETY: Nothing executes the page.
	unsafe { patch.enable() }.unwrap();
	drop(patch);
	assert_eq!(page.read(2), 0x75);
	assert!(!is_executable(address));
}
