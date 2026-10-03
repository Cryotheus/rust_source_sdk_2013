//! Calls through C++ vtables, and reads of objects' vtable pointers.
//!
//! Vtables are reached through raw pointers only, never `&V`: hook managers
//! patch vtables in place, so no reference to one could promise that it does
//! not change.

pub use crate::vcall;

/// Reads the pointer to the primary vtable of the polymorphic object `this`
/// points to, as a `*const V`.
///
/// This is for classes whose generated binding has no top-level `vtable_`
/// field, such as derived classes like `sys::CTFPlayer`, whose vtable pointer
/// is the first field of their primary base. The result is a raw pointer,
/// never `&V`, since hook managers patch vtables in place.
///
/// # Safety
///
/// `this` must point to a live polymorphic object whose primary vtable is laid
/// out as `V` on the target's ABI, such as the generated `__bindgen_vtable`
/// struct of its class.
pub unsafe fn vtable_pointer<V>(this: *const impl Sized) -> *const V {
	// SAFETY: A polymorphic object starts with the pointer to its primary
	// vtable, which the caller guarantees is laid out as `V`.
	unsafe { this.cast::<*const V>().read() }
}

/// Calls a virtual method through a generated binding's vtable.
///
/// `vcall!(this => Method(arguments...))` reads the vtable pointer and calls
/// `Method` with `this` as the receiver, without creating a reference to the
/// object. It must be used inside `unsafe`, with `this` pointing to a live
/// object whose `vtable_` matches the binding.
///
/// `vcall!(this as Vtable => Method(arguments...))` instead reads the
/// object's primary vtable as the generated `Vtable` struct, through
/// [`vtable_pointer`](crate::util::vtable::vtable_pointer), and casts `this`
/// to the method's receiver type. It is for classes whose generated binding
/// has no top-level `vtable_`, such as `sys::CTFPlayer`, whose methods take a
/// pointer to the class itself. `this` must be a local variable holding the
/// pointer. It must be used inside `unsafe`, with `this` pointing to a live
/// object whose primary vtable is laid out as `Vtable` on the target's ABI.
#[macro_export]
macro_rules! vcall {
	($this:ident as $Vtable:ty => $method:ident($($argument:expr),* $(,)?)) => {{
		let this = $this;
		let vtable = $crate::util::vtable::vtable_pointer::<$Vtable>(this);

		((*vtable).$method)(this.cast() $(, $argument)*)
	}};

	($this:expr => $method:ident($($argument:expr),* $(,)?)) => {{
		let this = $this;
		let vtable = (&raw const (*this).vtable_).read();

		((*vtable).$method)(this $(, $argument)*)
	}};
}

#[cfg(test)]
mod tests {
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
			assert_eq!(super::vtable_pointer::<Vtable>(derived), &raw const VTABLE);
		}
	}

	unsafe extern "C" fn get(this: *mut Base) -> c_int {
		// SAFETY: The tests pass a live `Base`.
		unsafe { (&raw const (*this).value).read() }
	}
}
