//! Hooks through KHook, in Metamod 2.0 builds 1469 through 1490.
//!
//! KHook calls a hook's functions as the hooked function, with its arguments,
//! and finds the hook's site through the context it was installed with.
//!
//! Metamod gives each plugin an `IKHook`, which it frees right after the
//! plugin's `Unload`, and removes the plugin's hooks later, while they can
//! still run. Its implementation forwards each call to KHook's globals and
//! ignores `this`, so its vtable is kept, and called with the latest object.
//!
//! Metamod keeps the ID of each hook a plugin installs through its `IKHook`,
//! and as it unloads the plugin, asks KHook to remove each, and unloads the
//! library once KHook reported every removal. KHook keeps a slot's detour
//! after its last hook leaves, and the detour then goes straight to the
//! original function, for much less than a call through a hook.
//!
//! A plugin removing a hook on a vtable slot itself leaves its ID with
//! Metamod, which asks for its removal again at the unload. Before build 1475,
//! KHook never reported the removal of a hook it no longer had, so Metamod
//! would wait for ever and never unload the library. From 1475, it reports it
//! at once, but until 1489, Metamod's unload removed IDs from the set it was
//! walking through, which could skip hooks, leave the library loaded, or
//! crash. Build 1490 walks a copy. So hooks on vtable slots are only removed
//! here under a Metamod that reports itself as a build known to cope (see
//! [`releases_slot_hooks`]); under any other, they stay until the unload.
//!
//! The removal is asynchronous: KHook's own thread removes the hook once no
//! call of the function runs, while the main thread goes on. A synchronous
//! removal would wait for every running call of the function to end, for ever
//! if one is running further up the main thread's own stack. Until the hook is
//! gone, KHook can still call it, alongside a newer hook of the same timing
//! that the plugin installed since, so each hook is called with a context of
//! its own, which stops running handlers once its removal is asked for. No
//! function of the plugin's is given to KHook to call back after a removal:
//! the removal can complete after Metamod unloaded the library.
//!
//! Metamod does not track the hooks of detoured functions before build 1474
//! (its `SetupHook` kept no record of them, unlike its `SetupVirtualHook`), so
//! those would stay installed after the library is unloaded, and the next
//! call of the function would crash. The plugin removes them as it unloads, by
//! [`MetamodApi::remove_function_hooks`](crate::MetamodApi::remove_function_hooks).
//! From build 1474, Metamod tracks them too, so their removal there is a
//! plugin removing hooks of its own, as above: safe from build 1490 only.

use super::signature::{HookReturn, Signature, Thunks, nothing};
use super::site::{Site, on_main_thread};
use super::{CallBackend, HookCall, HookError, HookTiming, Level};
use crate::MetamodApi;
use crate::sys::khook::{Action, HookId as KHookId, IKHook, IKHookVtable, INVALID_HOOK};
use std::cell::Cell;

#[cfg(any(test, feature = "sdk"))]
use std::ffi::CStr;

use std::ffi::{c_int, c_uint, c_void};
use std::mem::size_of;
use std::ops::RangeInclusive;
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicPtr, Ordering};

/// The Metamod 2.0 builds known to unload a plugin that removed hooks of its
/// own on vtable slots: those whose unload walks a copy of the plugin's hook
/// IDs, with a KHook that reports the removal of a hook it no longer has.
/// Later builds are not assumed to, until they are checked.
///
/// A build is the count of commits in the history Metamod was built from,
/// which names the official builds of its main history only. Metamod gives
/// plugins nothing else to tell builds apart by, so a custom build or a fork
/// of the same count is trusted as that build.
const RELEASING_BUILDS: RangeInclusive<u32> = 1490..=1490;

/// The latest `IKHook` Metamod gave the plugin.
static KHOOK: AtomicPtr<IKHook> = AtomicPtr::new(ptr::null_mut());

/// The vtable of every `IKHook` Metamod gives, which lasts as long as Metamod.
static KHOOK_VTABLE: AtomicPtr<IKHookVtable> = AtomicPtr::new(ptr::null_mut());

/// What KHook calls one of a site's hooks with.
pub(super) struct Context<S: Signature> {
	site: &'static Site<S>,
	/// Whether the hook runs the site's handlers, until its removal is asked
	/// for. Only the main thread reads or writes it.
	live: Cell<bool>,
}

