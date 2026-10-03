//! Helpers for tests that fake engine objects: mock vtables, and builders of
//! the engine's data structures that this crate's tests and those of crates
//! built on it share.
//!
//! These exist for tests only, of this crate and of crates built on it, which
//! enable the `test-support` feature as a dev-dependency. They are not part of
//! the crate's API.

pub mod commands;
pub mod edicts;
pub mod entities;
pub mod net;

use crate::abi::VTABLE_SLOT_SIZE;
use std::ffi::c_int;

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

/// A tier1 `CUtlVector` viewing `values`, as the engine's vectors hold their
/// elements: its capacity and size are both the slice's length.
///
/// For tests only. Both element pointers come from one `as_mut_ptr` call,
/// since a second call would invalidate the first. The vector borrows
/// nothing, so `values` must stay in place while it is used.
///
/// # Panics
///
/// If the slice is longer than a `c_int` can count.
pub fn utl_vector<T>(values: &mut [T]) -> sys::CUtlVector<T, sys::CUtlMemory<T>> {
	let len = c_int::try_from(values.len()).unwrap();
	let elements = values.as_mut_ptr();

	sys::CUtlVector {
		_phantom_0: Default::default(),
		_phantom_1: Default::default(),
		m_Memory: sys::CUtlMemory {
			_phantom_0: Default::default(),
			m_pMemory: elements,
			m_nAllocationCount: len,
			m_nGrowSize: 0,
		},
		m_Size: len,
		m_pElements: elements,
	}
}
