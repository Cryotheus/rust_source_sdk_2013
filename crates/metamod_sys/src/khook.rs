//! Raw KHook definitions, the dev channel's hooking library, from the
//! `third_party/khook/include/khook.hpp` of 2.0 builds 1469 through 1472.
//!
//! [`IKHook`] declares no virtual destructor and overloads no method, so its
//! methods occupy vtable slots in declaration order under the MSVC and Itanium
//! ABIs alike. The C++ shell asserts each method's type.
//!
//! KHook calls a hook's functions like members of the hooked object, with the
//! hooked call's own arguments. On x86-64, that is a call with `this` as the
//! first argument under both ABIs.

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::mem::{offset_of, size_of};

/// `KHook::HookID_t`.
pub type HookId = u32;

/// Called once KHook has removed a hook: the `hook_removal_fn` of
/// [`IKHookVtable::remove_hook`].
pub type HookRemovalFn = unsafe extern "C" fn(id: HookId, context: *mut c_void);

const _: () = {
	const SLOT: usize = size_of::<*const ()>();

	macro_rules! assert_slot {
		($field:ident, $slot:expr) => {
			assert!(offset_of!(IKHookVtable, $field) == $slot * SLOT);
		};
	}

	assert!(size_of::<Action>() == 1);
	assert!(size_of::<IKHook>() == SLOT);

	assert_slot!(setup_hook, 0);
	assert_slot!(setup_virtual_hook, 1);
	assert_slot!(remove_hook, 2);
	assert_slot!(get_context_ptr, 3);
	assert_slot!(get_original_function, 4);
	assert_slot!(get_original_value_ptr, 5);
	assert_slot!(get_override_value_ptr, 6);
	assert_slot!(get_current_value_ptr, 7);
	assert_slot!(destroy_return_value, 8);
	assert_slot!(find_original, 9);
	assert_slot!(find_original_virtual, 10);
	assert_slot!(do_recall, 11);
	assert_slot!(save_return_value, 12);
	assert_slot!(lookup_signature, 13);
	assert_slot!(was_original_function_skipped, 14);
	assert!(size_of::<IKHookVtable>() == 15 * SLOT);
};

/// Returned by [`IKHookVtable::setup_hook`] and
/// [`IKHookVtable::setup_virtual_hook`] for a refused hook.
pub const INVALID_HOOK: HookId = HookId::MAX;

/// `KHook::Action`, what a hook did. Greater values take precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Action(pub u8);

impl Action {
	/// The hook took no action.
	pub const IGNORE: Self = Self(0);

	/// The function runs, but returns the hook's value.
	pub const OVERRIDE: Self = Self(1);

	/// The function is skipped, returning the hook's value.
	pub const SUPERSEDE: Self = Self(2);
}

/// A C++ `KHook::IKHook` object, which Metamod's
/// `ISmmAPI::GetDetourInterface` gives each plugin.
#[repr(C)]
pub struct IKHook {
	pub vtable: *const IKHookVtable,
}

/// The functions given to the `setup_*` methods are KHook's calls into the
/// hook. `pre` and `post` may be null. Of every hook on a function, KHook uses
/// the `make_return` and `make_call_original` of the first in its list.
///
/// - `pre` and `post` run before and after the function, and report what they
///   did through [`Self::save_return_value`]. What they return is unused.
/// - `make_call_original` calls [`Self::get_original_function`] and saves its
///   return value with `original` set.
/// - `make_return` reads [`Self::get_current_value_ptr`] with `pop` set and
///   returns it after [`Self::destroy_return_value`].
///
/// `stack_size` is how many bytes of the caller's stack each of them receives
/// a copy of. `async_` adds the hook from KHook's worker thread, unless it is
/// the first on the function, as a detour may be running.
#[repr(C)]
pub struct IKHookVtable {
	pub setup_hook: unsafe extern "C" fn(
		*mut IKHook,
		function: *mut c_void,
		context: *mut c_void,
		removed_function: *mut c_void,
		pre: *mut c_void,
		post: *mut c_void,
		make_return: *mut c_void,
		make_call_original: *mut c_void,
		stack_size: c_uint,
		async_: bool,
	) -> HookId,

	pub setup_virtual_hook: unsafe extern "C" fn(
		*mut IKHook,
		vtable: *mut *mut c_void,
		index: c_int,
		context: *mut c_void,
		removed_function: *mut c_void,
		pre: *mut c_void,
		post: *mut c_void,
		make_return: *mut c_void,
		make_call_original: *mut c_void,
		stack_size: c_uint,
		async_: bool,
	) -> HookId,

	pub remove_hook: unsafe extern "C" fn(
		*mut IKHook,
		id: HookId,
		async_: bool,
		hook_removal_fn: Option<HookRemovalFn>,
		context: *mut c_void,
	),

	/// The `context` of the hook KHook is calling.
	pub get_context_ptr: unsafe extern "C" fn(*mut IKHook) -> *mut c_void,

	pub get_original_function: unsafe extern "C" fn(*mut IKHook) -> *mut c_void,

	/// The function's return value, or null if none was saved.
	pub get_original_value_ptr: unsafe extern "C" fn(*mut IKHook) -> *mut c_void,

	/// The overriding return value, or null if none was saved.
	pub get_override_value_ptr: unsafe extern "C" fn(*mut IKHook) -> *mut c_void,

	/// The overriding return value if a hook overrode the call, or else the
	/// function's own. `pop` reads the call that just finished, for
	/// `make_return`.
	pub get_current_value_ptr: unsafe extern "C" fn(*mut IKHook, pop: bool) -> *mut c_void,

	pub destroy_return_value: unsafe extern "C" fn(*mut IKHook),

	/// The original of a detoured function, or `function` itself.
	pub find_original: unsafe extern "C" fn(*mut IKHook, function: *mut c_void) -> *mut c_void,

	/// The original of a hooked vtable entry, or the entry itself.
	pub find_original_virtual:
		unsafe extern "C" fn(*mut IKHook, vtable: *mut *mut c_void, index: c_int) -> *mut c_void,

	pub do_recall: unsafe extern "C" fn(
		*mut IKHook,
		action: Action,
		ptr_to_return: *mut c_void,
		return_size: usize,
		init_op: *mut c_void,
		deinit_op: *mut c_void,
	) -> *mut c_void,

	/// Records what the hook did, keeping its value if `action` exceeds every
	/// earlier hook's. `init_op(destination, value)` copies the value into
	/// KHook's storage, and `deinit_op(value)` destroys it. Both are null, and
	/// `return_size` 0, for a function returning `void`.
	pub save_return_value: unsafe extern "C" fn(
		*mut IKHook,
		action: Action,
		ptr_to_return: *mut c_void,
		return_size: usize,
		init_op: *mut c_void,
		deinit_op: *mut c_void,
		original: bool,
	),

	pub lookup_signature: unsafe extern "C" fn(
		*mut IKHook,
		start: *mut c_void,
		size: usize,
		signature: *const c_char,
	) -> *mut c_void,

	/// Whether a hook superseded the call, valid in a post hook.
	pub was_original_function_skipped: unsafe extern "C" fn(*mut IKHook) -> bool,
}
