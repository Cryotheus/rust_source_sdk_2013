//! Rust-owned state behind the compiler-owned Metamod virtual objects.
//!
//! Native entry points run on Metamod's main thread. No state guard survives
//! calling a plugin callback or registering a native listener. Status getters
//! are synchronized as their public Rust API is safe to call from other threads.

use crate::plugin::{
	HookStatus, LevelInitCallback, LevelShutdownCallback, PluginCallbacks, PluginMetadata,
	PluginStatus,
};

use std::ffi::{c_char, c_int, c_void};
use std::ptr;
use std::sync::Mutex;

const _: () = assert!(std::mem::size_of::<Dispatch>() == 7 * std::mem::size_of::<usize>());

static DEV: Mutex<Shell> = Mutex::new(Shell::new());

pub(crate) static DISPATCH: Dispatch = Dispatch {
	load: dispatch_load,
	all_loaded: dispatch_all_loaded,
	query: dispatch_query,
	unload: dispatch_unload,
	pause: dispatch_pause,
	metadata: dispatch_metadata,
	level: dispatch_level,
};

static STABLE: Mutex<Shell> = Mutex::new(Shell::new());

/// Private function addresses keep each plugin's lifecycle state within its DSO.
#[repr(C)]
pub(crate) struct Dispatch {
	pub load: unsafe extern "C" fn(
		c_int,
		*mut c_void,
		c_int,
		*mut c_void,
		*mut c_char,
		usize,
		bool,
	) -> bool,
	pub all_loaded: unsafe extern "C" fn(c_int, *mut c_void),
	pub query: unsafe extern "C" fn(c_int, *mut c_void, *mut c_char, usize) -> bool,
	pub unload: unsafe extern "C" fn(c_int, *mut c_void, *mut c_char, usize) -> bool,
	pub pause: unsafe extern "C" fn(c_int, *mut c_void, *mut c_char, usize, bool) -> bool,
	pub metadata: extern "C" fn(c_int, c_int) -> *const c_char,
	pub level: unsafe extern "C" fn(c_int, *const c_char, bool),
}

struct Levels {
	init: Option<LevelInitCallback>,
	shutdown: Option<LevelShutdownCallback>,
	context: *mut c_void,
	registered: bool,
	generation: u64,
	exhausted: bool,
}

impl Levels {
	const fn new() -> Self {
		Self {
			init: None,
			shutdown: None,
			context: ptr::null_mut(),
			registered: false,
			generation: 0,
			exhausted: false,
		}
	}

	fn advance(&mut self) {
		match self.generation.checked_add(1) {
			Some(generation) => self.generation = generation,
			None => self.exhausted = true,
		}
	}
}

struct Shell {
	callbacks: *const PluginCallbacks,
	metadata: PluginMetadata,
	api: *mut c_void,
	status: PluginStatus,
	levels: Levels,
}

impl Shell {
	const fn new() -> Self {
		Self {
			callbacks: ptr::null(),
			metadata: PluginMetadata {
				author: ptr::null(),
				name: ptr::null(),
				description: ptr::null(),
				url: ptr::null(),
				license: ptr::null(),
				version: ptr::null(),
				date: ptr::null(),
				log_tag: ptr::null(),
			},
			api: ptr::null_mut(),
			status: PluginStatus::UNLOADED,
			levels: Levels::new(),
		}
	}

	fn begin_load(&mut self, id: c_int, api: *mut c_void) -> Option<(*const PluginCallbacks, u64)> {
		self.clear_load();
		let generation = self.status.generation.checked_add(1)?;
		if self.callbacks.is_null() {
			return None;
		}
		self.status = PluginStatus {
			generation,
			id,
			loaded: true,
			paused: false,
		};
		self.api = api;
		Some((self.callbacks, generation))
	}

	fn clear_load(&mut self) {
		self.api = ptr::null_mut();
		self.status.id = 0;
		self.status.loaded = false;
		self.status.paused = false;
		self.levels = Levels::new();
	}

