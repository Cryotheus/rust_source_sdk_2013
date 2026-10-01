//! Hooks through SourceHook, in Metamod 1.12 build 1226.
//!
//! SourceHook calls a hooked function's delegates from a hook manager's hook
//! function, which it patches the vtable slot with. What a C++ plugin
//! generates with `SH_DECL_MANUALHOOK` is generated here for each
//! [`Signature`]: the hook function, and the delegates' `Call`. SourceHook's
//! `IHookManagerAutoGen` cannot generate them instead, as it refuses every
//! prototype on 64-bit Linux. A hook function only learns its vtable slot from
//! its hook manager, so the plugin has a fixed set of hook managers, each
//! assigned one signature and slot.
//!
//! The hook managers are this library's, and only in use while its hooks are:
//! Metamod removes a plugin's hooks and hook managers when it unloads the
//! plugin, before unloading the library.

use super::signature::{HOOK_MANAGERS, Signature, nothing};
use super::site::{Site, on_main_thread};
use super::{CallBackend, HookCall, HookError, HookTiming, Level};
use crate::MetamodApi;

use crate::sys::sourcehook::{
	AddHookMode, HookManagerPubFunc, IHookContext, IHookManagerInfo, IShDelegate, ISourceHook,
	ISourceHookVtable, MMIFACE_SOURCEHOOK, MetaRes, PassInfo, PassInfoV2, ProtoInfo,
	SH_DELEGATE_CALL_SLOT, SH_HOOKMAN_VERSION, SH_IFACE_VERSION, SH_IMPL_VERSION,
};

use std::any::TypeId;
use std::ffi::{c_int, c_void};
use std::mem::size_of;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicI32, AtomicPtr, Ordering};

const PUBLIC_FUNCTIONS: [HookManagerPubFunc; HOOK_MANAGERS] = hook_managers!(public_function);

static HOOK_MANAGERS_IN_USE: [HookManager; HOOK_MANAGERS] =
	[const { HookManager::unused() }; HOOK_MANAGERS];

/// Metamod's SourceHook, for the hook functions, which run whenever their
/// function is called. It lasts as long as Metamod, so it is never cleared.
static SOURCEHOOK: AtomicPtr<ISourceHook> = AtomicPtr::new(ptr::null_mut());

/// One of this library's delegates, the handler of one of a site's hooks.
#[repr(C)]
struct Delegate<S: Signature> {
	/// Points to `functions`, as no static can be generic.
	vtable: *const DelegateVtable<S>,
	functions: DelegateVtable<S>,
	site: &'static Site<S>,
	timing: HookTiming,
	sourcehook: NonNull<ISourceHook>,
}

impl<S: Signature> Delegate<S> {
	/// A delegate for the site's hooks of `timing`, until SourceHook frees it.
	fn allocate(
		site: &'static Site<S>,
		timing: HookTiming,
		sourcehook: NonNull<ISourceHook>,
	) -> *mut Self {
		let delegate = Box::into_raw(Box::new(Self {
			vtable: ptr::null(),
			functions: DelegateVtable {
				is_equal: is_equal::<S>,
				delete_this: delete_this::<S>,
				call: S::THUNKS.sourcehook_call,
			},
			site,
			timing,
			sourcehook,
		}));

		// SAFETY: The allocation is live, and stays put until freed.
		unsafe { (*delegate).vtable = &raw const (*delegate).functions };

		delegate
	}
}

/// A delegate's vtable: [`crate::sys::sourcehook::IShDelegateVtable`], then
/// `Call` at [`SH_DELEGATE_CALL_SLOT`].
#[repr(C)]
struct DelegateVtable<S: Signature> {
	is_equal: unsafe extern "C" fn(*mut Delegate<S>, *mut IShDelegate) -> bool,
	delete_this: unsafe extern "C" fn(*mut Delegate<S>),
	call: S,
}

/// What a hook manager reports to SourceHook, and its hook function reads.
struct HookManager {
	/// The vtable slot it hooks, or -1 while unused.
	index: AtomicI32,
	/// What SourceHook knows the hook manager by while using it.
	info: AtomicPtr<IHookManagerInfo>,
	proto: AtomicPtr<ProtoInfo>,
	/// The hook function, which SourceHook reads through a pointer to it.
	hook_function: AtomicPtr<c_void>,
}

