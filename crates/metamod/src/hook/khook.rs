//! Hooks through KHook, in Metamod 2.0 builds 1469 through 1472.
//!
//! KHook calls a hook's functions as the hooked function, with its arguments,
//! and finds the hook's site through the context it was installed with.
//!
//! Metamod gives each plugin an `IKHook`, which it frees right after the
//! plugin's `Unload`, and removes the plugin's hooks later, while they can
//! still run. Its implementation forwards each call to KHook's globals and
//! ignores `this`, so its vtable is kept, and called with the latest object.
//!
//! Metamod unloads a library once KHook reports the removal of each hook its
//! plugin installed. A plugin removing a hook itself would keep Metamod
//! waiting for a report that never comes, so hooks are never removed here.

use super::signature::{HookReturn, Signature, nothing};
use super::site::{Site, on_main_thread};
use super::{CallBackend, HookCall, HookError, HookTiming, Level};
use crate::MetamodApi;
use crate::sys::khook::{Action, IKHook, IKHookVtable, INVALID_HOOK};
use std::ffi::{c_int, c_uint, c_void};
use std::mem::size_of;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicPtr, Ordering};

/// The latest `IKHook` Metamod gave the plugin.
static KHOOK: AtomicPtr<IKHook> = AtomicPtr::new(ptr::null_mut());

/// The vtable of every `IKHook` Metamod gives, which lasts as long as Metamod.
static KHOOK_VTABLE: AtomicPtr<IKHookVtable> = AtomicPtr::new(ptr::null_mut());

/// Finds the plugin's `IKHook`.
pub(super) fn bind(api: MetamodApi<'_>, plugin: c_int) -> Result<(), HookError> {
	let khook = api
		.detour_interface(plugin)
		.map_err(|_| HookError::Unsupported)?
		.ok_or(HookError::NotBound)?
		.cast::<IKHook>();

	// SAFETY: Metamod keeps the interface until the plugin's `Unload`.
	let vtable = unsafe { (*khook.as_ptr()).vtable };

	if vtable.is_null() {
		return Err(HookError::NotBound);
	}

	KHOOK_VTABLE.store(vtable.cast_mut(), Ordering::Release);
	KHOOK.store(khook.as_ptr(), Ordering::Release);

	Ok(())
}

/// KHook's `make_call_original`: calls the hooked function, and saves what it
/// returned.
///
/// # Safety
///
/// KHook must be calling it for a call of signature `S`, with its arguments.
pub(super) unsafe fn call_original<S: Signature>(this: *mut S::This, args: S::Args) -> S::Output {
	let (khook, vtable) = interface();

	// SAFETY: As the caller promises.
	let original = unsafe { (vtable.get_original_function)(khook) };

	let value = match NonNull::new(original) {
		// SAFETY: The original function has the hooked function's signature.
		Some(original) => unsafe { S::invoke(S::from_address(original), this, args) },

		None => nothing(),
	};

	// SAFETY: As the caller promises.
	unsafe { save(Action::IGNORE, Some(value), true) };

	value
}

/// Runs the site's handlers for a call, and reports what they decided.
///
/// # Safety
///
/// KHook must be calling a hook of a site of `S`'s, with the call's arguments.
unsafe fn callback<S: Signature>(
	timing: HookTiming,
	this: *mut S::This,
	args: S::Args,
) -> S::Output {
	if !on_main_thread() {
		return nothing();
	}

	let (khook, vtable) = interface();

	// SAFETY: As the caller promises, KHook is running a hook, whose context
	// `install` set to its site, which is never freed.
	let site = unsafe { &*(vtable.get_context_ptr)(khook).cast::<Site<S>>() };

	if !site.active() {
		return nothing();
	}

	let call = HookCall::<S>::new(this, args, timing, CallBackend::KHook);

	site.dispatch(&call);

	let outcome = call.outcome();

	let action = match outcome.level {
		Level::Ignore | Level::Handled => return nothing(),
		Level::Override => Action::OVERRIDE,
		Level::Supersede => Action::SUPERSEDE,
	};

	match (size_of::<S::Output>(), outcome.value) {
		// SAFETY: KHook is running the hook.
		(0, _) => unsafe { save::<S::Output>(action, None, false) },

		// SAFETY: As above.
		(_, Some(value)) => unsafe { save(action, Some(value), false) },

		(_, None) => {}
	}

	nothing()
}

/// KHook's `init_op`: copies a return value into KHook's storage.
unsafe extern "C" fn copy<R: HookReturn>(destination: *mut R, value: *const R) {
	// SAFETY: KHook allocates the storage at the size it was given.
	unsafe { destination.write_unaligned(value.read_unaligned()) };
}

/// KHook's `deinit_op`: return values need no destruction.
unsafe extern "C" fn destroy<R: HookReturn>(_value: *mut R) {}

