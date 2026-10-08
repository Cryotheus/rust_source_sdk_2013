//! Tests of calls through C++ vtables, and reads of objects' vtable pointers.

use source_sdk_2013_raw::util::vtable::vtable_pointer;
use source_sdk_2013_raw::vcall;
use std::ffi::c_int;

static VTABLE: Vtable = Vtable { get, add };

#[repr(C)]
struct Base {
	vtable_: *const Vtable,
	value: c_int,
}

#[repr(C)]
struct Derived {
	_base: Base,
}

#[repr(C)]
struct Vtable {
	get: unsafe extern "C" fn(this: *mut Base) -> c_int,
	add: unsafe extern "C" fn(this: *mut Derived, amount: c_int) -> c_int,
}

unsafe extern "C" fn add(this: *mut Derived, amount: c_int) -> c_int {
	// SAFETY: The tests pass a live `Derived`.
	unsafe { (&raw const (*this)._base.value).read() + amount }
}

#[test]
fn calls_go_through_the_objects_vtable() {
	let mut object = Derived {
		_base: Base {
			vtable_: &raw const VTABLE,
			value: 40,
		},
	};
	let base = (&raw mut object).cast::<Base>();
	let derived = &raw mut object;

	// SAFETY: Both pointers are to the live object, whose vtable is
	// `VTABLE`.
	unsafe {
		assert_eq!(vcall!(base => get()), 40);
		assert_eq!(vcall!(derived as Vtable => add(2)), 42);
		assert_eq!(vtable_pointer::<Vtable>(derived), &raw const VTABLE);
	}
}

unsafe extern "C" fn get(this: *mut Base) -> c_int {
	// SAFETY: The tests pass a live `Base`.
	unsafe { (&raw const (*this).value).read() }
}
