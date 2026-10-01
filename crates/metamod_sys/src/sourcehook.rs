//! Raw SourceHook definitions, the stable channel's hooking library, from the
//! 1.12 build 1226 `core/sourcehook/sourcehook.h`.
//!
//! None of these interfaces declares a virtual destructor or overloads a
//! method, so their methods occupy vtable slots in declaration order under the
//! MSVC and Itanium ABIs alike. The C++ shell asserts each method's type.

use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::mem::{align_of, offset_of, size_of};

/// `SourceHook::HookManagerPubFunc`: a hook manager's public function.
///
/// SourceHook calls it with `store` false to learn the hook manager's details,
/// which it reports through [`IHookManagerInfoVtable::set_info`] and returns
/// 0, or refuses with any other value. With `store` true, the hook manager
/// keeps `info` for its hook function until called again, with `info` null
/// once it is no longer used.
pub type HookManagerPubFunc =
	unsafe extern "C" fn(store: bool, info: *mut IHookManagerInfo) -> c_int;

/// `SourceHook::Plugin`, which Metamod gives the value of the plugin's
/// `PluginId`.
pub type Plugin = c_int;

const _: () = {
	const SLOT: usize = size_of::<*const ()>();

	macro_rules! assert_slot {
		($ty:ty, $field:ident, $slot:expr) => {
			assert!(offset_of!($ty, $field) == $slot * SLOT);
		};
	}

	assert!(size_of::<MetaRes>() == size_of::<c_int>());
	assert!(size_of::<AddHookMode>() == size_of::<c_int>());

	assert!(offset_of!(PassInfo, size) == 0);
	assert!(offset_of!(PassInfo, kind) == 8);
	assert!(offset_of!(PassInfo, flags) == 12);
	assert!(size_of::<PassInfo>() == 16);

	assert_slot!(PassInfoV2, normal_ctor, 0);
	assert_slot!(PassInfoV2, copy_ctor, 1);
	assert_slot!(PassInfoV2, dtor, 2);
	assert_slot!(PassInfoV2, assign_operator, 3);
	assert!(size_of::<PassInfoV2>() == 4 * SLOT);

	assert!(offset_of!(ProtoInfo, num_of_params) == 0);
	assert!(offset_of!(ProtoInfo, ret_pass_info) == 8);
	assert!(offset_of!(ProtoInfo, params_pass_info) == 24);
	assert!(offset_of!(ProtoInfo, convention) == 32);
	assert!(offset_of!(ProtoInfo, ret_pass_info2) == 40);
	assert!(offset_of!(ProtoInfo, params_pass_info2) == 72);
	assert!(size_of::<ProtoInfo>() == 80);
	assert!(align_of::<ProtoInfo>() == SLOT);

	assert_slot!(ISourceHookVtable, get_iface_version, 0);
	assert_slot!(ISourceHookVtable, get_impl_version, 1);
	assert_slot!(ISourceHookVtable, add_hook, 2);
	assert_slot!(ISourceHookVtable, remove_hook, 3);
	assert_slot!(ISourceHookVtable, remove_hook_by_id, 4);
	assert_slot!(ISourceHookVtable, pause_hook_by_id, 5);
	assert_slot!(ISourceHookVtable, unpause_hook_by_id, 6);
	assert_slot!(ISourceHookVtable, set_res, 7);
	assert_slot!(ISourceHookVtable, get_prev_res, 8);
	assert_slot!(ISourceHookVtable, get_status, 9);
	assert_slot!(ISourceHookVtable, get_orig_ret, 10);
	assert_slot!(ISourceHookVtable, get_override_ret, 11);
	assert_slot!(ISourceHookVtable, get_iface_ptr, 12);
	assert_slot!(ISourceHookVtable, get_override_ret_ptr, 13);
	assert_slot!(ISourceHookVtable, remove_hook_manager, 14);
	assert_slot!(ISourceHookVtable, set_ignore_hooks, 15);
	assert_slot!(ISourceHookVtable, reset_ignore_hooks, 16);
	assert_slot!(ISourceHookVtable, get_orig_vfn_ptr_entry, 17);
	assert_slot!(ISourceHookVtable, do_recall, 18);
	assert_slot!(ISourceHookVtable, setup_hook_loop, 19);
	assert_slot!(ISourceHookVtable, end_context, 20);
	assert_slot!(ISourceHookVtable, log_debug, 21);
	assert!(size_of::<ISourceHookVtable>() == 22 * SLOT);

	assert_slot!(IHookContextVtable, get_next, 0);
	assert_slot!(IHookContextVtable, get_override_ret_ptr, 1);
	assert_slot!(IHookContextVtable, get_orig_ret_ptr, 2);
	assert_slot!(IHookContextVtable, should_call_orig, 3);
	assert!(size_of::<IHookContextVtable>() == 4 * SLOT);

	assert_slot!(IShDelegateVtable, is_equal, 0);
	assert_slot!(IShDelegateVtable, delete_this, 1);
	assert!(size_of::<IShDelegateVtable>() == SH_DELEGATE_CALL_SLOT * SLOT);

	assert_slot!(IHookManagerInfoVtable, set_info, 0);
	assert!(size_of::<IHookManagerInfoVtable>() == SLOT);

	assert!(size_of::<ISourceHook>() == SLOT);
	assert!(size_of::<IHookContext>() == SLOT);
	assert!(size_of::<IShDelegate>() == SLOT);
	assert!(size_of::<IHookManagerInfo>() == SLOT);
};