	fn listen(
		&mut self,
		init: Option<LevelInitCallback>,
		shutdown: Option<LevelShutdownCallback>,
		context: *mut c_void,
	) -> Result<*mut c_void, HookStatus> {
		if !self.status.loaded || self.api.is_null() {
			return Err(HookStatus::NOT_BOUND);
		}
		if self.levels.registered {
			return Err(HookStatus::ALREADY_INSTALLED);
		}
		if init.is_none() && shutdown.is_none() {
			return Err(HookStatus::INVALID_ARGUMENT);
		}
		self.levels = Levels {
			init,
			shutdown,
			context,
			registered: true,
			..Levels::new()
		};
		Ok(self.api)
	}
}

// SAFETY: State stores opaque addresses; synchronized reads never dereference
// them. Only main-thread unsafe native entry points call the addresses after
// releasing the lock, under the caller's engine/callback lifetime contract.
unsafe impl Send for Shell {}

pub(crate) fn configure(
	version: c_int,
	callbacks: *const PluginCallbacks,
	metadata: PluginMetadata,
) -> bool {
	let Some(state) = state(version).filter(|_| !callbacks.is_null()) else {
		return false;
	};
	with(state, |shell| {
		shell.callbacks = callbacks;
		shell.metadata = metadata;
	});
	true
}

unsafe extern "C" fn dispatch_all_loaded(version: c_int, plugin: *mut c_void) {
	let Some(state) = state(version) else { return };
	let callbacks = with(state, |shell| shell.callbacks);
	if !callbacks.is_null() {
		// SAFETY: Called by the native shell under the configured table lifetime.
		unsafe { ((*callbacks).all_plugins_loaded)(plugin) };
	}
}

unsafe extern "C" fn dispatch_level(version: c_int, map: *const c_char, init: bool) {
	let Some(state) = state(version) else { return };
	// SAFETY: Called by the native listener on the main thread for this map.
	unsafe { level_event(state, map, init) }
}

unsafe extern "C" fn dispatch_load(
	version: c_int,
	plugin: *mut c_void,
	id: c_int,
	api: *mut c_void,
	error: *mut c_char,
	length: usize,
	late: bool,
) -> bool {
	let Some(state) = state(version) else {
		return false;
	};
	// SAFETY: Called only by the compiler-owned shell on Metamod's main thread.
	unsafe { load(state, plugin, id, api, error, length, late) }
}

extern "C" fn dispatch_metadata(version: c_int, field: c_int) -> *const c_char {
	let Some(state) = state(version) else {
		return ptr::null();
	};
	with(state, |shell| match field {
		0 => shell.metadata.author,
		1 => shell.metadata.name,
		2 => shell.metadata.description,
		3 => shell.metadata.url,
		4 => shell.metadata.license,
		5 => shell.metadata.version,
		6 => shell.metadata.date,
		7 => shell.metadata.log_tag,
		_ => ptr::null(),
	})
}

unsafe extern "C" fn dispatch_pause(
	version: c_int,
	plugin: *mut c_void,
	error: *mut c_char,
	length: usize,
	paused: bool,
) -> bool {
	let Some(state) = state(version) else {
		return false;
	};
	// SAFETY: Called by the compiler-owned shell during native pause/unpause.
	unsafe { pause(state, plugin, error, length, paused) }
}

unsafe extern "C" fn dispatch_query(
	version: c_int,
	plugin: *mut c_void,
	error: *mut c_char,
	length: usize,
) -> bool {
	let Some(state) = state(version) else {
		return false;
	};
	let callbacks = with(state, |shell| shell.callbacks);
	if callbacks.is_null() {
		return false;
	}
	// SAFETY: The native shell supplies the writable error buffer and a live table.
	unsafe { ((*callbacks).query_running)(plugin, error, length) }
}

unsafe extern "C" fn dispatch_unload(
	version: c_int,
	plugin: *mut c_void,
	error: *mut c_char,
	length: usize,
) -> bool {
	let Some(state) = state(version) else {
		return true;
	};
	// SAFETY: Called by the compiler-owned shell during native unload.
	unsafe { unload(state, plugin, error, length) }
}

unsafe fn level_event(state: &Mutex<Shell>, map: *const c_char, init: bool) {
	let (callback, shutdown, context) = with(state, |shell| {
		shell.levels.advance();
		let active = shell.status.loaded && shell.levels.registered && !shell.status.paused;
		(
			if active && init {
				shell.levels.init
			} else {
				None
			},
			if active && !init {
				shell.levels.shutdown
			} else {
				None
			},
			shell.levels.context,
		)
	});
	if let Some(callback) = callback {
		// SAFETY: Registration requires context/callback to live until unload;
		// the native listener supplies a map string valid for this notification.
		unsafe { callback(context, map) };
	}
	if let Some(shutdown) = shutdown {
		// SAFETY: As for the init callback above; no state guard is held.
		unsafe { shutdown(context) };
	}
}

