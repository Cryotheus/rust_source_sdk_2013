//! Tests of the C++ ABI details of objects Rust implements for the engine:
//! destructor slots and vtable pages.

use source_sdk_2013_raw::abi::{ItaniumDestructors, MsvcDestructor, VtablePage};
use std::cell::Cell;
use std::mem::ManuallyDrop;

/// The size of a memory page on the supported targets, which a [`VtablePage`]
/// fills.
const PAGE_SIZE: usize = 4096;

/// Counts the times it is dropped.
struct DropCounter<'counter>(&'counter Cell<usize>);

impl Drop for DropCounter<'_> {
	fn drop(&mut self) {
		self.0.set(self.0.get() + 1);
	}
}

#[test]
fn itanium_complete_destructor_drops_the_concrete_type() {
	let drops = Cell::new(0);
	let mut value = ManuallyDrop::new(DropCounter(&drops));
	let destructors = ItaniumDestructors::new_leak::<DropCounter<'_>>();

	// SAFETY: `value` is a live `DropCounter`, which `ManuallyDrop` keeps
	// from being dropped again.
	unsafe { (destructors.complete_dtor)((&raw mut value).cast()) };
	assert_eq!(drops.get(), 1);
}

#[test]
fn msvc_non_deleting_destructor_drops_the_concrete_type() {
	let drops = Cell::new(0);
	let mut value = ManuallyDrop::new(DropCounter(&drops));
	let destructor = MsvcDestructor::new_leak::<DropCounter<'_>>();

	// SAFETY: `value` is a live `DropCounter`, which `ManuallyDrop` keeps
	// from being dropped again, and flags of 0 leave its memory alone.
	unsafe { destructor.0((&raw mut value).cast(), 0) };
	assert_eq!(drops.get(), 1);
}

#[test]
fn vtable_pages_are_page_aligned_and_give_their_tables_address() {
	static PAGE: VtablePage<[usize; 3]> = VtablePage::new([1, 2, 3]);

	assert_eq!(align_of::<VtablePage<[usize; 3]>>(), PAGE_SIZE);
	assert_eq!(PAGE.get().addr() % PAGE_SIZE, 0);
	assert_eq!(
		PAGE.get().cast_const(),
		(&raw const PAGE).cast::<[usize; 3]>()
	);
}
