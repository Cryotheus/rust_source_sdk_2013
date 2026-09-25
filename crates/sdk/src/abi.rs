use std::alloc::{Layout, dealloc};
use std::ffi::c_void;
use std::ptr::drop_in_place;

/// Function pointer(s) present at the start of a vtable.
pub type CppDestructors = cfg_select! {
	target_os = "linux" => ItaniumDestructors,
	target_os = "windows" => MsvcDestructor,
};

/// Itanium C++ destructor slots for vtables implemented by Rust-owned objects.
/// Prefer [`CppDestructors`] when the code should select the target ABI.
///
/// The ABI is specialized for GCC.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ItaniumDestructors {
	pub complete_dtor: unsafe extern "C" fn(this: *mut c_void),
	pub delete_dtor: unsafe extern "C" fn(this: *mut c_void),
}

impl ItaniumDestructors {
	pub const VTABLE_SLOTS: usize = 2;

	/// # Safety
	/// The `delete` destructor uses Rust's allocator,
	/// and thus is undefined behavor if the object has been constructor within the C++ CRT heap.
	///
	/// This function itself is safe, as only calling the destructors raises soundness concerns.
	pub const fn new<T>() -> Self {
		unsafe extern "C" fn delete_dtor<T>(this: *mut c_void) {
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

	/// Calls [`std::process::abort`] if the delete destructor is used.
	///
	/// Technically doesn't cause a memory leak as the OS immediately releases the program's memory...
	/// I don't know, ask Jon.
	pub const fn new_dealloc_abort<T>() -> Self {
		unsafe extern "C" fn delete_dtor<T>(this: *mut c_void) {
			unsafe { drop_in_place(this.cast::<T>()) };
			std::process::abort()
		}

		Self {
			complete_dtor: c_drop_in_place::<T>,
			delete_dtor: delete_dtor::<T>,
		}
	}

	/// # Panics
	/// If delete is used.
	///
	/// # Memory Leak
	/// Won't deallocate where `delete` destructor is used, but still calls drop glue.
	pub const fn new_dealloc_panic<T>() -> Self {
		unsafe extern "C" fn delete_dtor<T>(this: *mut c_void) {
			unsafe { drop_in_place(this.cast::<T>()) };
			panic!()
		}

		Self {
			complete_dtor: c_drop_in_place::<T>,
			delete_dtor: delete_dtor::<T>,
		}
	}

	/// # Memory Leak
	/// Won't deallocate where `delete` destructor is used, but still runs typical Rust drop glue.
	pub const fn new_leak<T>() -> Self {
		Self {
			complete_dtor: c_drop_in_place::<T>,
			delete_dtor: c_drop_in_place::<T>,
		}
	}

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
pub struct MsvcDestructor(pub unsafe extern "C" fn(this: *mut c_void, flags: u32) -> *mut c_void);

impl MsvcDestructor {
	pub const VTABLE_SLOTS: usize = 1;

	/// # Safety
	/// The `delete` destructor uses Rust's allocator,
	/// and thus is undefined behavor if the object has been constructed within the C++ CRT heap.
	/// That is to say: allocation should be made on the Rust side.
	///
	/// This function itself is safe, as only calling the destructors raises soundness concerns.
	pub const fn new<T>() -> Self {
		unsafe extern "C" fn dtor<T>(this: *mut c_void, flags: u32) -> *mut c_void {
			unsafe { drop_in_place(this.cast::<T>()) };

			if flags & 1 == 1 {
				unsafe { dealloc(this.cast(), Layout::new::<T>()) }
			}

			this
		}

		Self(dtor::<T>)
	}

	/// Calls [`std::process::abort`] if the delete destructor is used.
	///
	/// Technically doesn't cause a memory leak as the OS immediately releases the program's memory...
	/// I don't know, ask Jon.
	pub const fn new_dealloc_abort<T>() -> Self {
		unsafe extern "C" fn dtor<T>(this: *mut c_void, flags: u32) -> *mut c_void {
			unsafe { drop_in_place(this.cast::<T>()) };
			if flags & 1 == 1 {
				std::process::abort()
			} else {
				this
			}
		}

		Self(dtor::<T>)
	}

	/// # Panics
	/// If delete is used.
	///
	/// # Memory Leak
	/// Won't deallocate where `delete` destructor is used, but still calls drop glue.
	pub const fn new_dealloc_panic<T>() -> Self {
		unsafe extern "C" fn dtor<T>(this: *mut c_void, flags: u32) -> *mut c_void {
			unsafe { drop_in_place(this.cast::<T>()) };
			if flags & 1 == 1 { panic!() } else { this }
		}

		Self(dtor::<T>)
	}

	/// # Memory Leak
	/// Won't deallocate where `delete` destructor is used, but still runs typical Rust drop glue.
	pub const fn new_leak<T>() -> Self {
		unsafe extern "C" fn dtor<T>(this: *mut c_void, _: u32) -> *mut c_void {
			unsafe { drop_in_place(this.cast::<T>()) };
			this
		}

		Self(dtor::<T>)
	}

	/// # Memory Leak
	/// Won't deallocate where `delete` destructor is used.
	pub const fn new_noop() -> Self {
		extern "C" fn noop(this: *mut c_void, _: u32) -> *mut c_void {
			this
		}

		Self(noop)
	}
}

pub type WChar = cfg_select! {
	all(target_os = "windows", target_pointer_width = "64") => u16,
	all(target_os = "linux", target_pointer_width = "64") => u32,
	_ => compile_error!("Unsupported target"),
};

unsafe extern "C" fn c_drop_in_place<T>(to_drop: *mut c_void) {
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

		unsafe { (destructors.complete_dtor)((&raw mut value).cast()) };
		assert_eq!(drops.get(), 1);
	}

	#[test]
	fn msvc_non_deleting_destructor_drops_the_concrete_type() {
		let drops = Cell::new(0);
		let mut value = ManuallyDrop::new(DropCounter(&drops));
		let destructor = MsvcDestructor::new_leak::<DropCounter<'_>>();

		unsafe { destructor.0((&raw mut value).cast(), 0) };
		assert_eq!(drops.get(), 1);
	}
}