pub(crate) fn level_generation(version: c_int) -> Option<u64> {
	let state = state(version)?;
	with(state, |shell| {
		(shell.status.loaded && shell.levels.registered && !shell.levels.exhausted)
			.then_some(shell.levels.generation)
	})
}

pub(crate) fn listen(
	version: c_int,
	init: Option<LevelInitCallback>,
	shutdown: Option<LevelShutdownCallback>,
	context: *mut c_void,
) -> Result<*mut c_void, HookStatus> {
	let state = state(version).ok_or(HookStatus::UNSUPPORTED)?;
	with(state, |shell| shell.listen(init, shutdown, context))
}

unsafe fn load(
	state: &Mutex<Shell>,
	plugin: *mut c_void,
	id: c_int,
	api: *mut c_void,
	error: *mut c_char,
	length: usize,
	late: bool,
) -> bool {
	let Some((callbacks, generation)) = with(state, |shell| shell.begin_load(id, api)) else {
		return false;
	};
	// SAFETY: Configuring the shell requires this table and callback to remain
	// valid throughout the load. No state reference is held across the call.
	let loaded = unsafe { ((*callbacks).load)(plugin, api, error, length, late) };
	if !loaded {
		with(state, |shell| {
			if shell.status.generation == generation {
				shell.clear_load();
			}
		});
	}
	loaded
}

unsafe fn pause(
	state: &Mutex<Shell>,
	plugin: *mut c_void,
	error: *mut c_char,
	length: usize,
	paused: bool,
) -> bool {
	let (callbacks, before) = with(state, |shell| (shell.callbacks, shell.status));
	if callbacks.is_null() {
		return false;
	}
	// SAFETY: The configured callbacks remain valid during this native call.
	let accepted = unsafe {
		let callback = if paused {
			(*callbacks).pause
		} else {
			(*callbacks).unpause
		};
		callback(plugin, error, length)
	};
	if accepted {
		with(state, |shell| {
			if shell.status.loaded && shell.status.generation == before.generation {
				shell.status.paused = paused;
			}
		});
	}
	accepted
}

fn state(version: c_int) -> Option<&'static Mutex<Shell>> {
	match version {
		16 => Some(&STABLE),
		18 => Some(&DEV),
		_ => None,
	}
}

pub(crate) fn status(version: c_int) -> PluginStatus {
	state(version).map_or(PluginStatus::UNLOADED, |state| {
		with(state, |shell| shell.status)
	})
}

unsafe fn unload(
	state: &Mutex<Shell>,
	plugin: *mut c_void,
	error: *mut c_char,
	length: usize,
) -> bool {
	let (callbacks, before) = with(state, |shell| (shell.callbacks, shell.status));
	if !before.loaded {
		return true;
	}
	if callbacks.is_null() {
		return false;
	}
	// SAFETY: The configured callback remains live until unloading completes.
	let unloaded = unsafe { ((*callbacks).unload)(plugin, error, length) };
	if unloaded {
		with(state, |shell| {
			if shell.status.generation == before.generation {
				shell.clear_load();
			}
		});
	}
	unloaded
}