/// The name `ISmmAPI::MetaFactory` gives SourceHook by: `MMIFACE_SOURCEHOOK`.
pub const MMIFACE_SOURCEHOOK: &CStr = c"ISourceHook";

/// The slot of `Call` in a delegate's vtable, after [`IShDelegateVtable`]'s.
/// Its type is the hooked function's, with the delegate as `this`.
pub const SH_DELEGATE_CALL_SLOT: usize = 2;

/// `SH_HOOKMAN_VERSION`, which hook managers pass to
/// [`IHookManagerInfoVtable::set_info`].
pub const SH_HOOKMAN_VERSION: c_int = 1;

/// `SH_IFACE_VERSION`, which [`ISourceHookVtable::get_iface_version`] must
/// return exactly for these declarations to apply.
pub const SH_IFACE_VERSION: c_int = 5;

/// `SH_IMPL_VERSION`, the least [`ISourceHookVtable::get_impl_version`] these
/// declarations support.
pub const SH_IMPL_VERSION: c_int = 5;

/// `ISourceHook::AddHookMode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct AddHookMode(pub c_int);

impl AddHookMode {
	/// `Hook_DVP`: calls through the vtable given as the interface pointer.
	pub const DVP: Self = Self(2);

	/// `Hook_Normal`: calls on the given interface only.
	pub const NORMAL: Self = Self(0);

	/// `Hook_VP`: calls on any object sharing the given interface's vtable.
	pub const VP: Self = Self(1);
}

/// A C++ `SourceHook::IHookContext` object: one hook function's loop.
#[repr(C)]
pub struct IHookContext {
	pub vtable: *const IHookContextVtable,
}

#[repr(C)]
pub struct IHookContextVtable {
	/// The next delegate to call, or null at the end of the pre or post hooks.
	pub get_next: unsafe extern "C" fn(*mut IHookContext) -> *mut IShDelegate,
	pub get_override_ret_ptr: unsafe extern "C" fn(*mut IHookContext) -> *mut c_void,
	pub get_orig_ret_ptr: unsafe extern "C" fn(*mut IHookContext) -> *const c_void,
	pub should_call_orig: unsafe extern "C" fn(*mut IHookContext) -> bool,
}

/// A C++ `SourceHook::IHookManagerInfo` object, which SourceHook passes to a
/// hook manager's [`HookManagerPubFunc`].
#[repr(C)]
pub struct IHookManagerInfo {
	pub vtable: *const IHookManagerInfoVtable,
}

#[repr(C)]
pub struct IHookManagerInfoVtable {
	/// `hook_function` points to where the hook function's address is kept,
	/// which SourceHook reads whenever it patches a vtable with it.
	pub set_info: unsafe extern "C" fn(
		*mut IHookManagerInfo,
		hook_manager_version: c_int,
		vtbl_offs: c_int,
		vtbl_idx: c_int,
		proto: *mut ProtoInfo,
		hook_function: *mut c_void,
	),
}

