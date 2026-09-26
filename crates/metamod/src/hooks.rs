//! Callbacks for each server frame, each level, and each message clients
//! send, through Metamod's hooks and listeners.

use crate::MetamodApi;
use crate::sys::plugin::{self as raw, HookStatus};
use source_sdk_2013::interfaces::ServerGameDll;
use source_sdk_2013::net::incoming::{
	HookTargetError, IncomingHandler, Verdict, hook_target, route_incoming,
};
use source_sdk_2013::{Server, ServerBinding};
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};

/// Why a hook or listener could not be installed.
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

	#[error("this Metamod version has no plugin shell")]
	Unsupported,
}

impl HookError {
	pub(crate) fn check(status: HookStatus) -> Result<(), Self> {
		match status {
			HookStatus::INSTALLED => Ok(()),
			HookStatus::NOT_BOUND => Err(Self::NotBound),
			HookStatus::ALREADY_INSTALLED => Err(Self::AlreadyInstalled),
			HookStatus::REFUSED => Err(Self::Refused),
			HookStatus::INVALID_ARGUMENT => Err(Self::InvalidArgument),
			_ => Err(Self::Unsupported),
		}
	}
}

/// Runs once per server frame, before the game simulates it.
///
/// `simulating` is false while the server is paused or has no players, when
/// the game runs no entity logic that frame.
pub type GameFrameFn = fn(server: Server<'_>, simulating: bool);

/// Metamod's notifications about levels.
#[derive(Debug, Clone, Copy, Default)]
pub struct LevelEvents {
	/// A level is loading, before the game's own `LevelInit`, with its map's
	/// name. No entity of the level exists yet.
	pub init: Option<fn(server: Server<'_>, map: &CStr)>,

	/// The level is shutting down, before the game's own `LevelShutdown`. Its
	/// entities still exist, but are freed right after.
	pub shutdown: Option<fn(server: Server<'_>)>,
}

/// A callback and the server it runs for, kept for the shell's hooks.
struct Route<T>(Cell<Option<(ServerBinding, T)>>);

// SAFETY: Only the server's main thread reaches a route: the hooks run there,
// and the functions setting them take a `MetamodApi`, which is confined to it.
unsafe impl<T> Sync for Route<T> {}

impl<T: Copy> Route<T> {
	const fn new() -> Self {
		Self(Cell::new(None))
	}

	fn context(&'static self) -> *mut c_void {
		ptr::from_ref(self).cast_mut().cast()
	}

	/// # Safety
	///
	/// `context` must be what [`Route::context`] returned for a route of `T`.
	unsafe fn from_context(context: *mut c_void) -> Option<(ServerBinding, T)> {
		// SAFETY: As the caller promises.
		unsafe { &*context.cast::<Self>() }.0.get()
	}
}

static GAME_FRAME: Route<GameFrameFn> = Route::new();
static LEVELS: Route<LevelEvents> = Route::new();
static NET_MESSAGES: Route<&'static dyn IncomingHandler> = Route::new();

/// Why clients' messages could not be hooked.
#[derive(Debug, thiserror::Error)]
pub enum NetMessageHookError {
	#[error(transparent)]
	Target(#[from] HookTargetError),

	#[error(transparent)]
	Hook(#[from] HookError),
}

impl MetamodApi<'_> {
	/// Calls `callback` once per server frame, before the game's own frame.
	///
	/// This hooks `IServerGameDLL::GameFrame`. The hook stops calling back while
	/// the plugin is paused and when it unloads, and Metamod removes it after
	/// unloading the plugin. Install it while loading.
	pub fn hook_game_frame(
		self,
		game_dll: ServerGameDll<'_>,
		binding: ServerBinding,
		callback: GameFrameFn,
	) -> Result<(), HookError> {
		GAME_FRAME.0.set(Some((binding, callback)));

		// SAFETY: A `MetamodApi` only exists during a callback, on the main
		// thread. `game_dll` is the game's interface, and the callback is a
		// function of this library, which only reads a static.
		let status = unsafe {
			raw::cpp_metamod_hook_game_frame(
				self.version().plugin_api_version(),
				game_dll.as_ptr().cast(),
				game_frame,
				GAME_FRAME.context(),
			)
		};

		HookError::check(status)
	}

	/// Passes Metamod's level notifications to `events`.
	///
	/// This registers an `IMetamodListener`. It stops calling back while the
	/// plugin is paused and when it unloads, and Metamod removes it after
	/// unloading the plugin.
	pub fn listen_level_events(
		self,
		binding: ServerBinding,
		events: LevelEvents,
	) -> Result<(), HookError> {
		LEVELS.0.set(Some((binding, events)));

		// SAFETY: As for `hook_game_frame`, with the level callbacks.
		let status = unsafe {
			raw::cpp_metamod_listen_levels(
				self.version().plugin_api_version(),
				events.init.map(|_| level_init as raw::LevelInitCallback),
				events
					.shutdown
					.map(|_| level_shutdown as raw::LevelShutdownCallback),
				LEVELS.context(),
			)
		};

		HookError::check(status)
	}

	/// Passes every message a client sends to `handler`, before the engine
	/// processes it, and drops the ones it blocks.
	///
	/// This hooks each `Process*` method of the engine's client message
	/// handler, which [`hook_target`] finds and checks. The hooks stop calling
	/// back while the plugin is paused and when it unloads, and Metamod removes
	/// them after unloading the plugin. The engine creates its client objects
	/// as players first connect, so this fails with
	/// [`HookTargetError::NotReady`] until someone has.
	pub fn hook_net_messages(
		self,
		server: Server<'_>,
		binding: ServerBinding,
		handler: &'static dyn IncomingHandler,
	) -> Result<(), NetMessageHookError> {
		let target = hook_target(server)?;

		NET_MESSAGES.0.set(Some((binding, handler)));

		// SAFETY: As for `hook_game_frame`. The target is a live handler of the
		// engine's, whose methods at the slots each take a message and return
		// `bool`.
		let status = unsafe {
			raw::cpp_metamod_hook_net_messages(
				self.version().plugin_api_version(),
				target.handler.as_ptr(),
				&target.slots,
				net_message,
				NET_MESSAGES.context(),
			)
		};

		Ok(HookError::check(status)?)
	}
}

/// Runs `f` with a server for the current call from the engine. A panic is
/// caught: it must not unwind into the engine, and the panic hook reports it.
fn with_server(binding: ServerBinding, f: impl FnOnce(Server<'_>)) {
	let scope = ();

	// SAFETY: The shell calls back from a hook or listener, on the server's
	// main thread, during a single call from the engine.
	let server = unsafe { binding.server(&scope) };

	catch_unwind(AssertUnwindSafe(|| f(server))).ok();
}

/// The shell's `GameFrame` callback.
unsafe extern "C" fn game_frame(context: *mut c_void, simulating: bool) {
	// SAFETY: `hook_game_frame` passes the route's context.
	if let Some((binding, callback)) = unsafe { Route::<GameFrameFn>::from_context(context) } {
		with_server(binding, |server| callback(server, simulating));
	}
}

/// The shell's `OnLevelInit` callback.
unsafe extern "C" fn level_init(context: *mut c_void, map: *const c_char) {
	// SAFETY: `listen_level_events` passes the route's context.
	let Some((binding, events)) = (unsafe { Route::<LevelEvents>::from_context(context) }) else {
		return;
	};

	let (Some(init), false) = (events.init, map.is_null()) else {
		return;
	};

	// SAFETY: Metamod passes the engine's map name, which lasts for the call.
	let map = unsafe { CStr::from_ptr(map) };

	with_server(binding, |server| init(server, map));
}

/// The shell's `OnLevelShutdown` callback.
unsafe extern "C" fn level_shutdown(context: *mut c_void) {
	// SAFETY: `listen_level_events` passes the route's context.
	if let Some((
		binding,
		LevelEvents {
			shutdown: Some(shutdown),
			..
		},
	)) = unsafe { Route::<LevelEvents>::from_context(context) }
	{
		with_server(binding, shutdown);
	}
}

/// The shell's callback for each message a client sends.
unsafe extern "C" fn net_message(
	context: *mut c_void,
	kind: c_int,
	handler: *mut c_void,
	message: *mut c_void,
) -> bool {
	// SAFETY: `hook_net_messages` passes the route's context.
	let route = unsafe { Route::<&'static dyn IncomingHandler>::from_context(context) };

	let (Some((binding, incoming)), Some(handler), Some(message)) =
		(route, NonNull::new(handler), NonNull::new(message))
	else {
		return false;
	};

	// SAFETY: The shell calls this from its hook on the handler method at
	// `kind`'s slot, before the method runs, on the main thread, with the
	// engine's handler and message. Panics are caught inside.
	unsafe { route_incoming(&binding, incoming, kind, handler, message) == Verdict::Block }
}
