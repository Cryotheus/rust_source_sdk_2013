//! The types of functions hooks apply to, and the calls into them each library
//! makes, generated for each.

use super::{khook, sourcehook};
use std::ffi::c_void;
use std::marker::PhantomData;
use std::mem::{self, size_of};
use std::ptr::NonNull;

/// How many virtual functions, told apart by signature and vtable slot, a
/// plugin can hook under SourceHook, which needs a hook manager of the
/// plugin's own for each.
pub(super) const HOOK_MANAGERS: usize = 64;

/// A parameter type of a hooked function, after `this`.
///
/// # Safety
///
/// C must pass a value of the type the way C++ passes the parameter's: as a
/// scalar or pointer of the same size and kind, or a `#[repr(C)]` type with
/// the layout of a trivially copyable C++ class.
pub unsafe trait HookArg: Copy + 'static {}

/// A return type of a hooked function: `()` for `void`.
///
/// # Safety
///
/// C must return a value of the type the way C++ returns the function's: as a
/// scalar or pointer of the same size and kind, in a register. No class type
/// qualifies, as MSVC returns every one through memory from a member function.
/// All-zero bytes must be a valid value.
pub unsafe trait HookReturn: Copy + 'static {}

macro_rules! scalars {
	($($ty:ty),* $(,)?) => {$(
		// SAFETY: C passes and returns scalars as C++ does, and zero is a value
		// of each.
		unsafe impl HookArg for $ty {}

		// SAFETY: As above.
		unsafe impl HookReturn for $ty {}
	)*};
}

scalars!(
	bool, i8, u8, i16, u16, i32, u32, i64, u64, isize, usize, f32, f64
);

/// The type of a C++ virtual function, written as the `extern "C"` function
/// taking `this` first: `bool IHandler::Process(Message *)` is
/// `unsafe extern "C" fn(*mut IHandler, *mut Message) -> bool`.
///
/// On x86-64, C++ calls a member function like that under both supported ABIs,
/// for the [`HookArg`] and [`HookReturn`] types. Implemented for such functions
/// of up to 16 parameters after `this`. Variadic functions are not supported.
pub trait Signature: Copy + 'static + sealed::Sealed {
	/// The parameters after `this`, as a tuple.
	type Args: Copy + 'static;

	/// The return type.
	type Output: HookReturn;

	/// The class whose member the function is.
	type This: 'static;

	#[doc(hidden)]
	const PARAMETER_SIZES: &'static [usize];

	#[doc(hidden)]
	const THUNKS: Thunks<Self>;

	/// The function at `address`.
	#[doc(hidden)]
	fn from_address(address: NonNull<c_void>) -> Self;

	/// Calls `function`.
	///
	/// # Safety
	///
	/// `function` must be safe to call with these arguments.
	#[doc(hidden)]
	unsafe fn invoke(function: Self, this: *mut Self::This, args: Self::Args) -> Self::Output;

	#[doc(hidden)]
	fn address(self) -> NonNull<c_void>;
}

/// The functions of this library each hooking library calls, for one
/// signature.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct Thunks<S: 'static> {
	pub(super) khook_call_original: S,
	pub(super) khook_make_return: S,
	pub(super) khook_post: S,
	pub(super) khook_pre: S,

	/// The `Call` of this library's delegates, with the delegate as `this`.
	pub(super) sourcehook_call: S,

	/// Each hook manager's hook function.
	pub(super) sourcehook_hook_functions: &'static [S; HOOK_MANAGERS],
}

/// Generic functions of a signature, which take its parameters.
struct Trampolines<S>(PhantomData<S>);

// SAFETY: A function returning `void` returns no value, and `()` has no bytes.
unsafe impl HookReturn for () {}

// SAFETY: C passes and returns pointers as C++ passes and returns pointers and
// references, and null is a value of each.
unsafe impl<T: 'static> HookArg for *const T {}

// SAFETY: As above.
unsafe impl<T: 'static> HookReturn for *const T {}

// SAFETY: As above.
unsafe impl<T: 'static> HookArg for *mut T {}

// SAFETY: As above.
unsafe impl<T: 'static> HookReturn for *mut T {}

// SAFETY: As above.
unsafe impl<T: 'static> HookArg for Option<NonNull<T>> {}

// SAFETY: As above.
unsafe impl<T: 'static> HookReturn for Option<NonNull<T>> {}

// SAFETY: The SDK's `Vector` is `#[repr(C)]`, with the layout of the engine's
// trivially copyable `Vector`, three `float`s, which C++ passes by value as C
// passes the struct.
#[cfg(feature = "sdk")]
unsafe impl HookArg for source_sdk_2013::sys::Vector {}