impl HookManager {
	const fn unused() -> Self {
		Self {
			index: AtomicI32::new(-1),
			info: AtomicPtr::new(ptr::null_mut()),
			proto: AtomicPtr::new(ptr::null_mut()),
			hook_function: AtomicPtr::new(ptr::null_mut()),
		}
	}
}

/// Finds SourceHook, checking it has the interface these declarations are of.
pub(super) fn bind(api: MetamodApi<'_>) -> Result<NonNull<ISourceHook>, HookError> {
	let sourcehook = api
		.meta_interface(MMIFACE_SOURCEHOOK)
		.ok_or(HookError::NotBound)?
		.cast::<ISourceHook>();

	// SAFETY: Metamod's SourceHook lasts as long as Metamod, and its vtable
	// starts with the version methods in every interface version.
	let (interface, implementation) = unsafe {
		let vtable = &*(*sourcehook.as_ptr()).vtable;

		(
			(vtable.get_iface_version)(sourcehook.as_ptr()),
			(vtable.get_impl_version)(sourcehook.as_ptr()),
		)
	};

	if interface != SH_IFACE_VERSION || implementation < SH_IMPL_VERSION {
		return Err(HookError::Unsupported);
	}

	SOURCEHOOK.store(sourcehook.as_ptr(), Ordering::Release);
	Ok(sourcehook)
}

/// A delegate's `Call`: runs the site's handlers for the call, and reports what
/// they decided.
///
/// # Safety
///
/// SourceHook must be calling the delegate, one of `S`, with the call's
/// arguments.
pub(super) unsafe fn call<S: Signature>(delegate: *mut c_void, args: S::Args) -> S::Output {
	if !on_main_thread() {
		return nothing();
	}

	// SAFETY: As the caller promises. SourceHook frees the delegate only after
	// removing its hook.
	let delegate = unsafe { &*delegate.cast::<Delegate<S>>() };

	if !delegate.site.active() {
		return nothing();
	}

	let sourcehook = delegate.sourcehook.as_ptr();

	// SAFETY: SourceHook is running the delegate.
	let vtable = unsafe { &*(*sourcehook).vtable };

	// SAFETY: As above.
	let this = unsafe { (vtable.get_iface_ptr)(sourcehook) };

	let call = HookCall::<S>::new(
		this.cast(),
		args,
		delegate.timing,
		CallBackend::SourceHook(delegate.sourcehook),
	);

	delegate.site.dispatch(&call);

	let outcome = call.outcome();

	// SAFETY: As above. `MRES_*` count up as `Level` does.
	unsafe { (vtable.set_res)(sourcehook, MetaRes(outcome.level as c_int)) };

	match outcome.level {
		Level::Override | Level::Supersede => outcome.value.unwrap_or_else(nothing),
		Level::Ignore | Level::Handled => nothing(),
	}
}

/// Calls each delegate SourceHook gives for one timing, as `SH_CALL_HOOKS`.
///
/// # Safety
///
/// As for [`hook_function`], with the hook function's loop and variables.
unsafe fn call_hooks<S: Signature>(
	context: *mut IHookContext,
	status: *mut MetaRes,
	previous: *mut MetaRes,
	current: *mut MetaRes,
	args: S::Args,
) {
	// SAFETY: The loop lasts until the hook function ends it.
	let context_vtable = unsafe { &*(*context).vtable };

	// SAFETY: The hook function's variables outlive its loop.
	unsafe { previous.write(MetaRes::IGNORED) };

	loop {
		// SAFETY: As above.
		let delegate = unsafe { (context_vtable.get_next)(context) };

		if delegate.is_null() {
			break;
		}

		// SAFETY: SourceHook gives delegates of hooks installed through hook
		// managers of this function's prototype, each with `Call` in this slot.
		let call = unsafe {
			(*delegate)
				.vtable
				.cast::<*mut c_void>()
				.add(SH_DELEGATE_CALL_SLOT)
				.read()
		};

		let Some(call) = NonNull::new(call) else {
			continue;
		};

		// SAFETY: As above.
		unsafe { current.write(MetaRes::IGNORED) };

		// SAFETY: As above, `Call` is a member of the delegate, of the hooked
		// function's signature.
		let returned = unsafe { S::invoke(S::from_address(call), delegate.cast(), args) };

		// SAFETY: The delegate reports its result through the pointer, which
		// SourceHook keeps for the loop.
		let result = unsafe { current.read() };

		// SAFETY: As above.
		unsafe {
			previous.write(result);

			if result > status.read() {
				status.write(result);
			}
		}

		if result >= MetaRes::OVERRIDE && size_of::<S::Output>() != 0 {
			// SAFETY: SourceHook points it at the hook function's override value,
			// or a recalling one's, of the function's return type.
			unsafe {
				(context_vtable.get_override_ret_ptr)(context)
					.cast::<S::Output>()
					.write_unaligned(returned);
			}
		}
	}
}

