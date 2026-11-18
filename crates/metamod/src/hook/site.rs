//! The functions a plugin hooks, and what their hooks run.
//!
//! A site is one hooked vtable slot, or one hooked function. It has one hook of
//! the library's per timing, through which it runs the handlers installed on
//! it, so that handlers come and go without the library's hooks doing so for
//! each. On a vtable slot, the library's hook of a timing leaves once no
//! handler of that timing is left, where the library allows it (see
//! [`Registry::release_emptied`]), and is installed again with the next one.

use super::signature::{HOOK_MANAGERS, Signature};

use super::{
	Handler, HookAction, HookCall, HookError, HookId, HookTiming, khook, sourcehook, timing,
};

use crate::MetamodApi;
use crate::api::MetamodVersion;
use crate::sys::khook::HookId as KHookId;
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

	/// Drops the handlers and the installed state of a site that hooks a
	/// function, whose detour was removed.
	fn forget_detour(&self);

	/// What the site hooks.
	fn place(&self) -> Place;

	/// Takes the library's hook of each timing that has no handler left off a
	/// vtable slot, unless the site's handlers are running.
	fn release(&self, backend: Backend);

	fn remove(&self, id: HookId) -> bool;
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

/// The hook of the library's through which a site runs its handlers of one
/// timing.
pub(super) enum LibraryHook<S: Signature> {
	/// A KHook detour of a function, which stays until the plugin removes every
	/// detour as it unloads, with [`Registry::remove_detours`].
	Detour,

	/// A KHook hook on a vtable slot, and the context KHook calls it with.
	KHook {
		id: KHookId,
		context: &'static khook::Context<S>,
	},

	/// A SourceHook hook on a vtable slot, by its ID.
	SourceHook(c_int),
}

impl<S: Signature> Clone for LibraryHook<S> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<S: Signature> Copy for LibraryHook<S> {}

/// A value only the server's main thread uses.
struct MainThread<T>(T);

// SAFETY: Only functions taking a `MetamodApi` reach the value, and a
// `MetamodApi` only exists on the main thread.
unsafe impl<T> Sync for MainThread<T> {}

/// What a site hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Place {
	/// A slot of a vtable.
	Slot {
		vtable: NonNull<*mut c_void>,
		index: usize,
	},

	/// A function, by its address, called directly.
	Function(NonNull<c_void>),
}

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
	/// The KHook hooks of detoured functions, which Metamod does not track.
	detours: Vec<crate::sys::khook::HookId>,
	/// Whether the library's hooks leave vtable slots without handlers.
	releases: bool,
}

impl Registry {
	const fn new() -> Self {
		Self {
			api_version: 0,
			backend: None,
			generation: None,
			hook_managers: [None; HOOK_MANAGERS],
			sites: Vec::new(),
			detours: Vec::new(),
			releases: false,
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
			let (backend, releases) = match api.version() {
				// SourceHook removes a hook by its ID whenever asked, and Metamod 1.12
				// removes the plugin's remaining hooks as it unloads it.
				MetamodVersion::Stable1226 => (
					Backend::SourceHook {
						sourcehook: sourcehook::bind(api)?,
						plugin: status.id,
					},
					true,
				),

				MetamodVersion::Dev1469 => {
					khook::bind(api, status.id)?;
					(Backend::KHook, khook::releases_slot_hooks(api))
				}
			};

			registry.backend = Some(backend);
			registry.releases = releases;
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

	/// The site of a function, created on first use.
	pub(super) fn function_site<S: Signature>(
		&mut self,
		function: NonNull<c_void>,
	) -> Result<&'static Site<S>, HookError> {
		self.site_at(Place::Function(function))
	}

	/// Installs the site's hook for `timing` with the hooking library.
	///
	/// # Safety
	///
	/// The site's vtable must hold a function of signature `S` at its slot, or
	/// its function be one of signature `S`.
	pub(super) unsafe fn install<S: Signature>(
		&mut self,
		site: &'static Site<S>,
		timing: HookTiming,
	) -> Result<(), HookError> {
		let hook = match (self.backend, site.place()) {
			// SAFETY: As the caller promises.
			(Some(Backend::KHook), Place::Slot { vtable, index }) => unsafe {
				khook::install_slot(site, vtable, index, timing)
					.map(|(id, context)| LibraryHook::KHook { id, context })
			},

			// SAFETY: As the caller promises.
			(Some(Backend::KHook), Place::Function(function)) => unsafe {
				khook::install_function(site, function, timing).map(|id| {
					self.detours.push(id);
					LibraryHook::Detour
				})
			},

			(Some(Backend::SourceHook { sourcehook, plugin }), Place::Slot { vtable, index }) => {
				let manager = sourcehook::hook_manager::<S>(&mut self.hook_managers, index)?;

				// SAFETY: As the caller promises.
				unsafe {
					sourcehook::install(sourcehook, plugin, manager, site, vtable, index, timing)
				}
				.map(LibraryHook::SourceHook)
			}

			// SourceHook only hooks virtual functions.
			(Some(Backend::SourceHook { .. }), Place::Function(_)) => {
				Err(HookError::DetourUnsupported)
			}

			(None, _) => Err(HookError::NotBound),
		}?;

		site.hooks[timing as usize].set(Some(hook));
		Ok(())
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

	/// Takes the library's hook of each timing of a vtable slot that no handler
	/// is left for off the slot, unless the slot's handlers are running, or the
	/// library cannot have it removed safely: then it stays, and each call
	/// through it finds no handler. It is installed again with the next handler.
	///
	/// SourceHook removes a hook at once, and restores the slot once it has no
	/// hook left. KHook removes it on its own thread soon after, without the
	/// main thread waiting for it, so that a removal never waits for a call of
	/// the hooked function that is running, maybe on this very thread. KHook
	/// hooks are only removed under a Metamod known to unload the plugin all
	/// the same (see [`khook::releases_slot_hooks`]).
	///
	/// Handlers removed while their site's handlers ran leave its hooks on the
	/// slot until the next removal or addition of a hook outside them.
	pub(super) fn release_emptied(&self) {
		let Some(backend) = self.backend.filter(|_| self.releases) else {
			return;
		};

		for site in &self.sites {
			site.release(backend);
		}
	}

	/// Stops the hook's handler, and takes the library's hooks that no handler
	/// is left for off their slots, where it can.
	pub(super) fn remove(&self, id: HookId) -> bool {
		let removed = self.sites.iter().any(|site| site.remove(id));

		if removed {
			self.release_emptied();
		}

		removed
	}

	/// Removes the detours of function hooks, and their handlers.
	pub(super) fn remove_detours(&mut self) {
		for site in &self.sites {
			site.forget_detour();
		}

		if matches!(self.backend, Some(Backend::KHook)) {
			khook::remove_detours(std::mem::take(&mut self.detours));
		}
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

		self.site_at(Place::Slot { vtable, index })
	}

	fn site_at<S: Signature>(&mut self, place: Place) -> Result<&'static Site<S>, HookError> {
		let found = self.sites.iter().find(|site| site.place() == place);

		if let Some(&site) = found {
			return site
				.as_any()
				.downcast_ref::<Site<S>>()
				.ok_or(HookError::SignatureMismatch);
		}

		let site: &'static Site<S> = Box::leak(Box::new(Site {
			api_version: self.api_version,
			generation: self.generation.unwrap_or_default(),
			place,
			entries: RefCell::new(Vec::new()),
			depth: Cell::new(0),
			hooks: [Cell::new(None), Cell::new(None)],
		}));

		self.sites.push(site);
		Ok(site)
	}
}

