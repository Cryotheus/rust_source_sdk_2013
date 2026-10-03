//! A running Metamod of either version, through mocks of its API and hooking
//! library, and the plugin status its shell would report.

use super::foreign::FOREIGN_KHOOK;
use super::khook::MockKHook;
use super::smm::MockSmm;
use super::sourcehook::MockSourceHook;
use crate::api::{MetamodApi, MetamodVersion};
use crate::hook::Signature;
use crate::sys::khook::IKHook;
use crate::sys::plugin::PluginStatus;
use crate::sys::sourcehook::ISourceHook;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Each harness is a load of its own.
static GENERATIONS: AtomicU64 = AtomicU64::new(1);

/// The hooks' state is the library's, so tests take turns.
static SERIAL: Mutex<()> = Mutex::new(());

thread_local! {
	/// What handlers noticed going wrong.
	static FAILURES: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };

	/// What the shell reports in place of the C++ shell's own status.
	static PLUGIN_STATUS: Cell<Option<PluginStatus>> = const { Cell::new(None) };
}

/// A running Metamod of one version, through mocks of its API and hooking
/// library.
pub(crate) struct Harness {
	pub(crate) version: MetamodVersion,
	smm: Box<MockSmm>,
	pub(crate) sourcehook: Box<MockSourceHook>,
	pub(crate) khook: Box<MockKHook>,
	/// The plugin's load.
	pub(crate) generation: u64,
	_serial: MutexGuard<'static, ()>,
}

impl Harness {
	/// A Metamod of `version` running the plugin, in a load of its own.
	pub(crate) fn new(version: MetamodVersion) -> Self {
		let serial = SERIAL.lock().unwrap_or_else(PoisonError::into_inner);
		let generation = GENERATIONS.fetch_add(1, Ordering::Relaxed);

		FAILURES.take();

		let mut harness = Self {
			version,
			smm: MockSmm::new(version, &[]),
			sourcehook: MockSourceHook::new(),
			khook: MockKHook::new(),
			generation,
			_serial: serial,
		};

		harness.smm.sourcehook = harness.sourcehook_ptr().cast();
		harness.smm.khook = harness.khook_ptr().cast();
		FOREIGN_KHOOK.set(harness.khook_ptr());
		harness.set_status(true, false, generation);

		harness
	}

	pub(crate) fn api(&self) -> MetamodApi<'_> {
		self.smm.api()
	}

	/// Calls the function at `index` of the object's vtable, as the engine
	/// would through a hooked vtable.
	pub(crate) fn call<S: Signature>(
		&self,
		object: *mut S::This,
		index: usize,
		args: S::Args,
	) -> S::Output {
		// SAFETY: Every object of the tests starts with its vtable, which has a
		// function of signature `S` at `index`.
		let vtable = unsafe { object.cast::<*mut *mut c_void>().read() };

		match self.version {
			MetamodVersion::Stable1226 => {
				// SAFETY: As above, SourceHook patched the slot with a hook function
				// of the same signature, if any.
				let function = unsafe { vtable.add(index).read() };

				// SAFETY: As above.
				unsafe {
					S::invoke(
						S::from_address(NonNull::new(function).unwrap()),
						object,
						args,
					)
				}
			}

			MetamodVersion::Dev1469 => self.khook.call::<S>(vtable, index, object, args),
		}
	}

	pub(crate) fn khook_ptr(&self) -> *mut IKHook {
		self.khook.ptr()
	}

	/// Has the shell report the plugin's status as this.
	pub(crate) fn set_status(&self, loaded: bool, paused: bool, generation: u64) {
		PLUGIN_STATUS.set(Some(PluginStatus {
			generation,
			id: 7,
			loaded,
			paused,
		}));
	}

	pub(crate) fn sourcehook_ptr(&self) -> *mut ISourceHook {
		self.sourcehook.ptr()
	}
}

impl Drop for Harness {
	fn drop(&mut self) {
		PLUGIN_STATUS.set(None);
		FOREIGN_KHOOK.set(std::ptr::null_mut());
	}
}

/// Notes what a handler found wrong, for [`on_both`] to fail the test with.
pub(crate) fn expect(condition: bool, failure: &'static str) {
	if !condition {
		FAILURES.with_borrow_mut(|failures| failures.push(failure));
	}
}

/// Runs `test` on a Metamod of each version. Fails if a handler noted a
/// failure with [`expect`], as handlers' panics are caught.
pub(crate) fn on_both(test: impl Fn(&Harness)) {
	for version in [MetamodVersion::Stable1226, MetamodVersion::Dev1469] {
		let harness = Harness::new(version);

		test(&harness);
		assert_eq!(FAILURES.take(), Vec::<&str>::new(), "on {version:?}");
	}
}

/// What the shell reports of the plugin while a harness runs, in place of
/// the C++ shell's status.
pub(crate) fn plugin_status() -> Option<PluginStatus> {
	PLUGIN_STATUS.get()
}