/// A C++ `SourceHook::ISHDelegate` object: one hook's handler.
#[repr(C)]
pub struct IShDelegate {
	pub vtable: *const IShDelegateVtable,
}

/// The slots every delegate shares, before `Call` at
/// [`SH_DELEGATE_CALL_SLOT`].
#[repr(C)]
pub struct IShDelegateVtable {
	/// Whether `other`, a delegate of the same plugin and hook manager, is
	/// equivalent.
	pub is_equal: unsafe extern "C" fn(*mut IShDelegate, other: *mut IShDelegate) -> bool,

	/// Frees the delegate, which SourceHook does as it removes its hook.
	pub delete_this: unsafe extern "C" fn(*mut IShDelegate),
}

/// A C++ `SourceHook::ISourceHook` object, owned by Metamod.
#[repr(C)]
pub struct ISourceHook {
	pub vtable: *const ISourceHookVtable,
}

#[repr(C)]
pub struct ISourceHookVtable {
	pub get_iface_version: unsafe extern "C" fn(*mut ISourceHook) -> c_int,
	pub get_impl_version: unsafe extern "C" fn(*mut ISourceHook) -> c_int,

	/// Returns the hook's ID, or 0 if refused.
	pub add_hook: unsafe extern "C" fn(
		*mut ISourceHook,
		plugin: Plugin,
		mode: AddHookMode,
		iface: *mut c_void,
		thisptr_offs: c_int,
		hook_manager: HookManagerPubFunc,
		handler: *mut IShDelegate,
		post: bool,
	) -> c_int,

	pub remove_hook: unsafe extern "C" fn(
		*mut ISourceHook,
		plugin: Plugin,
		iface: *mut c_void,
		thisptr_offs: c_int,
		hook_manager: HookManagerPubFunc,
		handler: *mut IShDelegate,
		post: bool,
	) -> bool,

	pub remove_hook_by_id: unsafe extern "C" fn(*mut ISourceHook, hook_id: c_int) -> bool,
	pub pause_hook_by_id: unsafe extern "C" fn(*mut ISourceHook, hook_id: c_int) -> bool,
	pub unpause_hook_by_id: unsafe extern "C" fn(*mut ISourceHook, hook_id: c_int) -> bool,

	/// Sets the result of the delegate SourceHook is calling.
	pub set_res: unsafe extern "C" fn(*mut ISourceHook, res: MetaRes),

	/// The result of the delegate called before the current one.
	pub get_prev_res: unsafe extern "C" fn(*mut ISourceHook) -> MetaRes,

	/// The greatest result of the call so far.
	pub get_status: unsafe extern "C" fn(*mut ISourceHook) -> MetaRes,

	/// Where the function's return value is kept, defined in post hooks.
	pub get_orig_ret: unsafe extern "C" fn(*mut ISourceHook) -> *const c_void,

	/// Where the overriding return value is kept, or null without one.
	pub get_override_ret: unsafe extern "C" fn(*mut ISourceHook) -> *const c_void,

	/// The `this` of the hooked call.
	pub get_iface_ptr: unsafe extern "C" fn(*mut ISourceHook) -> *mut c_void,

	pub get_override_ret_ptr: unsafe extern "C" fn(*mut ISourceHook) -> *mut c_void,
	pub remove_hook_manager:
		unsafe extern "C" fn(*mut ISourceHook, plugin: Plugin, hook_manager: HookManagerPubFunc),
	pub set_ignore_hooks: unsafe extern "C" fn(*mut ISourceHook, vfnptr: *mut c_void),
	pub reset_ignore_hooks: unsafe extern "C" fn(*mut ISourceHook, vfnptr: *mut c_void),

	/// The entry a hooked vtable slot held before SourceHook patched it, or
	/// null if it is not hooked.
	pub get_orig_vfn_ptr_entry:
		unsafe extern "C" fn(*mut ISourceHook, vfnptr: *mut c_void) -> *mut c_void,

	pub do_recall: unsafe extern "C" fn(*mut ISourceHook),