/// Frees a delegate, as SourceHook does when it removes its hook.
unsafe extern "C" fn delete_this<S: Signature>(delegate: *mut Delegate<S>) {
	// SAFETY: `install` allocated it, and SourceHook frees it once.
	drop(unsafe { Box::from_raw(delegate) });
}

/// The hook function of a hook manager, for each call of a function hooked
/// through it: calls the delegates and the function, as `SH_HANDLEFUNC`.
///
/// # Safety
///
/// SourceHook must have patched the vtable of `this` with it, at the slot of
/// the hook manager, for a function of signature `S`.
pub(super) unsafe fn hook_function<S: Signature>(
	manager: usize,
	this: *mut S::This,
	args: S::Args,
) -> S::Output {
	let manager = &HOOK_MANAGERS_IN_USE[manager];
	let sourcehook = SOURCEHOOK.load(Ordering::Acquire);

	// SAFETY: The hook function was installed through SourceHook, which lasts
	// as long as Metamod.
	let sourcehook_vtable: &ISourceHookVtable = unsafe { &*(*sourcehook).vtable };

	let index = manager.index.load(Ordering::Relaxed);
	let info = manager.info.load(Ordering::Relaxed);
	let returns = size_of::<S::Output>() != 0;

	// SAFETY: `this` is the object the hooked vtable belongs to, which has the
	// hook manager's slot.
	let vfnptr = unsafe {
		this.cast::<*mut *mut c_void>()
			.read()
			.offset(index as isize)
			.cast::<c_void>()
	};

	let mut original_entry: *mut c_void = ptr::null_mut();
	let mut status = MetaRes::IGNORED;
	let mut previous = MetaRes::IGNORED;
	let mut current = MetaRes::IGNORED;
	let mut original_return = nothing::<S::Output>();
	let mut override_return = nothing::<S::Output>();

	// SourceHook keeps these for the loop, writing through them as delegates
	// report, so they are only accessed through these pointers from here on.
	let status = &raw mut status;
	let previous = &raw mut previous;
	let current = &raw mut current;
	let original_return = &raw mut original_return;
	let override_return = &raw mut override_return;

	// SAFETY: The pointers outlive the loop, which `end_context` ends.
	let context = unsafe {
		(sourcehook_vtable.setup_hook_loop)(
			sourcehook,
			info,
			vfnptr,
			this.cast(),
			&mut original_entry,
			status,
			previous,
			current,
			if returns {
				original_return.cast_const().cast()
			} else {
				ptr::null()
			},
			if returns {
				override_return.cast()
			} else {
				ptr::null_mut()
			},
		)
	};

	// SAFETY: The loop lasts until `end_context`.
	let context_vtable = unsafe { &*(*context).vtable };

	// SAFETY: As above.
	unsafe { call_hooks::<S>(context, status, previous, current, args) };

	// SAFETY: As above.
	let call_original = unsafe { status.read() } != MetaRes::SUPERCEDE
		&& unsafe { (context_vtable.should_call_orig)(context) };

	if call_original {
		if let Some(original) = NonNull::new(original_entry) {
			// SAFETY: SourceHook gives the entry the slot held before it was
			// hooked, of signature `S`.
			let value = unsafe { S::invoke(S::from_address(original), this, args) };

			// SAFETY: The variable outlives the loop.
			unsafe { original_return.write(value) };
		}
	} else {
		// SAFETY: As above.
		unsafe { original_return.write(override_return.read()) };
	}

	// SAFETY: As above.
	unsafe { call_hooks::<S>(context, status, previous, current, args) };

	let value = match returns {
		false => nothing(),

		// SAFETY: SourceHook points both at values of the return type, the
		// hook function's own unless the call is a recall.
		true => unsafe {
			let value = if status.read() >= MetaRes::OVERRIDE {
				(context_vtable.get_override_ret_ptr)(context).cast_const()
			} else {
				(context_vtable.get_orig_ret_ptr)(context)
			};

			value.cast::<S::Output>().read_unaligned()
		},
	};

	// SAFETY: The loop is SourceHook's to end.
	unsafe { (sourcehook_vtable.end_context)(sourcehook, context) };

	value
}

