//! Hooks on C++ virtual functions, through the hooking library of whichever
//! Metamod:Source runs the plugin: SourceHook in 1.12 build 1226, and KHook in
//! 2.0 builds 1469 through 1472.
//!
//! A [`VirtualFunction`] names a function by its vtable slot and
//! [`Signature`], like SourceHook's `SH_DECL_MANUALHOOK` and KHook's
//! `Virtual`. [`MetamodApi::add_hook`] runs a [`Handler`] before or after each
//! call of the function that a [`HookTarget`] applies to, and the handler's
//! [`HookAction`] decides what the call does.
//!
//! # Behavior on both libraries
//!
//! - Handlers run only for calls on the server's main thread, the one Metamod
//!   runs the plugin on. Calls from other threads go to the function as if
//!   the plugin had no hooks on it.
//! - Handlers stop running while the plugin is paused, and once it unloads.
//! - A plugin's hooks stay installed until Metamod unloads it, which removes
//!   them before unloading the library: removing a hook only stops its
//!   handler. Metamod 2.0 never unloads a library whose plugin removed a KHook
//!   hook itself, so each hooked function keeps one hook per timing.
//! - Of a plugin's handlers on one function, the greatest action decides the
//!   call, with the value of the last one that overrode or superseded it.
//!
//! # Differences
//!
//! - KHook has no [`HookAction::Handled`], and ignores it.
//! - KHook does not report what other plugins' hooks did before the function
//!   runs, so [`HookCall::superseded`] cannot tell there.
//! - Between plugins, SourceHook returns the value of the last hook that
//!   overrode or superseded the call, and KHook that of the first hook with
//!   the greatest action.
//! - SourceHook's hook loop is not safe to run on two threads at once, so on
//!   other threads this library's hooks call the original function without
//!   entering it. Other plugins' SourceHook hooks on the same function are
//!   skipped there too when SourceHook patched the slot with one of this
//!   library's hook functions, and enter the loop as they always do when it
//!   patched it with theirs.

/// An array of `function::<0>` up to `function::<63>`, one per SourceHook hook
/// manager.
macro_rules! hook_managers {
	($($function:tt)*) => {
		[
			$($function)*::<0>, $($function)*::<1>, $($function)*::<2>, $($function)*::<3>,
			$($function)*::<4>, $($function)*::<5>, $($function)*::<6>, $($function)*::<7>,
			$($function)*::<8>, $($function)*::<9>, $($function)*::<10>, $($function)*::<11>,
			$($function)*::<12>, $($function)*::<13>, $($function)*::<14>, $($function)*::<15>,
			$($function)*::<16>, $($function)*::<17>, $($function)*::<18>, $($function)*::<19>,
			$($function)*::<20>, $($function)*::<21>, $($function)*::<22>, $($function)*::<23>,
			$($function)*::<24>, $($function)*::<25>, $($function)*::<26>, $($function)*::<27>,
			$($function)*::<28>, $($function)*::<29>, $($function)*::<30>, $($function)*::<31>,
			$($function)*::<32>, $($function)*::<33>, $($function)*::<34>, $($function)*::<35>,
			$($function)*::<36>, $($function)*::<37>, $($function)*::<38>, $($function)*::<39>,
			$($function)*::<40>, $($function)*::<41>, $($function)*::<42>, $($function)*::<43>,
			$($function)*::<44>, $($function)*::<45>, $($function)*::<46>, $($function)*::<47>,
			$($function)*::<48>, $($function)*::<49>, $($function)*::<50>, $($function)*::<51>,
			$($function)*::<52>, $($function)*::<53>, $($function)*::<54>, $($function)*::<55>,
			$($function)*::<56>, $($function)*::<57>, $($function)*::<58>, $($function)*::<59>,
			$($function)*::<60>, $($function)*::<61>, $($function)*::<62>, $($function)*::<63>,
		]
	};
}

mod khook;
mod signature;
mod site;
mod sourcehook;

#[cfg(test)]
#[path = "../tests/hook.rs"]
mod tests;

