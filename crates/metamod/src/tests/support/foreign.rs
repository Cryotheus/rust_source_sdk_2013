//! Another plugin's hooks, on functions of one parameter after `this`: a
//! SourceHook delegate, and a hook as KHook's `KHook::Virtual` installs it.

use super::khook::KhHook;
use crate::hook::HookReturn;
use crate::sys::khook::{Action, IKHook};
use crate::sys::sourcehook::{IShDelegate, ISourceHook, MetaRes};
use std::cell::Cell;
use std::ffi::c_void;
use std::mem::{self, size_of};
use std::ptr;

thread_local! {
	/// The mock KHook, for the hooks of another plugin's to call.
	pub(crate) static FOREIGN_KHOOK: Cell<*mut IKHook> = const { Cell::new(ptr::null_mut()) };
}

/// Another plugin's SourceHook delegate, which reports `result` and returns
/// `value`.
#[repr(C)]
pub(crate) struct ForeignDelegate<R> {
	vtable: *const *mut c_void,
	sourcehook: *mut ISourceHook,
	/// What the delegate reports to SourceHook.
	pub(crate) result: Cell<MetaRes>,
	value: R,
	/// `IsEqual`, `DeleteThis` and `Call`, which `vtable` points to.
	functions: [*mut c_void; 3],
}

impl<R: HookReturn> ForeignDelegate<R> {
	/// A delegate on a function whose parameter after `this` is an `A`.
	pub(crate) fn new<A: Copy + 'static>(
		sourcehook: *mut ISourceHook,
		result: MetaRes,
		value: R,
	) -> Box<Self> {
		let mut delegate = Box::new(Self {
			vtable: ptr::null(),
			sourcehook,
			result: Cell::new(result),
			value,
			functions: [
				is_equal as unsafe extern "C" fn(*mut IShDelegate, *mut IShDelegate) -> bool
					as *mut c_void,
				delete_this as unsafe extern "C" fn(*mut IShDelegate) as *mut c_void,
				call::<A, R> as unsafe extern "C" fn(*mut Self, A) -> R as *mut c_void,
			],
		});

		delegate.vtable = delegate.functions.as_ptr();
		delegate
	}

	pub(crate) fn ptr(&self) -> *mut IShDelegate {
		ptr::from_ref(self).cast::<IShDelegate>().cast_mut()
	}
}

/// The delegate's `Call`, with the delegate as `this`.
unsafe extern "C" fn call<A, R: HookReturn>(delegate: *mut ForeignDelegate<R>, _arg: A) -> R {
	// SAFETY: The tests' foreign delegates are live while hooked.
	let delegate = unsafe { &*delegate };

	// SAFETY: SourceHook is calling the delegate.
	unsafe {
		((*(*delegate.sourcehook).vtable).set_res)(delegate.sourcehook, delegate.result.get())
	};

	delegate.value
}

unsafe extern "C" fn copy<R: HookReturn>(destination: *mut R, value: *const R) {
	// SAFETY: KHook copies the value it was given into its storage.
	unsafe { destination.write(value.read()) };
}

unsafe extern "C" fn delete_this(_this: *mut IShDelegate) {}

unsafe extern "C" fn destroy<R>(_value: *mut R) {}

unsafe extern "C" fn is_equal(this: *mut IShDelegate, other: *mut IShDelegate) -> bool {
	this == other
}

/// Another plugin's `make_call_original`, as `KHook::Virtual` makes it.
unsafe extern "C" fn khook_call_original<T, A, R: HookReturn>(this: *mut T, arg: A) -> R {
	let khook = FOREIGN_KHOOK.get();

	// SAFETY: The mock KHook is running a detour of a function of this type,
	// which keeps the original value until the call ends.
	unsafe {
		let functions = &*(*khook).vtable;

		let original =
			mem::transmute::<*mut c_void, unsafe extern "C" fn(*mut T, A) -> R>((functions
				.get_original_function)(
				khook
			));

		let mut value = original(this, arg);

		(functions.save_return_value)(
			khook,
			Action::IGNORE,
			ptr::from_mut(&mut value).cast(),
			size_of::<R>(),
			copy::<R> as unsafe extern "C" fn(*mut R, *const R) as *mut c_void,
			destroy::<R> as unsafe extern "C" fn(*mut R) as *mut c_void,
			true,
		);

		value
	}
}

/// Another plugin's `make_return`, as `KHook::Virtual` makes it.
unsafe extern "C" fn khook_make_return<T, A, R: HookReturn>(_this: *mut T, _arg: A) -> R {
	let khook = FOREIGN_KHOOK.get();

	// SAFETY: As for `khook_call_original`. A value KHook kept is an `R`, and
	// a function returning nothing has none.
	unsafe {
		let functions = &*(*khook).vtable;
		let value = (functions.get_current_value_ptr)(khook, true).cast::<R>();

		let value = match value.is_null() {
			true => mem::zeroed(),
			false => value.read(),
		};

		(functions.destroy_return_value)(khook);
		value
	}
}

/// Another plugin's pre hook, superseding the call with the value its
/// context points to.
unsafe extern "C" fn khook_supersede<T, A, R: HookReturn>(_this: *mut T, _arg: A) -> R {
	let khook = FOREIGN_KHOOK.get();

	// SAFETY: As for `khook_call_original`. The hook's context is the value,
	// which outlives the hook, and all-zero bytes are an `R`.
	unsafe {
		let functions = &*(*khook).vtable;
		let mut value = (functions.get_context_ptr)(khook).cast::<R>().read();

		(functions.save_return_value)(
			khook,
			Action::SUPERSEDE,
			ptr::from_mut(&mut value).cast(),
			size_of::<R>(),
			copy::<R> as unsafe extern "C" fn(*mut R, *const R) as *mut c_void,
			destroy::<R> as unsafe extern "C" fn(*mut R) as *mut c_void,
			false,
		);

		mem::zeroed()
	}
}

/// Another plugin's pre hook on a function of type
/// `unsafe extern "C" fn(*mut T, A) -> R`, superseding each call with the `R`
/// that `value` points to, which must outlive the hook.
pub(crate) fn khook_superseding<T, A, R>(value: *const R) -> KhHook
where
	T: 'static,
	A: Copy + 'static,
	R: HookReturn,
{
	KhHook {
		context: value.cast_mut().cast(),
		pre: khook_supersede::<T, A, R> as unsafe extern "C" fn(*mut T, A) -> R as *mut c_void,
		post: ptr::null_mut(),
		make_return: khook_make_return::<T, A, R> as unsafe extern "C" fn(*mut T, A) -> R
			as *mut c_void,
		call_original: khook_call_original::<T, A, R> as unsafe extern "C" fn(*mut T, A) -> R
			as *mut c_void,
		stack_size: 0,
	}
}