/// One hooked vtable slot or function, of signature `S`, for one load of the
/// plugin.
pub(super) struct Site<S: Signature> {
	api_version: c_int,
	generation: u64,
	place: Place,
	entries: RefCell<Vec<Entry<S>>>,
	/// How many calls are running the site's handlers.
	depth: Cell<u32>,
	/// The library's hook for each timing, while installed.
	hooks: [Cell<Option<LibraryHook<S>>>; 2],
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

			let action = timing::time(
				|| entry.handler.name(),
				|| catch_unwind(AssertUnwindSafe(|| entry.handler.call(call))),
			)
			.unwrap_or(HookAction::Ignore);

			call.record(action);
		}

		let depth = self.depth.get() - 1;

		self.depth.set(depth);

		if depth == 0 {
			self.entries.borrow_mut().retain(|entry| !entry.removed);
		}
	}

	pub(super) fn installed(&self, timing: HookTiming) -> bool {
		self.hooks[timing as usize].get().is_some()
	}

	pub(super) const fn place(&self) -> Place {
		self.place
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

	fn forget_detour(&self) {
		if !matches!(self.place, Place::Function(_)) {
			return;
		}

		let mut entries = self.entries.borrow_mut();

		if self.depth.get() == 0 {
			entries.clear();
		} else {
			entries.iter_mut().for_each(|entry| entry.removed = true);
		}

		self.hooks.iter().for_each(|hook| hook.set(None));
	}

	fn place(&self) -> Place {
		self.place
	}

	fn release(&self, backend: Backend) {
		// While the handlers run, called from the library's hook itself, a
		// removal only marks the entries, and the hook stays until a removal or
		// addition outside them.
		if self.depth.get() != 0 {
			return;
		}

		for timing in [HookTiming::Pre, HookTiming::Post] {
			let hook = &self.hooks[timing as usize];

			let handled = self
				.entries
				.borrow()
				.iter()
				.any(|entry| entry.timing == timing && !entry.removed);

			let released = match (hook.get(), backend) {
				_ if handled => false,

				(Some(LibraryHook::KHook { id, context }), Backend::KHook) => {
					khook::remove_slot_hook(id, context);
					true
				}

				// SAFETY: The site's handlers are not running, so neither is the
				// hook's delegate, which runs them.
				(Some(LibraryHook::SourceHook(id)), Backend::SourceHook { sourcehook, .. }) => unsafe {
					sourcehook::remove(sourcehook, id);
					true
				},

				// Detours stay until the plugin unloads.
				_ => false,
			};

			if released {
				hook.set(None);
			}
		}
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
}

/// Whether this is the server's main thread.
pub(super) fn on_main_thread() -> bool {
	MAIN_THREAD.try_with(Cell::get).unwrap_or(false)
}

/// What the shell reports of the plugin.
fn plugin_status(api_version: c_int) -> PluginStatus {
	#[cfg(test)]
	if let Some(status) = crate::test_support::harness::plugin_status() {
		return status;
	}

	cpp_metamod_plugin_status(api_version)
}
