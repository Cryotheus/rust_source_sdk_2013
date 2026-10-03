//! Details of the target's C++ ABI that objects Rust implements for the engine
//! need, such as the destructor slots at the start of their vtables.
//!
//! # Destructors
//!
//! C++ calls an object's destructor at most once, with `this` pointing to the
//! live object the destructor was made for. Each constructor of
//! [`ItaniumDestructors`] and [`MsvcDestructor`] documents what its deleting
//! destructor does with the object's memory; making destructors is safe, and
//! whoever gives C++ an object whose vtable holds them must uphold what they
//! require of that object.

use std::alloc::{Layout, dealloc};
use std::cell::UnsafeCell;
use std::ffi::c_void;
use std::ptr::drop_in_place;

pub use crate::vtable_slot;

/// Function pointer(s) present at the start of a vtable.
///
/// This is [`ItaniumDestructors`] on Linux and [`MsvcDestructor`] on Windows.
pub type CppDestructors = cfg_select! {
	target_os = "linux" => ItaniumDestructors,
	target_os = "windows" => MsvcDestructor,
};

/// An unsigned integer as wide as C++'s `wchar_t` on the target: 16 bits on
/// Windows and 32 bits on Linux.
#[doc(alias = "wchar_t")]
pub type WChar = cfg_select! {
	all(target_os = "windows", target_pointer_width = "64") => u16,
	all(target_os = "linux", target_pointer_width = "64") => u32,
};

/// The bit of an MSVC deleting destructor's flags that asks it to free the
/// object after destroying it.
const MSVC_DELETE_FLAG: u32 = 1;

/// The size of a memory page on the supported targets, which a
/// [`VtablePage`] fills.
const PAGE_SIZE: usize = 4096;

/// The size of one vtable slot, a pointer.
pub const VTABLE_SLOT_SIZE: usize = size_of::<*const ()>();

/// Itanium C++ destructor slots for vtables implemented by Rust-owned objects.
/// Prefer [`CppDestructors`] when the code should select the target ABI.
///
/// The ABI is specialized for GCC.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ItaniumDestructors {
	/// The complete object destructor, which destroys the object without
	/// freeing it.
	pub complete_dtor: unsafe extern "C" fn(this: *mut c_void),

	/// The deleting destructor, which C++ calls to destroy the object, then
	/// free it.
	///
	/// What it does with the object's memory depends on the constructor that
	/// made it. The one from [`Self::new`] frees the object with Rust's global
	/// allocator, so C++ may only call it for an object allocated that way,
	/// never for one in the C++ runtime's heap.
	pub delete_dtor: unsafe extern "C" fn(this: *mut c_void),
}

impl ItaniumDestructors {
	/// The number of vtable slots the destructors occupy.
	pub const VTABLE_SLOTS: usize = 2;

	/// Destructors that drop a `T` in place, the deleting one then freeing it
	/// with the global allocator and `T`'s layout, as dropping a `Box<T>` would.
	///
	/// The deleting destructor may therefore only be called for a `T` the
	/// global allocator allocated with `T`'s layout, such as a leaked
	/// `Box<T>`.
	pub const fn new<T>() -> Self {
		unsafe extern "C" fn delete_dtor<T>(this: *mut c_void) {
			// SAFETY: C++ calls a destructor once, with `this` pointing to the
			// live `T` it was made for, which the global allocator allocated with
			// `T`'s layout, as `new` documents.
			unsafe {
				drop_in_place(this.cast::<T>());
				dealloc(this.cast(), Layout::new::<T>());
			}
		}

		Self {
			complete_dtor: c_drop_in_place::<T>,
			delete_dtor: delete_dtor::<T>,
		}
	}

	/// Destructors that drop a `T` in place, the deleting one then aborting
	/// the process instead of freeing it.
	///
	/// Nothing leaks, since the process ends.
	pub const fn new_dealloc_abort<T>() -> Self {
		unsafe extern "C" fn delete_dtor<T>(this: *mut c_void) {
			// SAFETY: As for `c_drop_in_place`.
			unsafe { drop_in_place(this.cast::<T>()) };
			std::process::abort()
		}

		Self {
			complete_dtor: c_drop_in_place::<T>,
			delete_dtor: delete_dtor::<T>,
		}
	}