	/// Starts a hook function's loop over the delegates of `vfnptr`, the vtable
	/// slot the call came through. The pointers must stay valid until
	/// [`Self::end_context`], and both return pointers are null for a `void`
	/// function.
	pub setup_hook_loop: unsafe extern "C" fn(
		*mut ISourceHook,
		info: *mut IHookManagerInfo,
		vfnptr: *mut c_void,
		thisptr: *mut c_void,
		orig_call_addr: *mut *mut c_void,
		status: *mut MetaRes,
		prev_res: *mut MetaRes,
		cur_res: *mut MetaRes,
		orig_ret: *const c_void,
		override_ret: *mut c_void,
	) -> *mut IHookContext,

	pub end_context: unsafe extern "C" fn(*mut ISourceHook, context: *mut IHookContext),
	pub log_debug: unsafe extern "C" fn(*mut ISourceHook, format: *const c_char, ...),
}

/// `META_RES`, what a hook did. Greater values take precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct MetaRes(pub c_int);

impl MetaRes {
	/// `MRES_HANDLED`: the hook did something, but the function still runs.
	pub const HANDLED: Self = Self(1);

	/// `MRES_IGNORED`: the hook took no action.
	pub const IGNORED: Self = Self(0);

	/// `MRES_OVERRIDE`: the function runs, but returns the hook's value.
	pub const OVERRIDE: Self = Self(2);

	/// `MRES_SUPERCEDE`: the function is skipped, returning the hook's value.
	pub const SUPERCEDE: Self = Self(3);
}

/// `SourceHook::PassInfo`: how a parameter or return value is passed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct PassInfo {
	pub size: usize,
	/// One of the `PASS_TYPE_*` values: `type` in C++.
	pub kind: c_int,
	/// A combination of the `PASS_FLAG_*` values.
	pub flags: c_uint,
}

impl PassInfo {
	pub const PASS_FLAG_ASSIGN_OP: c_uint = 1 << 4;
	pub const PASS_FLAG_BY_REF: c_uint = 1 << 1;
	pub const PASS_FLAG_BY_VAL: c_uint = 1 << 0;
	pub const PASS_FLAG_C_CTOR: c_uint = 1 << 5;
	pub const PASS_FLAG_O_CTOR: c_uint = 1 << 3;
	pub const PASS_FLAG_O_DTOR: c_uint = 1 << 2;
	pub const PASS_FLAG_RET_MEM: c_uint = 1 << 6;
	pub const PASS_FLAG_RET_REG: c_uint = 1 << 7;
	pub const PASS_TYPE_BASIC: c_int = 1;
	pub const PASS_TYPE_FLOAT: c_int = 2;
	pub const PASS_TYPE_OBJECT: c_int = 3;
	pub const PASS_TYPE_UNKNOWN: c_int = 0;
}

/// `SourceHook::PassInfo::V2Info`: the special members of an object passed by
/// value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct PassInfoV2 {
	pub normal_ctor: *mut c_void,
	pub copy_ctor: *mut c_void,
	pub dtor: *mut c_void,
	pub assign_operator: *mut c_void,
}

impl PassInfoV2 {
	/// For a type without special members.
	pub const TRIVIAL: Self = Self {
		normal_ctor: std::ptr::null_mut(),
		copy_ctor: std::ptr::null_mut(),
		dtor: std::ptr::null_mut(),
		assign_operator: std::ptr::null_mut(),
	};
}

/// `SourceHook::ProtoInfo`: a hooked function's prototype.
///
/// `params_pass_info[0]` and `params_pass_info2[0]` precede the parameters:
/// the former's `size` gives the version of the structure, 1 for this layout.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct ProtoInfo {
	pub num_of_params: c_int,
	/// Has `size` 0 for a function returning `void`.
	pub ret_pass_info: PassInfo,
	pub params_pass_info: *const PassInfo,
	/// One of the `CALL_CONV_*` values.
	pub convention: c_int,
	pub ret_pass_info2: PassInfoV2,
	pub params_pass_info2: *const PassInfoV2,
}

impl ProtoInfo {
	pub const CALL_CONV_CDECL: c_int = 2;
	pub const CALL_CONV_HAS_VAFMT: c_int = Self::CALL_CONV_HAS_VAR_ARGS | (1 << 17);
	pub const CALL_CONV_HAS_VAR_ARGS: c_int = 1 << 16;
	pub const CALL_CONV_STD_CALL: c_int = 3;
	pub const CALL_CONV_THIS_CALL: c_int = 1;
	pub const CALL_CONV_UNKNOWN: c_int = 0;
}