/// The hook manager of `S` at a vtable slot, assigned on first use.
pub(super) fn hook_manager<S: Signature>(
	assigned: &mut [Option<(TypeId, c_int)>; HOOK_MANAGERS],
	index: usize,
) -> Result<usize, HookError> {
	let index = c_int::try_from(index).map_err(|_| HookError::InvalidArgument)?;
	let key = Some((TypeId::of::<S>(), index));

	if let Some(manager) = assigned.iter().position(|assignment| *assignment == key) {
		return Ok(manager);
	}

	let manager = assigned
		.iter()
		.position(Option::is_none)
		.ok_or(HookError::TooManyFunctions)?;

	let state = &HOOK_MANAGERS_IN_USE[manager];
	let hook_function = S::THUNKS.sourcehook_hook_functions[manager].address();

	state.proto.store(prototype::<S>(), Ordering::Relaxed);
	state
		.hook_function
		.store(hook_function.as_ptr(), Ordering::Relaxed);
	state.info.store(ptr::null_mut(), Ordering::Relaxed);
	state.index.store(index, Ordering::Relaxed);
	assigned[manager] = key;

	Ok(manager)
}

/// Adds a delegate for the site's handlers of `timing`, on the site's vtable,
/// through the hook manager.
///
/// # Safety
///
/// The site's vtable must hold a function of signature `S` at its slot, which
/// the hook manager is assigned.
pub(super) unsafe fn install<S: Signature>(
	sourcehook: NonNull<ISourceHook>,
	plugin: c_int,
	manager: usize,
	site: &'static Site<S>,
	timing: HookTiming,
) -> Result<(), HookError> {
	let delegate = Delegate::allocate(site, timing, sourcehook);

	// SAFETY: SourceHook lasts as long as Metamod. Calls through the vtable are
	// what the caller promises go to a function of the hook manager's
	// signature, and the hook manager is this library's until Metamod unloads
	// the plugin, as is the delegate until SourceHook frees it.
	let id = unsafe {
		let vtable = &*(*sourcehook.as_ptr()).vtable;

		(vtable.add_hook)(
			sourcehook.as_ptr(),
			plugin,
			AddHookMode::DVP,
			site.vtable().as_ptr().cast(),
			0,
			PUBLIC_FUNCTIONS[manager],
			delegate.cast(),
			timing == HookTiming::Post,
		)
	};

	if id == 0 {
		// SAFETY: SourceHook keeps no delegate of a hook it refused.
		drop(unsafe { Box::from_raw(delegate) });
		return Err(HookError::Refused);
	}

	Ok(())
}

/// Whether `other` is this delegate, for SourceHook's comparisons of a
/// plugin's delegates.
unsafe extern "C" fn is_equal<S: Signature>(
	delegate: *mut Delegate<S>,
	other: *mut IShDelegate,
) -> bool {
	ptr::eq(delegate.cast::<IShDelegate>(), other)
}

/// The function a vtable slot held before SourceHook hooked it.
///
/// # Safety
///
/// `vtable` must be a live vtable with the slot.
pub(super) unsafe fn original(
	sourcehook: NonNull<ISourceHook>,
	vtable: NonNull<*mut c_void>,
	index: c_int,
) -> *mut c_void {
	// SAFETY: As the caller promises.
	let slot = unsafe { vtable.as_ptr().offset(index as isize) };

	// SAFETY: SourceHook lasts as long as Metamod.
	let original = unsafe {
		let functions = &*(*sourcehook.as_ptr()).vtable;

		(functions.get_orig_vfn_ptr_entry)(sourcehook.as_ptr(), slot.cast())
	};

	match original.is_null() {
		// SAFETY: As the caller promises.
		true => unsafe { slot.read() },

		false => original,
	}
}

