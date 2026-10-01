//! The functions a plugin hooks, and what their hooks run.
//!
//! A site is one hooked vtable slot. It has one hook of the library's per
//! timing, through which it runs the handlers installed on it, so that hooks
//! come and go without the library's hooks doing so.

use super::signature::{HOOK_MANAGERS, Signature};
use super::{Handler, HookAction, HookCall, HookError, HookId, HookTiming, khook, sourcehook};
use crate::MetamodApi;
use crate::api::MetamodVersion;
use crate::sys::plugin::{PluginStatus, cpp_metamod_plugin_status};
use crate::sys::sourcehook::ISourceHook;
use std::any::{Any, TypeId};
use std::cell::{Cell, RefCell, RefMut};
use std::ffi::{c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};

static REGISTRY: MainThread<RefCell<Registry>> = MainThread(RefCell::new(Registry::new()));

thread_local! {
	/// Whether this is the server's main thread, which installed the hooks.
	static MAIN_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// What the registry needs of a site, whatever its signature.
trait AnySite: Any {
	fn as_any(&self) -> &dyn Any;

	fn contains(&self, id: HookId) -> bool;

	fn remove(&self, id: HookId) -> bool;

	/// The vtable and the index in it.
	fn slot(&self) -> (NonNull<*mut c_void>, usize);
}

/// The hooking library a registry installs hooks with.
#[derive(Debug, Clone, Copy)]
enum Backend {
	KHook,

	SourceHook {
		sourcehook: NonNull<ISourceHook>,
		plugin: c_int,
	},
}

/// A hook installed on a site.
#[derive(Clone, Copy)]
struct Entry<S: Signature> {
	id: HookId,
	timing: HookTiming,
	/// The only object the hook applies to, if any.
	instance: Option<NonNull<c_void>>,
	handler: &'static dyn Handler<S>,
	/// Removed while the site's handlers ran, to be dropped after.
	removed: bool,
}

/// A value only the server's main thread uses.
struct MainThread<T>(T);

// SAFETY: Only functions taking a `MetamodApi` reach the value, and a
// `MetamodApi` only exists on the main thread.
unsafe impl<T> Sync for MainThread<T> {}

/// The sites of one load of the plugin, and the library hooking them.
pub(super) struct Registry {
	/// The plugin API version of the Metamod running.
	api_version: c_int,
	backend: Option<Backend>,
	/// The load of the plugin the registry is for.
	generation: Option<u64>,
	/// SourceHook's hook manager for each signature and vtable slot.
	hook_managers: [Option<(TypeId, c_int)>; HOOK_MANAGERS],
	sites: Vec<&'static dyn AnySite>,
}

impl Registry {
	const fn new() -> Self {
		Self {
			api_version: 0,
			backend: None,
			generation: None,
			hook_managers: [None; HOOK_MANAGERS],
			sites: Vec::new(),
		}
	}

	/// The registry for the plugin's current load, bound to its hooking
	/// library.
	pub(super) fn bind(api: MetamodApi<'_>) -> Result<RefMut<'static, Self>, HookError> {
		let api_version = api.version().plugin_api_version();
		let status = plugin_status(api_version);

		if !status.loaded {
			return Err(HookError::NotBound);
		}

		MAIN_THREAD.with(|main_thread| main_thread.set(true));

		// Nothing borrows the registry while a handler runs or the library is
		// called, so a borrow only fails on re-entry from another thread.
		let mut registry = REGISTRY
			.0
			.try_borrow_mut()
			.map_err(|_| HookError::NotBound)?;

		if registry.generation != Some(status.generation) {
			// The sites of an earlier load stay allocated, as its hooks may still
			// run, but their handlers no longer do.
			*registry = Self::new();
			registry.api_version = api_version;
			registry.generation = Some(status.generation);
			sourcehook::reset_hook_managers();
		}

		if registry.backend.is_none() {
			registry.backend = Some(match api.version() {
				MetamodVersion::Stable1226 => Backend::SourceHook {
					sourcehook: sourcehook::bind(api)?,
					plugin: status.id,
				},

				MetamodVersion::Dev1469 => {
					khook::bind(api, status.id)?;
					Backend::KHook
				}
			});
		}

		Ok(registry)
	}

	/// The registry, if it is for the plugin's current load.
	pub(super) fn current(api: MetamodApi<'_>) -> Option<RefMut<'static, Self>> {
		let status = plugin_status(api.version().plugin_api_version());
		let registry = REGISTRY.0.try_borrow_mut().ok()?;

		(status.loaded && registry.generation == Some(status.generation)).then_some(registry)
	}

	pub(super) fn contains(&self, id: HookId) -> bool {
		self.sites.iter().any(|site| site.contains(id))
	}

	/// Installs the site's hook for `timing` with the hooking library.
	///
	/// # Safety
	///
	/// The site's vtable must hold a function of signature `S` at its slot.
	pub(super) unsafe fn install<S: Signature>(
		&mut self,
		site: &'static Site<S>,
		timing: HookTiming,
	) -> Result<(), HookError> {
		match self.backend {
			// SAFETY: As the caller promises.
			Some(Backend::KHook) => unsafe { khook::install(site, timing) },

			Some(Backend::SourceHook { sourcehook, plugin }) => {
				let manager = sourcehook::hook_manager::<S>(&mut self.hook_managers, site.index)?;

				// SAFETY: As the caller promises.
				unsafe { sourcehook::install(sourcehook, plugin, manager, site, timing) }
			}

			None => Err(HookError::NotBound),
		}
	}

	/// The function a vtable slot held before any hook.
	///
	/// # Safety
	///
	/// `vtable` must be a live vtable with the slot.
	pub(super) unsafe fn original(
		&self,
		vtable: NonNull<*mut c_void>,
		index: c_int,
	) -> *mut c_void {
		match self.backend {
			// SAFETY: As the caller promises.
			Some(Backend::KHook) => unsafe { khook::original(vtable, index) },

			// SAFETY: As the caller promises.
			Some(Backend::SourceHook { sourcehook, .. }) => unsafe {
				sourcehook::original(sourcehook, vtable, index)
			},

			None => ptr::null_mut(),
		}
	}

	pub(super) fn remove(&self, id: HookId) -> bool {
		self.sites.iter().any(|site| site.remove(id))
	}

	/// The site of a vtable slot, created on first use.
	pub(super) fn site<S: Signature>(
		&mut self,
		vtable: NonNull<*mut c_void>,
		index: usize,
	) -> Result<&'static Site<S>, HookError> {
		if c_int::try_from(index).is_err() {
			return Err(HookError::InvalidArgument);
		}

		let found = self
			.sites
			.iter()
			.find(|site| site.slot() == (vtable, index));

		if let Some(&site) = found {
			return site
				.as_any()
				.downcast_ref::<Site<S>>()
				.ok_or(HookError::SignatureMismatch);
		}

		let site: &'static Site<S> = Box::leak(Box::new(Site {
			api_version: self.api_version,
			generation: self.generation.unwrap_or_default(),
			vtable,
			index,
			entries: RefCell::new(Vec::new()),
			depth: Cell::new(0),
			installed: [Cell::new(false), Cell::new(false)],
		}));

		self.sites.push(site);
		Ok(site)
	}
}