impl<S: Signature> Context<S> {
	/// A context for a new hook of the site's. It is never freed, as KHook may
	/// still call the hook for a while after its removal is asked for: each
	/// installation keeps one, of two words.
	fn leak(site: &'static Site<S>) -> &'static Self {
		Box::leak(Box::new(Self {
			site,
			live: Cell::new(true),
		}))
	}
}

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

/// The build a Metamod 2.0 version string names, such as 1490 in
/// `2.0.0-dev+1490`: the count of commits the build was made from.
#[cfg(any(test, feature = "sdk"))]
pub(super) fn build_of(version: &CStr) -> Option<u32> {
	let (release, build) = version
		.to_str()
		.ok()?
		.strip_prefix("2.0.")?
		.split_once("-dev+")?;

	let digits = |text: &str| !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());

	(digits(release) && digits(build))
		.then(|| build.parse().ok())
		.flatten()
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
	// `install_slot` or `install_function` set to one of `S`, which is never
	// freed.
	let context = unsafe { &*(vtable.get_context_ptr)(khook).cast::<Context<S>>() };
	let site = context.site;

	if !context.live.get() || !site.active() {
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

/// KHook's `pre` and `post` for a hook of `timing`, one of them null.
fn callbacks<S: Signature>(thunks: Thunks<S>, timing: HookTiming) -> (*mut c_void, *mut c_void) {
	match timing {
		HookTiming::Pre => (thunks.khook_pre.address().as_ptr(), ptr::null_mut()),
		HookTiming::Post => (ptr::null_mut(), thunks.khook_post.address().as_ptr()),
	}
}

/// KHook's `init_op`: copies a return value into KHook's storage.
unsafe extern "C" fn copy<R: HookReturn>(destination: *mut R, value: *const R) {
	// SAFETY: KHook allocates the storage at the size it was given.
	unsafe { destination.write_unaligned(value.read_unaligned()) };
}

/// The value Metamod declared its `metamod_version` console variable with.
///
/// It reads the variable by hand rather than through `source_sdk_2013`'s
/// `Cvar::find_var` and `ConVar::default_string`, which need a `Server`
/// binding: the plugin may install its first hook before it made one, and the
/// hook registry keeps none.
#[cfg(all(not(test), feature = "sdk"))]
fn declared_version(api: MetamodApi<'_>) -> Option<std::ffi::CString> {
	use source_sdk_2013::raw::interfaces::cvar::VERSION;
	use source_sdk_2013::raw::vcall;
	use source_sdk_2013::sys;

	let factory = api.engine_factory()?;

	// SAFETY: The engine's own factory, not Metamod's wrapper of it, takes a
	// version string and an optional place for a return code.
	let cvar = unsafe { factory(VERSION.as_ptr(), ptr::null_mut()) }.cast::<sys::ICvar>();
	let cvar = NonNull::new(cvar)?;

	// SAFETY: The engine exports its `ICvar` under this exact version, live as
	// long as the engine, and the plugin runs on the main thread, in one of
	// Metamod's callbacks.
	let var = unsafe { vcall!(cvar.as_ptr() => ICvar_FindVar(c"metamod_version".as_ptr())) };
	let var = NonNull::new(var)?;

	// SAFETY: The registry keeps the variable it found, and the one holding its
	// value, which is itself unless another module registered the name first.
	// Their declared values never change. Fields are read without forming
	// references.
	unsafe {
		let parent = (&raw const (*var.as_ptr()).m_pParent).read();
		let holder = if parent.is_null() {
			var.as_ptr()
		} else {
			parent
		};
		let declared = (&raw const (*holder).m_pszDefaultValue).read();

		(!declared.is_null()).then(|| CStr::from_ptr(declared).to_owned())
	}
}

/// KHook's `deinit_op`: return values need no destruction.
unsafe extern "C" fn destroy<R: HookReturn>(_value: *mut R) {}

/// Adds a hook running the site's handlers of `timing`, on a function, which
/// KHook detours.
///
/// # Safety
///
/// `function` must be the entry of a function of signature `S`, which stays
/// loaded until Metamod unloads the plugin.
pub(super) unsafe fn install_function<S: Signature>(
	site: &'static Site<S>,
	function: NonNull<c_void>,
	timing: HookTiming,
) -> Result<KHookId, HookError> {
	let (khook, functions) = interface();
	let thunks = S::THUNKS;
	let (pre, post) = callbacks(thunks, timing);
	let context = Context::leak(site);

	// SAFETY: As the caller promises, the function is of the hook's signature.
	// The context, which is never freed, leads to the site, and the hook only
	// runs before Metamod unloads the library.
	let id = unsafe {
		(functions.setup_hook)(
			khook,
			function.as_ptr(),
			ptr::from_ref(context).cast_mut().cast(),
			ptr::null_mut(),
			pre,
			post,
			thunks.khook_make_return.address().as_ptr(),
			thunks.khook_call_original.address().as_ptr(),
			stack_size::<S>(),
			// As `KHook::Function` does, as a detour may be running.
			true,
		)
	};

	match id {
		INVALID_HOOK => Err(HookError::Refused),
		id => Ok(id),
	}
}

/// Adds a hook running the site's handlers of `timing`, on a vtable slot, with
/// the context KHook calls it with.
///
/// # Safety
///
/// The vtable must hold a function of signature `S` at the slot.
pub(super) unsafe fn install_slot<S: Signature>(
	site: &'static Site<S>,
	vtable: NonNull<*mut c_void>,
	index: usize,
	timing: HookTiming,
) -> Result<(KHookId, &'static Context<S>), HookError> {
	let index = c_int::try_from(index).map_err(|_| HookError::InvalidArgument)?;
	let (khook, functions) = interface();
	let thunks = S::THUNKS;
	let (pre, post) = callbacks(thunks, timing);
	let context = Context::leak(site);

	// SAFETY: As the caller promises, calls through the slot go to a function of
	// the hook's signature. The context, which is never freed, leads to the
	// site, and the hook only runs before Metamod unloads the library.
	let id = unsafe {
		(functions.setup_virtual_hook)(
			khook,
			vtable.as_ptr(),
			index,
			ptr::from_ref(context).cast_mut().cast(),
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
		id => Ok((id, context)),
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

/// Whether hooks on vtable slots may be removed before the plugin unloads: under
/// a Metamod that reports itself as one of [`RELEASING_BUILDS`].
///
/// Metamod gives plugins no call that reports its build, only its version
/// string, as the value its `metamod_version` console variable was declared
/// with, which console commands cannot change. Without the `sdk` feature, the
/// plugin cannot read it, and hooks stay.
pub(super) fn releases_slot_hooks(api: MetamodApi<'_>) -> bool {
	running_build(api).is_some_and(|build| RELEASING_BUILDS.contains(&build))
}

/// Removes the detours `ids`, which [`install_function`] added, so that no
/// detour calls into the unloaded library under a Metamod before build 1474,
/// which keeps no record of them. Call it before the plugin's `Unload`
/// returns. From build 1474, Metamod removes them itself, and only build 1490
/// copes with the plugin removing them first (see the [module](self)).
pub(super) fn remove_detours(ids: Vec<KHookId>) {
	if ids.is_empty() {
		return;
	}

	let (khook, vtable) = interface();

	for id in ids {
		// SAFETY: Metamod keeps the interface until the plugin's `Unload`. The
		// removal is synchronous, so the detour is gone when it returns.
		unsafe { (vtable.remove_hook)(khook, id, false, None, ptr::null_mut()) };
	}
}

/// Removes a hook [`install_slot`] added, from KHook's own thread, and stops it
/// running handlers meanwhile.
pub(super) fn remove_slot_hook<S: Signature>(id: KHookId, context: &'static Context<S>) {
	context.live.set(false);

	let (khook, vtable) = interface();

	// SAFETY: Metamod keeps the interface until the plugin's `Unload`. An
	// asynchronous removal never waits for the hook's detour, and calls back
	// nothing of the plugin's.
	unsafe { (vtable.remove_hook)(khook, id, true, None, ptr::null_mut()) };
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

/// The build of the Metamod running, from its version string.
fn running_build(api: MetamodApi<'_>) -> Option<u32> {
	#[cfg(test)]
	{
		let _ = api;
		crate::test_support::harness::metamod_build()
	}

	#[cfg(all(not(test), feature = "sdk"))]
	{
		declared_version(api).as_deref().and_then(build_of)
	}

	#[cfg(all(not(test), not(feature = "sdk")))]
	{
		let _ = api;
		None
	}
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
