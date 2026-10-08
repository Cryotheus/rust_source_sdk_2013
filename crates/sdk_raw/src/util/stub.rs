//! Jumps in memory of their own, outside every module, whose target can be
//! changed at any time.
//!
//! A library that hands one of its own functions to code that outlives it,
//! such as a global function pointer another library may save and call later,
//! can hand over a [`JumpStub`] to the function instead. Before the library
//! unloads, it points the stub elsewhere, so a caller that saved the stub's
//! address never calls into code that is gone.

#[cfg(test)]
#[path = "../tests/util/stub.rs"]
mod tests;

use super::platform::{allocate_pages, make_executable};
use std::ffi::c_void;
use std::io;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, Ordering};

/// `int3`, which fills the rest of the code page.
const INT3: u8 = 0xCC;

/// `jmp qword ptr [rip + disp32]`, the bytes before the displacement.
const JMP_RIP_INDIRECT: [u8; 2] = [0xFF, 0x25];

/// The size of a page on x86-64 under both supported systems, which split
/// code from data in a stub.
const PAGE: usize = 4096;

/// A jump to a function, in executable memory of its own outside every
/// module, that can be pointed at another function at any time, from any
/// thread.
///
/// The stub's code is one indirect jump through a pointer on the next page,
/// which stays writable while the code's page is only executable, so changing
/// the target never changes the protection of code another thread may be
/// running. The jump passes every argument register and the stack untouched,
/// so callers call the target with its own signature.
///
/// A stub is never freed, since code that saved its address may call it for as
/// long as the process runs. Each costs two pages.
#[derive(Debug, Clone, Copy)]
pub struct JumpStub {
	entry: NonNull<c_void>,
	target: &'static AtomicPtr<c_void>,
}

impl JumpStub {
	/// Allocates a stub that jumps to `target`.
	///
	/// Fails if the system refuses to allocate or protect the stub's pages,
	/// and always under Miri, which cannot run machine code.
	pub fn new(target: NonNull<c_void>) -> io::Result<Self> {
		if cfg!(miri) {
			return Err(io::ErrorKind::Unsupported.into());
		}

		let pages = allocate_pages(2 * PAGE)?;
		let code = pages.as_ptr();

		// SAFETY: The second page, of the two just allocated.
		let slot = unsafe { code.add(PAGE) }.cast::<AtomicPtr<c_void>>();

		// The jump reads its target from the slot, relative to the end of the
		// jump's six bytes.
		let displacement = i32::try_from(PAGE - 6).expect("the page is small");
		let mut jump = [INT3; 16];
		jump[..2].copy_from_slice(&JMP_RIP_INDIRECT);
		jump[2..6].copy_from_slice(&displacement.to_le_bytes());

		// SAFETY: The pages are writable, and nothing runs or reads them yet.
		// The slot is page-aligned, so aligned for an `AtomicPtr`.
		unsafe {
			code.write_bytes(INT3, PAGE);
			code.copy_from_nonoverlapping(jump.as_ptr(), jump.len());
			slot.write(AtomicPtr::new(target.as_ptr()));
		}

		// SAFETY: Only the code's page changes, which nothing runs yet; the
		// slot's page stays writable.
		unsafe { make_executable(pages.cast(), PAGE) }?;

		Ok(Self {
			entry: pages.cast(),
			// SAFETY: The slot is initialized, and its pages are never freed.
			target: unsafe { &*slot },
		})
	}

	/// The address to call: the stub's code, which jumps to its target.
	pub fn entry(self) -> NonNull<c_void> {
		self.entry
	}

	/// Points the stub at `target`. Calls that already jumped keep running
	/// the old target; every jump after this goes to the new one.
	pub fn retarget(self, target: NonNull<c_void>) {
		self.target.store(target.as_ptr(), Ordering::SeqCst);
	}

	/// The function the stub currently jumps to.
	pub fn target(self) -> NonNull<c_void> {
		NonNull::new(self.target.load(Ordering::SeqCst)).expect("a stub always has a target")
	}
}

// SAFETY: The stub is an address and a reference to an atomic, which any
// thread may read and change.
unsafe impl Send for JumpStub {}

// SAFETY: As above.
unsafe impl Sync for JumpStub {}