/// One hooked vtable slot, of signature `S`, for one load of the plugin.
pub(super) struct Site<S: Signature> {
	api_version: c_int,
	generation: u64,
	vtable: NonNull<*mut c_void>,
	index: usize,
	entries: RefCell<Vec<Entry<S>>>,
	/// How many calls are running the site's handlers.
	depth: Cell<u32>,
	/// Whether the library's hook for each timing is installed.
	installed: [Cell<bool>; 2],
}

impl<S: Signature> Site<S> {
	/// Whether the site's handlers run: the plugin is loaded and not paused,
	/// in the load the site is for.
	pub(super) fn active(&self) -> bool {
		let status = plugin_status(self.api_version);

		status.loaded && !status.paused && status.generation == self.generation
	}

	/// Runs the handlers for the call, in the order they were installed.
	///
	/// Call it on the main thread.
	pub(super) fn dispatch(&self, call: &HookCall<'_, S>) {
		self.depth.set(self.depth.get() + 1);

		let mut position = 0;

		loop {
			// Handlers may install and remove hooks, so the entries stay unborrowed
			// while one runs. Removing one meanwhile only marks it.
			let entry = self.entries.borrow().get(position).copied();
			let Some(entry) = entry else { break };

			position += 1;

			if entry.removed
				|| entry.timing != call.timing()
				|| entry
					.instance
					.is_some_and(|instance| instance.as_ptr() != call.this().cast())
			{
				continue;
			}

			let action = catch_unwind(AssertUnwindSafe(|| entry.handler.call(call)))
				.unwrap_or(HookAction::Ignore);

			call.record(action);
		}

		let depth = self.depth.get() - 1;

		self.depth.set(depth);

		if depth == 0 {
			self.entries.borrow_mut().retain(|entry| !entry.removed);
		}
	}

	pub(super) const fn index(&self) -> usize {
		self.index
	}

	pub(super) fn installed(&self, timing: HookTiming) -> bool {
		self.installed[timing as usize].get()
	}

	pub(super) fn push(
		&self,
		id: HookId,
		timing: HookTiming,
		instance: Option<NonNull<c_void>>,
		handler: &'static dyn Handler<S>,
	) {
		self.entries.borrow_mut().push(Entry {
			id,
			timing,
			instance,
			handler,
			removed: false,
		});
	}

	pub(super) fn set_installed(&self, timing: HookTiming) {
		self.installed[timing as usize].set(true);
	}

	pub(super) const fn vtable(&self) -> NonNull<*mut c_void> {
		self.vtable
	}
}

impl<S: Signature> AnySite for Site<S> {
	fn as_any(&self) -> &dyn Any {
		self
	}

	fn contains(&self, id: HookId) -> bool {
		self.entries
			.borrow()
			.iter()
			.any(|entry| entry.id == id && !entry.removed)
	}

	fn remove(&self, id: HookId) -> bool {
		let mut entries = self.entries.borrow_mut();

		let Some(position) = entries
			.iter()
			.position(|entry| entry.id == id && !entry.removed)
		else {
			return false;
		};

		if self.depth.get() == 0 {
			entries.remove(position);
		} else {
			entries[position].removed = true;
		}

		true
	}

	fn slot(&self) -> (NonNull<*mut c_void>, usize) {
		(self.vtable, self.index)
	}
}

/// Whether this is the server's main thread.
pub(super) fn on_main_thread() -> bool {
	MAIN_THREAD.try_with(Cell::get).unwrap_or(false)
}

/// What the shell reports of the plugin.
fn plugin_status(api_version: c_int) -> PluginStatus {
	#[cfg(test)]
	if let Some(status) = super::tests::plugin_status() {
		return status;
	}

	cpp_metamod_plugin_status(api_version)
}