use crate::MetamodApi;
use crate::sys::sourcehook::{ISourceHook, MetaRes};
use site::Registry;
use std::cell::Cell;
use std::ffi::{c_int, c_void};
use std::marker::PhantomData;
use std::mem::size_of;
use std::num::NonZeroU64;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};

pub use signature::{HookArg, HookReturn, Signature};

/// How a call reached a handler, for its queries.
#[derive(Debug, Clone, Copy)]
enum CallBackend {
	SourceHook(NonNull<ISourceHook>),
	KHook,
}

/// Runs for the calls of a hooked function. Implemented for functions and
/// closures of the same signature as [`Handler::call`].
pub trait Handler<S: Signature>: 'static {
	/// Called for each call on the server's main thread, before or after the
	/// function, as the hook was installed. A panic is caught, and counts as
	/// [`HookAction::Ignore`].
	fn call(&self, call: &HookCall<'_, S>) -> HookAction<S::Output>;
}

impl<S, F> Handler<S> for F
where
	S: Signature,
	F: Fn(&HookCall<'_, S>) -> HookAction<S::Output> + 'static,
{
	fn call(&self, call: &HookCall<'_, S>) -> HookAction<S::Output> {
		self(call)
	}
}

/// What a handler does with a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookAction<R> {
	/// Takes no action: SourceHook's `MRES_IGNORED`.
	Ignore,

	/// Did something, but lets the call proceed: SourceHook's `MRES_HANDLED`.
	/// KHook takes it as [`Self::Ignore`].
	Handled,

	/// Lets the function run, but returns this value instead.
	Override(R),

	/// Skips the function, if it has not run, and returns this value instead.
	Supersede(R),
}

impl<R> HookAction<R> {
	fn into_parts(self) -> (Level, Option<R>) {
		match self {
			Self::Ignore => (Level::Ignore, None),
			Self::Handled => (Level::Handled, None),
			Self::Override(value) => (Level::Override, Some(value)),
			Self::Supersede(value) => (Level::Supersede, Some(value)),
		}
	}
}

/// A call of a hooked function, as a [`Handler`] sees it.
pub struct HookCall<'call, S: Signature> {
	this: *mut S::This,
	args: S::Args,
	timing: HookTiming,
	backend: CallBackend,
	outcome: Cell<Outcome<S::Output>>,
	_call: PhantomData<&'call ()>,
}

impl<S: Signature> HookCall<'_, S> {
	fn new(this: *mut S::This, args: S::Args, timing: HookTiming, backend: CallBackend) -> Self {
		Self {
			this,
			args,
			timing,
			backend,
			outcome: Cell::new(Outcome::IGNORED),
			_call: PhantomData,
		}
	}

	/// The arguments after `this`.
	pub fn args(&self) -> S::Args {
		self.args
	}

	fn outcome(&self) -> Outcome<S::Output> {
		self.outcome.get()
	}

	fn record(&self, action: HookAction<S::Output>) {
		let mut outcome = self.outcome.get();
		let (level, value) = action.into_parts();

		outcome.level = outcome.level.max(level);

		if value.is_some() {
			outcome.value = value;
		}

		self.outcome.set(outcome);
	}

	/// What the call returns so far: the value an earlier hook overrode it
	/// with, or after the function ran, its own. `None` before either, and in
	/// a pre hook for a function returning nothing.
	pub fn return_value(&self) -> Option<S::Output> {
		let outcome = self.outcome.get();

		if outcome.level >= Level::Override {
			return outcome.value;
		}

		if size_of::<S::Output>() == 0 {
			return (self.timing == HookTiming::Post).then(signature::nothing);
		}

		let value = match self.backend {
			// SAFETY: SourceHook is running a delegate of this call's.
			CallBackend::SourceHook(sourcehook) => unsafe {
				sourcehook::return_value(sourcehook, self.timing)
			},

			// SAFETY: KHook is running a hook of this call's.
			CallBackend::KHook => unsafe { khook::return_value() },
		};

		// SAFETY: Either library keeps the hooked function's return value, of
		// this signature's type, for as long as the call runs.
		NonNull::new(value.cast_mut())
			.map(|value| unsafe { value.cast::<S::Output>().read_unaligned() })
	}

	/// In a pre hook, whether a hook that ran before this one superseded the
	/// call, so that the function will not run. `None` in a post hook, or if
	/// the hooking library cannot tell: KHook reports nothing about other
	/// plugins' hooks.
	pub fn superseded(&self) -> Option<bool> {
		if self.timing == HookTiming::Post {
			return None;
		}

		if self.outcome.get().level == Level::Supersede {
			return Some(true);
		}

		match self.backend {
			// SAFETY: SourceHook is running a delegate of this call's.
			CallBackend::SourceHook(sourcehook) => {
				Some(unsafe { sourcehook::status(sourcehook) } >= MetaRes::SUPERCEDE)
			}

			CallBackend::KHook => None,
		}
	}

	/// The object the function was called on.
	pub fn this(&self) -> *mut S::This {
		self.this
	}

	pub fn timing(&self) -> HookTiming {
		self.timing
	}
}