fn with<R>(state: &Mutex<Shell>, f: impl FnOnce(&mut Shell) -> R) -> R {
	// State transitions contain no external code or panicking operations.
	// Recovering the guard also keeps an FFI entry from unwinding if a test
	// or a future internal change poisons a lock.
	let mut state = state
		.lock()
		.unwrap_or_else(std::sync::PoisonError::into_inner);
	f(&mut state)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::cell::Cell;
	use std::ffi::CStr;

	static ACCEPT: PluginCallbacks = PluginCallbacks {
		load: accept_load,
		all_plugins_loaded: all_loaded,
		query_running: accept,
		unload: accept,
		pause: accept,
		unpause: accept,
	};

	static NATIVE: PluginCallbacks = PluginCallbacks {
		load: inspect_native_load,
		..ACCEPT
	};

	static REFUSE_CHANGES: PluginCallbacks = PluginCallbacks {
		unload: refuse,
		pause: refuse,
		unpause: refuse,
		..ACCEPT
	};

	static REFUSE_LOAD: PluginCallbacks = PluginCallbacks {
		load: refuse_load,
		..ACCEPT
	};

	static RELOAD_DURING_UNLOAD: PluginCallbacks = PluginCallbacks {
		unload: reload_during_unload,
		..ACCEPT
	};

	static UNLOAD_DURING_PAUSE: PluginCallbacks = PluginCallbacks {
		pause: unload_during_pause,
		..ACCEPT
	};

	#[repr(C)]
	struct ApiFixture {
		vtable: *const *const c_void,
		plugin: *mut c_void,
		listener: *mut c_void,
		version: c_int,
		unlocked: bool,
	}

	struct Counts {
		init: Cell<usize>,
		shutdown: Cell<usize>,
	}

	struct Reentry {
		state: *const Mutex<Shell>,
		api: *mut c_void,
	}

	unsafe extern "C" fn accept(_: *mut c_void, _: *mut c_char, _: usize) -> bool {
		true
	}

	unsafe extern "C" fn accept_load(
		plugin: *mut c_void,
		api: *mut c_void,
		_: *mut c_char,
		_: usize,
		late: bool,
	) -> bool {
		!plugin.is_null() && !api.is_null() && late
	}

	unsafe extern "C" fn add_listener(
		api: *mut c_void,
		plugin: *mut c_void,
		listener: *mut c_void,
	) {
		// SAFETY: Native AddListener receives the live fixture object.
		let api = unsafe { &mut *api.cast::<ApiFixture>() };
		api.plugin = plugin;
		api.listener = listener;
		api.unlocked = state(api.version).is_some_and(|state| state.try_lock().is_ok());
	}

	unsafe extern "C" fn all_loaded(_: *mut c_void) {}

	#[test]
	fn callback_reentry_cannot_clear_a_new_load_or_repause_an_unloaded_shell() {
		let mut shell = Shell::new();
		shell.callbacks = &RELOAD_DURING_UNLOAD;
		let state = Mutex::new(shell);
		let api = ptr::NonNull::<u8>::dangling().as_ptr().cast();
		let mut context = Reentry { state: &state, api };
		let plugin = (&raw mut context).cast();
		// SAFETY: The fixture owns context/state and uses only static callbacks;
		// all engine pointers remain opaque and are never dereferenced.
		unsafe {
			assert!(load(&state, plugin, 1, api, ptr::null_mut(), 0, true));
			assert!(unload(&state, plugin, ptr::null_mut(), 0));
			assert!(with(&state, |shell| shell.status.loaded
				&& shell.status.id == 99
				&& shell.status.generation == 2));
			with(&state, |shell| shell.callbacks = &UNLOAD_DURING_PAUSE);
			assert!(pause(&state, plugin, ptr::null_mut(), 0, true));
			assert!(with(&state, |shell| !shell.status.loaded
				&& !shell.status.paused
				&& !shell.levels.registered));
		}
	}

	#[test]
	fn compiler_owned_shells_route_both_apis_and_reset_notification_lifetimes() {
		// GetApiVersion precedes the destructor. Itanium has two destructor
		// entries; MSVC has one. Do not manufacture these native vtables.
		let load_slot = if cfg!(target_os = "windows") { 2 } else { 3 };
		for version in [16, 18] {
			let mut api_vtable = [ptr::null(); 32];
			let add_slot = if version == 16 { 12 } else { 11 };
			api_vtable[add_slot] = add_listener as *const c_void;
			let mut api = ApiFixture {
				vtable: api_vtable.as_ptr(),
				plugin: ptr::null_mut(),
				listener: ptr::null_mut(),
				version,
				unlocked: false,
			};
			let api_pointer = (&raw mut api).cast();
			let mut counts = Counts {
				init: Cell::new(0),
				shutdown: Cell::new(0),
			};
			let context = (&raw mut counts).cast();
			let mut metadata = Shell::new().metadata;
			metadata.author = c"shell contract".as_ptr();
			// SAFETY: All callbacks, metadata, API and context remain valid for
			// this serialized native-shell fixture; no engine objects are used.
			unsafe {
				let plugin = crate::plugin::cpp_metamod_plugin(version, &NATIVE, metadata);
				assert!(!plugin.is_null());
				let version_fn: unsafe extern "C" fn(*mut c_void) -> c_int =
					std::mem::transmute(slot(plugin, 0));
				assert_eq!(version_fn(plugin), version);
				let load_fn: unsafe extern "C" fn(
					*mut c_void,
					c_int,
					*mut c_void,
					*mut c_char,
					usize,
					bool,
				) -> bool = std::mem::transmute(slot(plugin, load_slot));
				let pause_fn: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> bool =
					std::mem::transmute(slot(plugin, load_slot + 4));
				let unpause_fn: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> bool =
					std::mem::transmute(slot(plugin, load_slot + 5));
				let unload_fn: unsafe extern "C" fn(*mut c_void, *mut c_char, usize) -> bool =
					std::mem::transmute(slot(plugin, load_slot + 3));
				let author_fn: unsafe extern "C" fn(*mut c_void) -> *const c_char =
					std::mem::transmute(slot(plugin, load_slot + 6));
				assert_eq!(CStr::from_ptr(author_fn(plugin)), c"shell contract");
				assert!(load_fn(plugin, 7, api_pointer, ptr::null_mut(), 0, true));
				assert_eq!(status(version).id, 7);
				assert_eq!(level_generation(version), None);
				assert_eq!(
					crate::plugin::cpp_metamod_listen_levels(version, None, None, context),
					HookStatus::INVALID_ARGUMENT
				);
				assert_eq!(
					crate::plugin::cpp_metamod_listen_levels(
						version,
						Some(init),
						Some(shutdown),
						context
					),
					HookStatus::INSTALLED
				);
				assert_eq!(api.plugin, plugin);
				assert!(
					api.unlocked,
					"native registration must not hold a Rust state guard"
				);
				assert!(!api.listener.is_null());
				assert_eq!(
					crate::plugin::cpp_metamod_listen_levels(version, Some(init), None, context),
					HookStatus::ALREADY_INSTALLED
				);
				let level_init: unsafe extern "C" fn(
					*mut c_void,
					*const c_char,
					*const c_char,
					*const c_char,
					*const c_char,
					bool,
					bool,
				) = std::mem::transmute(slot(api.listener, 4));
				let level_shutdown: unsafe extern "C" fn(*mut c_void) =
					std::mem::transmute(slot(api.listener, 5));
				level_init(
					api.listener,
					c"cp_dustbowl".as_ptr(),
					ptr::null(),
					ptr::null(),
					ptr::null(),
					false,
					false,
				);
				assert_eq!((level_generation(version), counts.init.get()), (Some(1), 1));
				assert!(pause_fn(plugin, ptr::null_mut(), 0));
				level_shutdown(api.listener);
				level_init(
					api.listener,
					c"ctf_2fort".as_ptr(),
					ptr::null(),
					ptr::null(),
					ptr::null(),
					false,
					false,
				);
				assert_eq!(
					(
						level_generation(version),
						counts.init.get(),
						counts.shutdown.get()
					),
					(Some(3), 1, 0)
				);
				assert!(unpause_fn(plugin, ptr::null_mut(), 0));
				level_shutdown(api.listener);
				assert_eq!(
					(level_generation(version), counts.shutdown.get()),
					(Some(4), 1)
				);
				with(state(version).unwrap(), |shell| {
					shell.levels.generation = u64::MAX
				});
				assert_eq!(level_generation(version), Some(u64::MAX));
				level_shutdown(api.listener);
				assert_eq!(level_generation(version), None);
				assert!(unload_fn(plugin, ptr::null_mut(), 0));
				assert!(!status(version).loaded);
				assert_eq!(level_generation(version), None);
				assert!(load_fn(plugin, 8, api_pointer, ptr::null_mut(), 0, true));
				assert_eq!(
					crate::plugin::cpp_metamod_listen_levels(version, Some(init), None, context),
					HookStatus::INSTALLED
				);
				assert_eq!(level_generation(version), Some(0));
				assert!(unload_fn(plugin, ptr::null_mut(), 0));
			}
		}
	}

	unsafe extern "C" fn init(context: *mut c_void, _: *const c_char) {
		// SAFETY: The fixture retains its counters throughout registration.
		let counts = unsafe { &*context.cast::<Counts>() };
		counts.init.set(counts.init.get() + 1);
	}

	unsafe extern "C" fn inspect_native_load(
		plugin: *mut c_void,
		api: *mut c_void,
		_: *mut c_char,
		_: usize,
		late: bool,
	) -> bool {
		if plugin.is_null() || api.is_null() || !late {
			return false;
		}
		// SAFETY: Only the native-vtable fixture installs this callback.
		let api = unsafe { &*api.cast::<ApiFixture>() };
		let Some(state) = state(api.version) else {
			return false;
		};
		let Ok(current) = state.try_lock() else {
			return false;
		};
		current.status.loaded && !current.status.paused && !current.api.is_null()
	}

	unsafe extern "C" fn refuse(_: *mut c_void, _: *mut c_char, _: usize) -> bool {
		false
	}

	unsafe extern "C" fn refuse_load(
		_: *mut c_void,
		_: *mut c_void,
		_: *mut c_char,
		_: usize,
		_: bool,
	) -> bool {
		false
	}

	#[test]
	fn refused_lifecycle_changes_preserve_only_the_current_load() {
		let mut shell = Shell::new();
		shell.callbacks = &REFUSE_LOAD;
		let state = Mutex::new(shell);
		let pointer = ptr::NonNull::<u8>::dangling().as_ptr().cast();
		// SAFETY: The callbacks inspect no engine pointers; the opaque address
		// is only checked for null. Tables stay valid throughout the test.
		unsafe {
			assert!(!load(&state, pointer, 1, pointer, ptr::null_mut(), 0, true));
			assert!(!with(&state, |shell| shell.status.loaded));
			with(&state, |shell| shell.callbacks = &REFUSE_CHANGES);
			assert!(load(&state, pointer, 2, pointer, ptr::null_mut(), 0, true));
			with(&state, |shell| {
				shell.listen(Some(init), None, ptr::null_mut())
			})
			.unwrap();
			assert!(!unload(&state, pointer, ptr::null_mut(), 0));
			assert!(with(&state, |shell| shell.status.loaded
				&& shell.levels.registered));
			assert!(!pause(&state, pointer, ptr::null_mut(), 0, true));
			assert!(!with(&state, |shell| shell.status.paused));
			with(&state, |shell| shell.status.generation = u64::MAX);
			assert!(!load(&state, pointer, 3, pointer, ptr::null_mut(), 0, true));
			assert!(with(&state, |shell| !shell.status.loaded
				&& !shell.levels.registered
				&& shell.api.is_null()));
		}
	}

	unsafe extern "C" fn reload_during_unload(
		plugin: *mut c_void,
		error: *mut c_char,
		length: usize,
	) -> bool {
		// SAFETY: The test owns this context and state through both callbacks.
		let context = unsafe { &*plugin.cast::<Reentry>() };
		let state = unsafe { &*context.state };
		if state.try_lock().is_err() {
			return false;
		}
		// SAFETY: Only opaque fixture addresses and static callbacks are used.
		unsafe { load(state, plugin, 99, context.api, error, length, true) }
	}

	unsafe extern "C" fn shutdown(context: *mut c_void) {
		// SAFETY: As for init.
		let counts = unsafe { &*context.cast::<Counts>() };
		counts.shutdown.set(counts.shutdown.get() + 1);
	}

	unsafe fn slot(object: *mut c_void, index: usize) -> *const c_void {
		// SAFETY: Fixtures only request declared slots of live compiler-built
		// virtual objects, using the platform-specific destructor slot count.
		unsafe { *(*object.cast::<*const *const c_void>()).add(index) }
	}

	unsafe extern "C" fn unload_during_pause(
		plugin: *mut c_void,
		error: *mut c_char,
		length: usize,
	) -> bool {
		// SAFETY: As for reload_during_unload.
		let context = unsafe { &*plugin.cast::<Reentry>() };
		let state = unsafe { &*context.state };
		if state.try_lock().is_err() {
			return false;
		}
		// SAFETY: The fixture's unload callback accepts without native calls.
		unsafe { unload(state, plugin, error, length) }
	}
}