	/// Destructors that drop a `T` in place, the deleting one then panicking
	/// instead of freeing it.
	///
	/// # Panics
	/// If delete is used. The panic cannot unwind out of the `extern "C"`
	/// destructor, so it aborts the process.
	///
	/// # Memory Leak
	/// Won't deallocate where `delete` destructor is used, but still calls drop glue.
	pub const fn new_dealloc_panic<T>() -> Self {
		unsafe extern "C" fn delete_dtor<T>(this: *mut c_void) {
			// SAFETY: As for `c_drop_in_place`.
			unsafe { drop_in_place(this.cast::<T>()) };
			panic!()
		}

		Self {
			complete_dtor: c_drop_in_place::<T>,
			delete_dtor: delete_dtor::<T>,
		}
	}

	/// Destructors that drop a `T` in place and never free it.
	///
	/// # Memory Leak
	/// Won't deallocate where `delete` destructor is used, but still runs typical Rust drop glue.
	pub const fn new_leak<T>() -> Self {
		Self {
			complete_dtor: c_drop_in_place::<T>,
			delete_dtor: c_drop_in_place::<T>,
		}
	}

	/// Destructors that do nothing, leaving the object to its Rust owner.
	///
	/// # Memory Leak
	/// Won't deallocate where `delete` destructor is used.
	pub const fn new_noop() -> Self {
		extern "C" fn noop(_: *mut c_void) {}

		Self {
			complete_dtor: noop,
			delete_dtor: noop,
		}
	}
}

/// MSVC C++ deleting-destructor slot for vtables implemented by Rust-owned objects.
/// Prefer [`CppDestructors`] when the code should select the target ABI.
///
/// The ABI is specialized for MSVC.
#[derive(Debug, Clone, Copy)]
#[repr(transparent)]
pub struct MsvcDestructor(
	/// The deleting destructor, which destroys the object and returns `this`.
	/// C++ sets bit 0 of `flags` when it should also free the object.
	///
	/// What it does with the object's memory then depends on the constructor
	/// that made it. The one from [`Self::new`] frees the object with Rust's
	/// global allocator, so C++ may only ask it to for an object allocated
	/// that way, never for one in the C++ runtime's heap.
	pub unsafe extern "C" fn(this: *mut c_void, flags: u32) -> *mut c_void,
);

impl MsvcDestructor {
	/// The number of vtable slots the destructor occupies.
	pub const VTABLE_SLOTS: usize = 1;

	/// A destructor that drops a `T` in place, then, if asked to delete it,
	/// frees it with the global allocator and `T`'s layout, as dropping a
	/// `Box<T>` would.
	///
	/// It may therefore only be asked to delete a `T` the global allocator
	/// allocated with `T`'s layout, such as a leaked `Box<T>`.
	pub const fn new<T>() -> Self {
		unsafe extern "C" fn dtor<T>(this: *mut c_void, flags: u32) -> *mut c_void {
			// SAFETY: As for `c_drop_in_place`.
			unsafe { drop_in_place(this.cast::<T>()) };

			if flags & MSVC_DELETE_FLAG != 0 {
				// SAFETY: An object C++ deletes was allocated by the global
				// allocator with `T`'s layout, as `new` documents.
				unsafe { dealloc(this.cast(), Layout::new::<T>()) }
			}

			this
		}

		Self(dtor::<T>)
	}

	/// A destructor that drops a `T` in place, then aborts the process if
	/// asked to delete it.
	///
	/// Nothing leaks, since the process ends.
	pub const fn new_dealloc_abort<T>() -> Self {
		unsafe extern "C" fn dtor<T>(this: *mut c_void, flags: u32) -> *mut c_void {
			// SAFETY: As for `c_drop_in_place`.
			unsafe { drop_in_place(this.cast::<T>()) };
			if flags & MSVC_DELETE_FLAG != 0 {
				std::process::abort()
			} else {
				this
			}
		}

		Self(dtor::<T>)
	}