/// Why a hook could not be installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HookError {
	#[error(
		"hooks can only be installed while Metamod runs the plugin, and with a hooking library"
	)]
	NotBound,

	#[error("the hook is already installed")]
	AlreadyInstalled,

	#[error("Metamod's hooking library refused the hook")]
	Refused,

	#[error("the hook was given invalid arguments")]
	InvalidArgument,

	#[error("this Metamod version is not supported")]
	Unsupported,

	#[error("the function is already hooked with another signature")]
	SignatureMismatch,

	#[error("too many virtual functions are hooked")]
	TooManyFunctions,
}

/// Identifies an installed hook, for [`MetamodApi::remove_hook`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HookId(NonZeroU64);

impl HookId {
	fn next() -> Self {
		static NEXT: AtomicU64 = AtomicU64::new(1);

		Self(NonZeroU64::new(NEXT.fetch_add(1, Ordering::Relaxed)).expect("hook IDs ran out"))
	}
}

/// The calls a hook applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HookTarget(Target);

impl HookTarget {
	/// Calls on every object sharing `object`'s vtable: objects of its class,
	/// but not of the classes derived from it, which have vtables of their own.
	pub fn class_of<T>(object: NonNull<T>) -> Self {
		Self(Target::ClassOf(object.cast()))
	}

	/// Calls on `object` alone.
	pub fn instance<T>(object: NonNull<T>) -> Self {
		Self(Target::Instance(object.cast()))
	}

	/// Calls through `vtable`: on every object of the class it belongs to.
	pub fn vtable(vtable: NonNull<*mut c_void>) -> Self {
		Self(Target::Vtable(vtable))
	}

	/// The vtable the calls go through, and the object they must be on.
	///
	/// # Safety
	///
	/// For an object target, that object must be live. For a direct vtable
	/// target, the table must be live; no instance is required.
	unsafe fn resolve(self) -> Option<(NonNull<*mut c_void>, Option<NonNull<c_void>>)> {
		// SAFETY: As the caller promises, a polymorphic object starts with its
		// vtable pointer.
		let vtable_of = |object: NonNull<c_void>| unsafe {
			NonNull::new(object.cast::<*mut *mut c_void>().read())
		};

		match self.0 {
			Target::ClassOf(object) => Some((vtable_of(object)?, None)),
			Target::Instance(object) => Some((vtable_of(object)?, Some(object))),
			Target::Vtable(vtable) => Some((vtable, None)),
		}
	}
}

/// When a handler runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookTiming {
	/// Before the function, which it may supersede.
	Pre,

	/// After the function, or after it was superseded.
	Post,
}

/// The greatest of [`HookAction`]'s levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
	Ignore,
	Handled,
	Override,
	Supersede,
}

/// What a site's handlers decided for a call, so far.
#[derive(Debug, Clone, Copy)]
struct Outcome<R> {
	level: Level,
	value: Option<R>,
}