/// A prototype for `S`, as `SH_DECL_MANUALHOOK` declares one. It is leaked, as
/// hook managers are assigned once per load.
fn prototype<S: Signature>() -> *mut ProtoInfo {
	let parameter = |size| PassInfo {
		size,
		kind: PassInfo::PASS_TYPE_UNKNOWN,
		flags: PassInfo::PASS_FLAG_BY_VAL,
	};

	// The first entry gives the structure's version, which reads the second
	// array of special members.
	let version = PassInfo {
		size: 1,
		kind: 0,
		flags: 0,
	};

	let parameters: &'static [PassInfo] = Vec::leak(
		[version]
			.into_iter()
			.chain(S::PARAMETER_SIZES.iter().copied().map(parameter))
			.collect(),
	);

	let special_members: &'static [PassInfoV2] =
		Vec::leak(vec![PassInfoV2::TRIVIAL; parameters.len()]);

	let returned = match size_of::<S::Output>() {
		0 => PassInfo {
			size: 0,
			kind: PassInfo::PASS_TYPE_UNKNOWN,
			flags: 0,
		},

		size => parameter(size),
	};

	Box::leak(Box::new(ProtoInfo {
		num_of_params: c_int::try_from(S::PARAMETER_SIZES.len()).unwrap_or(c_int::MAX),
		ret_pass_info: returned,
		params_pass_info: parameters.as_ptr(),
		convention: ProtoInfo::CALL_CONV_THIS_CALL,
		ret_pass_info2: PassInfoV2::TRIVIAL,
		params_pass_info2: special_members.as_ptr(),
	}))
}

/// SourceHook's `HookManagerPubFunc` for the hook manager `MANAGER`.
unsafe extern "C" fn public_function<const MANAGER: usize>(
	store: bool,
	info: *mut IHookManagerInfo,
) -> c_int {
	let manager = &HOOK_MANAGERS_IN_USE[MANAGER];
	let index = manager.index.load(Ordering::Relaxed);

	// SourceHook only knows assigned hook managers, but refuses any other.
	if index < 0 {
		return 1;
	}

	if store {
		manager.info.store(info, Ordering::Relaxed);
	}

	if let Some(info) = NonNull::new(info) {
		// SAFETY: SourceHook passes its record of the hook manager, which takes
		// a copy of the prototype, and reads the hook function through the
		// pointer, a static's, whenever it patches a vtable.
		unsafe {
			((*(*info.as_ptr()).vtable).set_info)(
				info.as_ptr(),
				SH_HOOKMAN_VERSION,
				0,
				index,
				manager.proto.load(Ordering::Relaxed),
				manager.hook_function.as_ptr().cast(),
			);
		}
	}

	0
}

/// Forgets the hook managers' assignments, as Metamod removed them when it
/// last unloaded the plugin.
pub(super) fn reset_hook_managers() {
	for manager in &HOOK_MANAGERS_IN_USE {
		manager.index.store(-1, Ordering::Relaxed);
		manager.info.store(ptr::null_mut(), Ordering::Relaxed);
	}
}

/// Where the value the call returns so far is kept, or null.
///
/// # Safety
///
/// SourceHook must be running a delegate.
pub(super) unsafe fn return_value(
	sourcehook: NonNull<ISourceHook>,
	timing: HookTiming,
) -> *const c_void {
	// SAFETY: As the caller promises.
	unsafe {
		let vtable = &*(*sourcehook.as_ptr()).vtable;

		if (vtable.get_status)(sourcehook.as_ptr()) >= MetaRes::OVERRIDE {
			(vtable.get_override_ret)(sourcehook.as_ptr())
		} else if timing == HookTiming::Post {
			(vtable.get_orig_ret)(sourcehook.as_ptr())
		} else {
			ptr::null()
		}
	}
}

/// The greatest result of the call so far.
///
/// # Safety
///
/// SourceHook must be running a delegate.
pub(super) unsafe fn status(sourcehook: NonNull<ISourceHook>) -> MetaRes {
	// SAFETY: As the caller promises.
	unsafe { ((*(*sourcehook.as_ptr()).vtable).get_status)(sourcehook.as_ptr()) }
}