	/// A destructor that drops a `T` in place, then panics if asked to delete
	/// it.
	///
	/// # Panics
	/// If delete is used. The panic cannot unwind out of the `extern "C"`
	/// destructor, so it aborts the process.
	///
	/// # Memory Leak
	/// Won't deallocate where `delete` destructor is used, but still calls drop glue.
	pub const fn new_dealloc_panic<T>() -> Self {
		unsafe extern "C" fn dtor<T>(this: *mut c_void, flags: u32) -> *mut c_void {
			// SAFETY: As for `c_drop_in_place`.
			unsafe { drop_in_place(this.cast::<T>()) };
			if flags & MSVC_DELETE_FLAG != 0 {
				panic!()
			} else {
				this
			}
		}

		Self(dtor::<T>)
	}

	/// A destructor that drops a `T` in place and never frees it.
	///
	/// # Memory Leak
	/// Won't deallocate where `delete` destructor is used, but still runs typical Rust drop glue.
	pub const fn new_leak<T>() -> Self {
		unsafe extern "C" fn dtor<T>(this: *mut c_void, _: u32) -> *mut c_void {
			// SAFETY: As for `c_drop_in_place`.
			unsafe { drop_in_place(this.cast::<T>()) };
			this
		}

		Self(dtor::<T>)
	}

	/// A destructor that does nothing, leaving the object to its Rust owner.
	///
	/// # Memory Leak
	/// Won't deallocate where `delete` destructor is used.
	pub const fn new_noop() -> Self {
		extern "C" fn noop(this: *mut c_void, _: u32) -> *mut c_void {
			this
		}

		Self(noop)
	}
}

/// A vtable Rust implements for the engine, alone on its memory page.
///
/// Other plugins hook an object by overwriting entries of its vtable in
/// place, so the table is interior-mutable, which also places a `static` one
/// in writable memory. KHook sets a page it patched back to read-and-execute,
/// which would fault the next write to any other static on that page; the
/// alignment makes the table fill its page. The table is only reached through
/// the address [`Self::get`] returns, which is what the engine's objects
/// point at.
#[repr(C, align(4096))]
pub struct VtablePage<T>(UnsafeCell<T>);

impl<T> VtablePage<T> {
	/// Places `vtable` alone on its page.
	///
	/// Fails to compile if the table does not fit in one page.
	pub const fn new(vtable: T) -> Self {
		const { assert!(size_of::<Self>() == PAGE_SIZE) };

		Self(UnsafeCell::new(vtable))
	}

	/// The address of the table, for the objects that use it.
	pub const fn get(&self) -> *mut T {
		self.0.get()
	}
}

// SAFETY: A shared `VtablePage` never reads or writes its table: it only gives
// out the table's address, and every access through that address is `unsafe`,
// for its user to synchronize. The engine and hooking libraries access tables
// on the server's main thread.
unsafe impl<T> Sync for VtablePage<T> {}

/// The index of a method's slot in a generated vtable struct: the offset of
/// its field divided by [`VTABLE_SLOT_SIZE`].
///
/// `vtable_slot!(Vtable, Field)` is a constant expression, so it can define
/// slot constants or check hand-written slot numbers against a generated
/// binding.
///
/// ```
/// use source_sdk_2013_raw::abi::vtable_slot;
///
/// #[repr(C)]
/// struct Vtable {
///     first: unsafe extern "C" fn(),
///     second: unsafe extern "C" fn(),
/// }
///
/// const _: () = assert!(vtable_slot!(Vtable, second) == 1);
/// ```
#[macro_export]
macro_rules! vtable_slot {
	($Vtable:ty, $field:ident) => {
		::core::mem::offset_of!($Vtable, $field) / $crate::abi::VTABLE_SLOT_SIZE
	};
}

/// Drops the `T` that `to_drop` points to, as a destructor slot that leaves
/// freeing it to its owner.
unsafe extern "C" fn c_drop_in_place<T>(to_drop: *mut c_void) {
	// SAFETY: C++ calls a destructor once, with `to_drop` pointing to the live
	// `T` it was made for.
	unsafe { drop_in_place(to_drop.cast::<T>()) };
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::cell::Cell;
	use std::mem::ManuallyDrop;

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

		// SAFETY: As above.
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
}
