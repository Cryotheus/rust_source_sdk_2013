//! Callbacks for each server frame, each level, and each message clients
//! send, through Metamod's hooks and listeners.

#[cfg(test)]
#[path = "tests/server_hooks.rs"]
mod tests;

use crate::MetamodApi;

use crate::hook::{
	Handler, HookAction, HookCall, HookError, HookId, HookTarget, HookTiming, VirtualFunction,
};

use crate::sys::plugin::{self as raw, HookStatus};
use source_sdk_2013::interfaces::ServerGameDll;

use source_sdk_2013::net::incoming::{
	HookTargetError, IncomingHandler, IncomingKind, Verdict, hook_target, route_incoming,
};

use source_sdk_2013::raw::interfaces::server_game_dll::{
	GAME_FRAME_SLOT, GameFrameFn as GameFrame, SET_SERVER_HIBERNATION_SLOT,
	SetServerHibernationFn as SetServerHibernation, THINK_SLOT, ThinkFn as Think,
};

use source_sdk_2013::raw::net::incoming::ProcessMessageFn as ProcessMessage;
use source_sdk_2013::{Server, ServerBinding};
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};

/// Runs once per server frame: before the game's own frame when installed with
/// [`MetamodApi::hook_game_frame`], and after it when installed with
/// [`MetamodApi::hook_game_frame_post`].
///
/// `simulating` can be false while the server is paused or empty, and during
/// startup before the engine has client slots. Do not assume client slots are
/// available just because this callback runs.
pub type GameFrameFn = fn(server: Server<'_>, simulating: bool);

/// Runs as the server starts or stops hibernating, after the game has, when
/// installed with [`MetamodApi::hook_server_hibernation`].
///
/// `hibernating` is true as the server starts hibernating, with no human
/// players left, and false as it wakes.
pub type HibernationFn = fn(server: Server<'_>, hibernating: bool);

/// Runs before the game's own `Think`, which the engine calls every frame,
/// even when no level is loaded, when installed with
/// [`MetamodApi::hook_server_think`].
///
/// `final_tick` is the engine's `finalTick`, true on the last tick of a
/// frame.
pub type ServerThinkFn = fn(server: Server<'_>, final_tick: bool);

/// `IServerGameDLL::GameFrame`, which runs the game's frame.
const GAME_FRAME: VirtualFunction<GameFrame> = VirtualFunction::new(GAME_FRAME_SLOT);

/// `IServerGameDLL::SetServerHibernation`, which tells the game the server
/// started or stopped hibernating.
const SET_SERVER_HIBERNATION: VirtualFunction<SetServerHibernation> =
	VirtualFunction::new(SET_SERVER_HIBERNATION_SLOT);

/// `IServerGameDLL::Think`, which the engine calls once per frame, even
/// without a level.
const THINK: VirtualFunction<Think> = VirtualFunction::new(THINK_SLOT);

static GAME_FRAMES: Route<GameFrameFn> = Route::new();
static GAME_FRAMES_POST: Route<GameFrameFn> = Route::new();
static HIBERNATION: Route<HibernationFn> = Route::new();
static LEVELS: LevelRoute = LevelRoute(Cell::new(None));

/// The handler of each kind of message, in [`IncomingKind::ALL`]'s order.
static NET_MESSAGE_KINDS: [NetMessageKind; IncomingKind::ALL.len()] = {
	let mut kinds = [const { NetMessageKind(0) }; IncomingKind::ALL.len()];
	let mut kind = 0;

	while kind < kinds.len() {
		kinds[kind] = NetMessageKind(kind as c_int);
		kind += 1;
	}

	kinds
};

static NET_MESSAGES: Route<&'static dyn IncomingHandler, { IncomingKind::ALL.len() }> =
	Route::new();

static SERVER_THINKS: Route<ServerThinkFn> = Route::new();

/// Metamod's notifications about levels.
#[derive(Debug, Clone, Copy, Default)]
pub struct LevelEvents {
	/// Called after the game's own `LevelInit`, with the map's name.
	pub init: Option<fn(server: Server<'_>, map: &CStr)>,

	/// Called after the game's own `LevelShutdown`. Do not access the level's
	/// entities here; discard map-specific state instead.
	pub shutdown: Option<fn(server: Server<'_>)>,
}

/// The level callbacks and the server they run for, kept for the shell's
/// listener.
struct LevelRoute(Cell<Option<(ServerBinding, LevelEvents)>>);

impl LevelRoute {
	/// # Safety
	///
	/// `context` must be what [`LevelRoute::context`] returned.
	unsafe fn from_context(context: *mut c_void) -> Option<(ServerBinding, LevelEvents)> {
		// SAFETY: As the caller promises.
		unsafe { &*context.cast::<Self>() }.0.get()
	}

	fn context(&'static self) -> *mut c_void {
		ptr::from_ref(self).cast_mut().cast()
	}
}

// SAFETY: Only the server's main thread reaches it: the listener runs there,
// and `listen_level_events` takes a `MetamodApi`, which is confined to it.
unsafe impl Sync for LevelRoute {}

/// Why clients' messages could not be hooked.
#[derive(Debug, thiserror::Error)]
pub enum NetMessageHookError {
	#[error(transparent)]
	Target(#[from] HookTargetError),

	#[error(transparent)]
	Hook(#[from] HookError),
}

/// The handler of one kind of client message, by its index in
/// [`IncomingKind::ALL`].
struct NetMessageKind(c_int);

impl Handler<ProcessMessage> for NetMessageKind {
	fn call(&self, call: &HookCall<'_, ProcessMessage>) -> HookAction<bool> {
		// An earlier hook, such as SourceMod's, blocked it.
		if call.superseded() == Some(true) {
			return HookAction::Ignore;
		}

		let (message,) = call.args();

		let (Some(routed), Some(handler), Some(message)) = (
			NET_MESSAGES.get(),
			NonNull::new(call.this()),
			NonNull::new(message),
		) else {
			return HookAction::Ignore;
		};

		// SAFETY: The hook runs before the engine's handler method at this kind's
		// slot of the vtable `hook_target` found, on the main thread, with the
		// engine's handler and message. Panics are caught inside.
		match unsafe { route_incoming(&routed.binding, routed.target, self.0, handler, message) } {
			// A blocked message counts as processed, as if its handler returned
			// true.
			Verdict::Block => HookAction::Supersede(true),

			Verdict::Continue => HookAction::Ignore,
		}
	}
}

/// A callback and the server it runs for, kept for the hooks that run it.
struct Route<T, const HOOKS: usize = 1>(Cell<Option<Routed<T, HOOKS>>>);

impl<T: Copy, const HOOKS: usize> Route<T, HOOKS> {
	const fn new() -> Self {
		Self(Cell::new(None))
	}

	fn get(&self) -> Option<Routed<T, HOOKS>> {
		self.0.get()
	}

	/// Whether the route's hooks are installed, for this load of the plugin.
	fn installed(&self, api: MetamodApi<'_>) -> bool {
		self.get().is_some_and(|routed| {
			routed
				.hooks
				.into_iter()
				.flatten()
				.any(|hook| api.has_hook(hook))
		})
	}

	fn set(&self, hooks: [Option<HookId>; HOOKS], binding: ServerBinding, target: T) {
		self.0.set(Some(Routed {
			hooks,
			binding,
			target,
		}));
	}
}

impl Handler<GameFrame> for Route<GameFrameFn> {
	fn call(&self, call: &HookCall<'_, GameFrame>) -> HookAction<()> {
		let (simulating,) = call.args();

		if let Some(routed) = self.get() {
			with_server(routed.binding, |server| (routed.target)(server, simulating));
		}

		HookAction::Ignore
	}
}

// SAFETY: Only the server's main thread reaches a route: hooks only run their
// handlers there, and the functions setting them take a `MetamodApi`, which is
// confined to it.
unsafe impl<T, const HOOKS: usize> Sync for Route<T, HOOKS> {}

#[derive(Clone, Copy)]
struct Routed<T, const HOOKS: usize> {
	hooks: [Option<HookId>; HOOKS],
	binding: ServerBinding,
	target: T,
}

impl MetamodApi<'_> {
	/// Calls `callback` once per server frame, before the game's own frame.
	///
	/// This hooks `IServerGameDLL::GameFrame`. The hook stops calling back while
	/// the plugin is paused and when it unloads, and Metamod removes it after
	/// unloading the plugin. Install it while loading. To run after the frame
	/// instead, or as well, see [`Self::hook_game_frame_post`], which also
	/// describes when Metamod 2.0 starts running either hook late.
	pub fn hook_game_frame(
		self,
		game_dll: ServerGameDll<'_>,
		binding: ServerBinding,
		callback: GameFrameFn,
	) -> Result<(), HookError> {
		hook_game_dll(
			self,
			GAME_FRAME,
			&GAME_FRAMES,
			HookTiming::Pre,
			game_dll,
			binding,
			callback,
		)
	}

	/// Calls `callback` once per server frame, after the game's own frame.
	///
	/// This hooks `IServerGameDLL::GameFrame` after the call, apart from
	/// [`Self::hook_game_frame`]: either can be installed without the other.
	/// With both, each frame runs that callback, then the game's frame, then
	/// this one. This one also runs after a frame the game cut short, or that
	/// another plugin's hook skipped. Entities removed during the frame may
	/// already be freed by then.
	///
	/// The hook stops calling back while the plugin is paused and when it
	/// unloads, and Metamod removes it after unloading the plugin. Install it
	/// while loading. Under Metamod 2.0, when `GameFrame` is already detoured,
	/// by another plugin or by this plugin's other `GameFrame` hook, KHook adds
	/// the new hook from a worker thread, so it may miss the next few frames.
	/// Of the two hooks, the one installed second is always added this way, so
	/// callbacks that pair up must handle frames where only one of them ran.
	pub fn hook_game_frame_post(
		self,
		game_dll: ServerGameDll<'_>,
		binding: ServerBinding,
		callback: GameFrameFn,
	) -> Result<(), HookError> {
		hook_game_dll(
			self,
			GAME_FRAME,
			&GAME_FRAMES_POST,
			HookTiming::Post,
			game_dll,
			binding,
			callback,
		)
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
		if NET_MESSAGES.installed(self) {
			return Err(HookError::AlreadyInstalled.into());
		}

		let target = hook_target(server)?;
		let mut hooks = [None; IncomingKind::ALL.len()];

		for (kind, &slot) in target.slots.iter().enumerate() {
			let hooked = usize::try_from(slot)
				.map_err(|_| HookError::InvalidArgument)
				.and_then(|slot| {
					// SAFETY: As for `hook_game_dll`. The target is a live handler
					// of the engine's, whose methods at the slots each take a message
					// and return `bool`, and its class lasts as long as the engine.
					unsafe {
						self.add_hook(
							VirtualFunction::<ProcessMessage>::new(slot),
							HookTarget::class_of(target.handler),
							HookTiming::Pre,
							&NET_MESSAGE_KINDS[kind],
						)
					}
				});

			match hooked {
				Ok(hook) => hooks[kind] = Some(hook),

				Err(error) => {
					for hook in hooks.into_iter().flatten() {
						self.remove_hook(hook);
					}

					return Err(error.into());
				}
			}
		}

		NET_MESSAGES.set(hooks, binding, handler);
		Ok(())
	}

	/// Calls `callback` after the game learns the server started or stopped
	/// hibernating.
	///
	/// This hooks `IServerGameDLL::SetServerHibernation` after the call. The
	/// engine only calls it on a change, so a plugin loaded while the server
	/// hibernates is not told: work out the state at load as well. The hook
	/// stops calling back while the plugin is paused and when it unloads, and
	/// Metamod removes it after unloading the plugin. Install it while loading.
	pub fn hook_server_hibernation(
		self,
		game_dll: ServerGameDll<'_>,
		binding: ServerBinding,
		callback: HibernationFn,
	) -> Result<(), HookError> {
		hook_game_dll(
			self,
			SET_SERVER_HIBERNATION,
			&HIBERNATION,
			HookTiming::Post,
			game_dll,
			binding,
			callback,
		)
	}

	/// Calls `callback` before each call of the game's own `Think`.
	///
	/// This hooks `IServerGameDLL::Think`, which the engine calls even when no
	/// level is loaded, apart from [`Self::hook_game_frame`]. The hook stops
	/// calling back while the plugin is paused and when it unloads, and Metamod
	/// removes it after unloading the plugin. Install it while loading.
	pub fn hook_server_think(
		self,
		game_dll: ServerGameDll<'_>,
		binding: ServerBinding,
		callback: ServerThinkFn,
	) -> Result<(), HookError> {
		hook_game_dll(
			self,
			THINK,
			&SERVER_THINKS,
			HookTiming::Pre,
			game_dll,
			binding,
			callback,
		)
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

		// SAFETY: A `MetamodApi` only exists during a callback, on the main
		// thread. The callbacks are functions of this library, which only read a
		// static.
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

		match status {
			HookStatus::INSTALLED => Ok(()),
			HookStatus::NOT_BOUND => Err(HookError::NotBound),
			HookStatus::ALREADY_INSTALLED => Err(HookError::AlreadyInstalled),
			HookStatus::INVALID_ARGUMENT => Err(HookError::InvalidArgument),
			_ => Err(HookError::Unsupported),
		}
	}
}

/// Hooks `function`, a method of `IServerGameDLL` taking a `bool`, at
/// `timing`, and has the hook pass each call to `callback` through `route`.
fn hook_game_dll(
	api: MetamodApi<'_>,
	function: VirtualFunction<GameFrame>,
	route: &'static Route<GameFrameFn>,
	timing: HookTiming,
	game_dll: ServerGameDll<'_>,
	binding: ServerBinding,
	callback: GameFrameFn,
) -> Result<(), HookError> {
	if route.installed(api) {
		return Err(HookError::AlreadyInstalled);
	}

	let game_dll = NonNull::new(game_dll.as_ptr()).ok_or(HookError::InvalidArgument)?;

	// SAFETY: A `MetamodApi` only exists during a callback, on the main thread.
	// `game_dll` is the game's interface, which outlives the plugin, and has a
	// method of the function's signature at its slot.
	let hook = unsafe { api.add_hook(function, HookTarget::instance(game_dll), timing, route) }?;

	route.set([Some(hook)], binding, callback);
	Ok(())
}

/// The shell's `OnLevelInit` callback.
unsafe extern "C" fn level_init(context: *mut c_void, map: *const c_char) {
	// SAFETY: `listen_level_events` passes the route's context.
	let Some((binding, events)) = (unsafe { LevelRoute::from_context(context) }) else {
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
	)) = unsafe { LevelRoute::from_context(context) }
	{
		with_server(binding, shutdown);
	}
}

/// Runs `f` with a server for the current call from the engine. A panic is
/// caught: it must not unwind into the engine, and the panic hook reports it.
pub(crate) fn with_server(binding: ServerBinding, f: impl FnOnce(Server<'_>)) {
	let scope = ();

	// SAFETY: Hooks and listeners call back on the server's main thread, during
	// a single call from the engine.
	let server = unsafe { binding.server(&scope) };

	catch_unwind(AssertUnwindSafe(|| f(server))).ok();
}