macro_rules! signatures {
	($(($($arg:ident: $Arg:ident),*))*) => {$(
		impl<T: 'static, R: HookReturn, $($Arg: HookArg),*>
			Trampolines<unsafe extern "C" fn(*mut T $(, $Arg)*) -> R>
		{
			/// KHook's `make_call_original`.
			unsafe extern "C" fn khook_call_original(this: *mut T $(, $arg: $Arg)*) -> R {
				// SAFETY: KHook calls it as the hooked function, of this signature.
				unsafe {
					khook::call_original::<unsafe extern "C" fn(*mut T $(, $Arg)*) -> R>(
						this,
						($($arg,)*),
					)
				}
			}

			/// KHook's `make_return`.
			unsafe extern "C" fn khook_make_return(this: *mut T $(, $arg: $Arg)*) -> R {
				let _ = (this, $($arg),*);

				// SAFETY: KHook calls it last for a call of this signature.
				unsafe { khook::make_return::<R>() }
			}

			/// KHook's `post`.
			unsafe extern "C" fn khook_post(this: *mut T $(, $arg: $Arg)*) -> R {
				// SAFETY: KHook calls it from a hook installed for this signature.
				unsafe {
					khook::post::<unsafe extern "C" fn(*mut T $(, $Arg)*) -> R>(this, ($($arg,)*))
				}
			}

			/// KHook's `pre`.
			unsafe extern "C" fn khook_pre(this: *mut T $(, $arg: $Arg)*) -> R {
				// SAFETY: KHook calls it from a hook installed for this signature.
				unsafe {
					khook::pre::<unsafe extern "C" fn(*mut T $(, $Arg)*) -> R>(this, ($($arg,)*))
				}
			}

			/// A delegate's `Call`, with the delegate as `this`.
			unsafe extern "C" fn sourcehook_call(delegate: *mut T $(, $arg: $Arg)*) -> R {
				// SAFETY: A hook function calls it on one of this library's
				// delegates of this signature.
				unsafe {
					sourcehook::call::<unsafe extern "C" fn(*mut T $(, $Arg)*) -> R>(
						delegate.cast(),
						($($arg,)*),
					)
				}
			}

			/// The hook function of the hook manager `MANAGER`.
			unsafe extern "C" fn sourcehook_hook_function<const MANAGER: usize>(
				this: *mut T
				$(, $arg: $Arg)*
			) -> R {
				// SAFETY: SourceHook patched vtables with it only for a function
				// of this signature, at the hook manager's slot.
				unsafe {
					sourcehook::hook_function::<unsafe extern "C" fn(*mut T $(, $Arg)*) -> R>(
						MANAGER,
						this,
						($($arg,)*),
					)
				}
			}
		}

		impl<T: 'static, R: HookReturn, $($Arg: HookArg),*> sealed::Sealed
			for unsafe extern "C" fn(*mut T $(, $Arg)*) -> R
		{
		}

		impl<T: 'static, R: HookReturn, $($Arg: HookArg),*> Signature
			for unsafe extern "C" fn(*mut T $(, $Arg)*) -> R
		{
			type Args = ($($Arg,)*);
			type Output = R;
			type This = T;

			const PARAMETER_SIZES: &'static [usize] = &[$(size_of::<$Arg>()),*];

			const THUNKS: Thunks<Self> = Thunks {
				khook_call_original: Trampolines::<Self>::khook_call_original,
				khook_make_return: Trampolines::<Self>::khook_make_return,
				khook_post: Trampolines::<Self>::khook_post,
				khook_pre: Trampolines::<Self>::khook_pre,
				sourcehook_call: Trampolines::<Self>::sourcehook_call,
				sourcehook_hook_functions: &hook_managers!(
					Trampolines::<Self>::sourcehook_hook_function
				),
			};

			fn address(self) -> NonNull<c_void> {
				// SAFETY: A function pointer is never null.
				unsafe { NonNull::new_unchecked(self as *mut c_void) }
			}

			fn from_address(address: NonNull<c_void>) -> Self {
				// SAFETY: A function pointer is a non-null address. Calling it is
				// what needs the function to have this signature.
				unsafe { mem::transmute::<*mut c_void, Self>(address.as_ptr()) }
			}

			unsafe fn invoke(function: Self, this: *mut T, args: Self::Args) -> R {
				let ($($arg,)*) = args;

				// SAFETY: As the caller promises.
				unsafe { function(this $(, $arg)*) }
			}
		}
	)*};
}

signatures! {
	()
	(a0: A0)
	(a0: A0, a1: A1)
	(a0: A0, a1: A1, a2: A2)
	(a0: A0, a1: A1, a2: A2, a3: A3)
	(a0: A0, a1: A1, a2: A2, a3: A3, a4: A4)
	(a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5)
	(a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6)
	(a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7)
	(a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7, a8: A8)
	(a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7, a8: A8, a9: A9)
	(a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7, a8: A8, a9: A9, a10: A10)
	(
		a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7, a8: A8, a9: A9, a10: A10,
		a11: A11
	)
	(
		a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7, a8: A8, a9: A9, a10: A10,
		a11: A11, a12: A12
	)
	(
		a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7, a8: A8, a9: A9, a10: A10,
		a11: A11, a12: A12, a13: A13
	)
	(
		a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7, a8: A8, a9: A9, a10: A10,
		a11: A11, a12: A12, a13: A13, a14: A14
	)
	(
		a0: A0, a1: A1, a2: A2, a3: A3, a4: A4, a5: A5, a6: A6, a7: A7, a8: A8, a9: A9, a10: A10,
		a11: A11, a12: A12, a13: A13, a14: A14, a15: A15
	)
}

/// A value for a function to return when its value is unused.
pub(super) fn nothing<R: HookReturn>() -> R {
	// SAFETY: All-zero bytes are a value of every `HookReturn` type.
	unsafe { mem::zeroed() }
}

mod sealed {
	pub trait Sealed {}
}