/// Adds a hook running the site's handlers of `timing`.
///
/// # Safety
///
/// The site's vtable must hold a function of signature `S` at its slot.
pub(super) unsafe fn install<S: Signature>(
	site: &'static Site<S>,
	timing: HookTiming,
) -> Result<(), HookError> {
	let index = c_int::try_from(site.index()).map_err(|_| HookError::InvalidArgument)?;
	let (khook, vtable) = interface();
	let thunks = S::THUNKS;

	let (pre, post) = match timing {
		HookTiming::Pre => (thunks.khook_pre.address().as_ptr(), ptr::null_mut()),
		HookTiming::Post => (ptr::null_mut(), thunks.khook_post.address().as_ptr()),
	};

	// SAFETY: As the caller promises, calls through the slot go to a function of
	// the hook's signature. The site, which is never freed, is the context, and
	// the hook only runs before Metamod unloads the library.
	let id = unsafe {
		(vtable.setup_virtual_hook)(
			khook,
			site.vtable().as_ptr(),
			index,
			ptr::from_ref(site).cast_mut().cast(),
			ptr::null_mut(),
			pre,
			post,
			thunks.khook_make_return.address().as_ptr(),
			thunks.khook_call_original.address().as_ptr(),
			stack_size::<S>(),
			// As `KHook::Virtual` does, as a detour may be running.
			true,
		)
	};

	match id {
		INVALID_HOOK => Err(HookError::Refused),
		_ => Ok(()),
	}
}

/// The plugin's latest `IKHook`, and the vtable of them all.
fn interface() -> (*mut IKHook, &'static IKHookVtable) {
	let vtable = KHOOK_VTABLE.load(Ordering::Acquire);

	// SAFETY: Hooks are installed after `bind` stored it, and Metamod keeps its
	// vtable while it can call them.
	(KHOOK.load(Ordering::Acquire), unsafe { &*vtable })
}

/// KHook's `make_return`: the value the call returns, once KHook is done.
///
/// # Safety
///
/// KHook must be calling it last for a call returning `R`.
pub(super) unsafe fn make_return<R: HookReturn>() -> R {
	let (khook, vtable) = interface();

	let value = match size_of::<R>() {
		0 => nothing(),

		// SAFETY: As the caller promises, KHook keeps the value the call
		// returns, of the type `R`, until it is destroyed.
		_ => unsafe {
			let value = (vtable.get_current_value_ptr)(khook, true).cast::<R>();

			match value.is_null() {
				true => nothing(),
				false => value.read_unaligned(),
			}
		},
	};

	// SAFETY: As the caller promises.
	unsafe { (vtable.destroy_return_value)(khook) };

	value
}

/// The function a vtable slot held before KHook hooked it.
///
/// # Safety
///
/// `vtable` must be a live vtable with the slot.
pub(super) unsafe fn original(vtable: NonNull<*mut c_void>, index: c_int) -> *mut c_void {
	let (khook, functions) = interface();

	// SAFETY: As the caller promises.
	unsafe { (functions.find_original_virtual)(khook, vtable.as_ptr(), index) }
}

/// KHook's `post`.
///
/// # Safety
///
/// As for [`callback`].
pub(super) unsafe fn post<S: Signature>(this: *mut S::This, args: S::Args) -> S::Output {
	// SAFETY: As the caller promises.
	unsafe { callback::<S>(HookTiming::Post, this, args) }
}

/// KHook's `pre`.
///
/// # Safety
///
/// As for [`callback`].
pub(super) unsafe fn pre<S: Signature>(this: *mut S::This, args: S::Args) -> S::Output {
	// SAFETY: As the caller promises.
	unsafe { callback::<S>(HookTiming::Pre, this, args) }
}

/// Where the value the call returns so far is kept, or null.
///
/// # Safety
///
/// KHook must be running a hook.
pub(super) unsafe fn return_value() -> *const c_void {
	let (khook, vtable) = interface();

	// SAFETY: As the caller promises.
	unsafe { (vtable.get_current_value_ptr)(khook, false) }.cast_const()
}

/// Reports what a hook did, with its value, if the function returns one.
///
/// # Safety
///
/// KHook must be running a hook of a function returning `R`, or its
/// `make_call_original` if `original`.
unsafe fn save<R: HookReturn>(action: Action, value: Option<R>, original: bool) {
	let (khook, vtable) = interface();
	let mut value = value.filter(|_| size_of::<R>() != 0);

	let (pointer, size, copy, destroy) = match &mut value {
		Some(value) => (
			ptr::from_mut(value).cast(),
			size_of::<R>(),
			copy::<R> as unsafe extern "C" fn(*mut R, *const R) as *mut c_void,
			destroy::<R> as unsafe extern "C" fn(*mut R) as *mut c_void,
		),

		None => (ptr::null_mut(), 0, ptr::null_mut(), ptr::null_mut()),
	};

	// SAFETY: As the caller promises. KHook copies the value before returning.
	unsafe { (vtable.save_return_value)(khook, action, pointer, size, copy, destroy, original) };
}

/// How much of the caller's stack KHook copies for each call into a hook, as
/// `KHook::Hook::_copy_stack_size` computes it, or more.
fn stack_size<S: Signature>() -> c_uint {
	let returned = size_of::<S::Output>();

	let size = if cfg!(windows) {
		// Space for the four arguments passed in registers, and a slot for each
		// other one. A returned value may be passed as the address to store it.
		let arguments = 1 + S::PARAMETER_SIZES.len() + usize::from(returned != 0);

		32 + 8 * arguments.saturating_sub(4)
	} else {
		// A slot or more for every argument, wherever it is passed, and the
		// returned value.
		let slots = |size: usize| size.next_multiple_of(8).max(8);
		let returned = if returned != 0 { slots(returned) } else { 0 };

		returned
			+ 8
			+ S::PARAMETER_SIZES
				.iter()
				.map(|&size| slots(size))
				.sum::<usize>()
	};

	c_uint::try_from(size).unwrap_or(c_uint::MAX)
}
