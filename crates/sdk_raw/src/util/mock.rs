//! Helpers for tests that fake engine objects, such as mock vtables.
//!
//! These exist for tests only, of this crate and of crates built on it, which
//! enable the `test-support` feature as a dev-dependency. They are not part of
//! the crate's API.

use crate::abi::VTABLE_SLOT_SIZE;

/// Builds a vtable whose every slot holds `fill`, then lets `patch` install
/// the methods a test expects to be called.
///
/// For tests only.
///
/// # Panics
///
/// If `V`'s size is not a whole number of slots.
///
/// # Safety
///
/// `V` must consist solely of function pointer slots, and `fill` must be
/// callable through any of them without being reached, e.g. an aborting stub
/// such as [`unexpected_call`].
pub unsafe fn mock_vtable<V>(fill: *const (), patch: impl FnOnce(*mut V)) -> Box<V> {
	assert_eq!(size_of::<V>() % VTABLE_SLOT_SIZE, 0);

	let mut vtable = Box::<V>::new_uninit();
	let slots = vtable.as_mut_ptr().cast::<*const ()>();

	for slot in 0..size_of::<V>() / VTABLE_SLOT_SIZE {
		// SAFETY: The slot lies within the allocation.
		unsafe { slots.add(slot).write(fill) };
	}

	patch(vtable.as_mut_ptr());

	// SAFETY: Every slot now holds a function pointer.
	unsafe { vtable.assume_init() }
}

/// A vtable slot that fails the test if it is ever called.
///
/// For tests only. Unwinding cannot leave an `extern "C"` function, so a call
/// aborts the test process.
pub extern "C" fn unexpected_call() {
	panic!("unexpected virtual call");
}