impl<R> Outcome<R> {
	const IGNORED: Self = Self {
		level: Level::Ignore,
		value: None,
	};
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Target {
	ClassOf(NonNull<c_void>),
	Instance(NonNull<c_void>),
	Vtable(NonNull<*mut c_void>),
}

/// A virtual function by its vtable slot and [`Signature`]: like SourceHook's
/// `SH_DECL_MANUALHOOK`, or KHook's `Virtual` by index.
///
/// The slot counts from 0 in the vtable `this` points to. A class's slots can
/// differ between the ABIs: MSVC groups overloads in reverse order, and gives a
/// virtual destructor one slot where the Itanium ABI gives two.
#[derive(Debug, Clone, Copy)]
pub struct VirtualFunction<S: Signature> {
	index: usize,
	_signature: PhantomData<fn() -> S>,
}

impl<S: Signature> VirtualFunction<S> {
	pub const fn new(index: usize) -> Self {
		Self {
			index,
			_signature: PhantomData,
		}
	}

	pub const fn index(self) -> usize {
		self.index
	}
}

impl MetamodApi<'_> {
	/// Runs `handler` for the calls of `function` that `target` applies to.
	///
	/// The hook lasts until [removed](Self::remove_hook), or until Metamod
	/// unloads the plugin. It is installed on the vtable the target goes
	/// through. When KHook has already detoured the function's slot, for another
	/// plugin or for this plugin's hooks of the other [`HookTiming`], it adds the
	/// hook from a worker thread. The worker polls every 5 milliseconds, and
	/// tries again later while the function is being called, so the hook can
	/// miss the calls made in the meantime.
	///
	/// # Safety
	///
	/// For an object target, that object must be live. For a direct vtable
	/// target, the table must be live; no instance is required. The table must
	/// hold, at `function`'s slot, a function of the C++ type `S` stands for.
	/// The vtable and function must stay loaded until Metamod unloads the plugin.
	pub unsafe fn add_hook<S: Signature>(
		self,
		function: VirtualFunction<S>,
		target: HookTarget,
		timing: HookTiming,
		handler: &'static dyn Handler<S>,
	) -> Result<HookId, HookError> {
		// SAFETY: As the caller promises.
		let (vtable, instance) = unsafe { target.resolve() }.ok_or(HookError::InvalidArgument)?;
		let mut registry = Registry::bind(self)?;
		let site = registry.site::<S>(vtable, function.index())?;

		if !site.installed(timing) {
			// SAFETY: As the caller promises, the slot holds a function of the
			// site's signature.
			unsafe { registry.install(site, timing) }?;
			site.set_installed(timing);
		}

		let id = HookId::next();

		site.push(id, timing, instance, handler);
		Ok(id)
	}

	/// Whether the hook is installed, for this load of the plugin.
	pub fn has_hook(self, id: HookId) -> bool {
		Registry::current(self).is_some_and(|registry| registry.contains(id))
	}

	/// The function `target`'s vtable held at `function`'s slot before any
	/// hook: calling it skips every plugin's hooks.
	///
	/// # Safety
	///
	/// As for [`Self::add_hook`].
	pub unsafe fn original_function<S: Signature>(
		self,
		function: VirtualFunction<S>,
		target: HookTarget,
	) -> Result<S, HookError> {
		// SAFETY: As the caller promises.
		let (vtable, _) = unsafe { target.resolve() }.ok_or(HookError::InvalidArgument)?;
		let index = c_int::try_from(function.index()).map_err(|_| HookError::InvalidArgument)?;
		let registry = Registry::bind(self)?;

		// SAFETY: As the caller promises, the vtable is live and has the slot.
		let original = unsafe { registry.original(vtable, index) };

		NonNull::new(original)
			.map(S::from_address)
			.ok_or(HookError::InvalidArgument)
	}

	/// Stops the hook's handler. Returns whether the hook was installed, for
	/// this load of the plugin.
	pub fn remove_hook(self, id: HookId) -> bool {
		Registry::current(self).is_some_and(|registry| registry.remove(id))
	}
}
