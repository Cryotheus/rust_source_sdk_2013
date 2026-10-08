//! Watching the console output of the whole process, line by line, as it is
//! printed.
//!
//! tier0 passes everything printed through `Msg`, `Warning`, `Log`, `Error`,
//! `ConMsg`, `ConDMsg`, `DevMsg`, `DevWarning` and their kin, by the engine,
//! the game and plugins alike, to one *spew function*. The dedicated server's
//! spew function shows each line in the server's window, in rcon's reply while
//! rcon runs a command, in `con_logfile`, and in the server's log. [`watch`]
//! puts a function of the plugin's in front of it, which sees every line and
//! then passes it on unchanged.
//!
//! # What a watch sees
//!
//! - Every line any thread prints through tier0, once tier0 decides to print
//!   it: tier0 drops `DevMsg` and `DevWarning` lines unless `developer` is at
//!   least 1, and `ConDMsg` lines unless it is at least 2, before any spew
//!   function sees them.
//! - Text as printed, which may be part of a line or several lines, with or
//!   without a trailing newline.
//! - Not what the server sends to one player: replies the game makes through
//!   `ClientPrint` (a `TextMsg` user message), `IVEngineServer::ClientPrintf`,
//!   and chat. Those never reach the server console.
//!
//! # Other plugins
//!
//! tier0 keeps one spew function, so plugins that watch it chain: each saves
//! the function it replaced and calls it. Restoring a saved function blindly
//! would unhook every plugin that chained after, and leaving a function of an
//! unloaded library installed, or saved by another plugin, would crash the
//! server on its next line.
//!
//! So a watch never hands tier0 its own function. It hands over a
//! [`JumpStub`], executable memory outside every library, which jumps to the
//! watch's function. When the watch stops, it points the stub at the function
//! it replaced, and makes that tier0's spew function again only if the stub is
//! still tier0's. Whoever saved the stub meanwhile, such as a plugin that
//! chained after, or SourceMod while it runs a command for one of its own
//! plugins, keeps a working function. The stub is never freed, and costs two
//! pages.

#[cfg(test)]
#[path = "tests/spew.rs"]
mod tests;

use crate::server::Server;
use sdk_raw::tier0::{SpewApi, SpewColor, SpewOutputFn, SpewRetval, SpewType, spew_api};
use sdk_raw::util::stub::JumpStub;
use std::cell::Cell;
use std::ffi::{CStr, c_char, c_void};
use std::io;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::{self, NonNull};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// How long [`SpewWatch::stop`] waits for lines other threads are still
/// passing through the watch's function.
const STOP_WAIT: Duration = Duration::from_secs(1);

/// How long [`SpewWatch::stop`] waits once no call counts itself inside the
/// watch's function, for any call between the stub and that count: one that
/// jumped through the stub before it was retargeted but has not counted
/// itself yet, or one that has counted itself out but not yet returned.
const SETTLE: Duration = Duration::from_millis(100);

/// The tier0 functions of the latest watch, leaked so that any thread can read
/// them without a lock.
static API: AtomicPtr<SpewApi> = AtomicPtr::new(ptr::null_mut());

/// The watch's callback, as a pointer, or null while nothing watches.
static CALLBACK: AtomicPtr<()> = AtomicPtr::new(ptr::null_mut());

/// The spew function the current watch replaced, or null for tier0's
/// default.
static PREVIOUS: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

/// How many calls are inside [`output`], on any thread.
static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// Whether a watch exists.
static WATCHING: AtomicBool = AtomicBool::new(false);

thread_local! {
	/// Whether this thread is running the callback, whose own output passes
	/// straight on. A `bool` needs no destructor, so no thread, the engine's
	/// included, gets one registered from this library.
	static IN_CALLBACK: Cell<bool> = const { Cell::new(false) };
}

/// Runs for each line of console output while a watch exists, on whichever
/// thread printed it. It must be quick, and should not print: output printed
/// from inside it passes straight on, unseen by it.
pub type SpewFn = fn(spew: &Spew<'_>);

/// Counts a call out of [`output`] when it returns.
struct Running;

impl Drop for Running {
	fn drop(&mut self) {
		RUNNING.fetch_sub(1, Ordering::SeqCst);
	}
}

/// One piece of console output, as tier0 printed it.
#[derive(Debug, Clone, Copy)]
pub struct Spew<'a> {
	kind: SpewKind,
	text: &'a CStr,
	group: &'a CStr,
	level: i32,
	color: Option<SpewColor>,
}

impl<'a> Spew<'a> {
	/// The colour tier0 gave the output, if any. The server's window colours
	/// lines by kind instead.
	pub fn color(&self) -> Option<SpewColor> {
		self.color
	}

	/// The spew group the output was printed in, such as `"developer"` or
	/// `"console"`, or empty.
	pub fn group(&self) -> &'a CStr {
		self.group
	}

	/// The kind of output.
	pub fn kind(&self) -> SpewKind {
		self.kind
	}

	/// The level within the group the output was printed at.
	pub fn level(&self) -> i32 {
		self.level
	}

	/// The text, which may be part of a line or several lines.
	pub fn text(&self) -> &'a CStr {
		self.text
	}
}

/// The kind of a piece of console output.
#[doc(alias("SpewType_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SpewKind {
	/// `Msg`, `ConMsg`, `DevMsg` and the like.
	Message,

	/// `Warning`, `DevWarning` and the like.
	Warning,

	/// A failed assertion.
	Assert,

	/// `Error`: a fatal error, after which the process exits.
	Error,

	/// `Log` and `ConLog`.
	Log,

	/// A kind tier0 has no name for.
	Other(i32),
}

impl From<SpewType> for SpewKind {
	fn from(kind: SpewType) -> Self {
		match kind {
			SpewType::MESSAGE => Self::Message,
			SpewType::WARNING => Self::Warning,
			SpewType::ASSERT => Self::Assert,
			SpewType::ERROR => Self::Error,
			SpewType::LOG => Self::Log,
			SpewType(other) => Self::Other(other),
		}
	}
}

/// A running watch of the console output, which [`watch`] starts.
///
/// Stop it before the plugin unloads, with [`Self::stop`] or by dropping it.
/// A watch kept in a static is never dropped, so stop that one explicitly.
/// While [`Self::stop`] returns [`Stopped::Busy`], keep the watch and the
/// library loaded, and stop it again later. The watch stays on the server's
/// main thread, where it started.
#[must_use = "dropping the watch stops it"]
#[derive(Debug)]
pub struct SpewWatch {
	api: SpewApi,
	stub: JumpStub,
	previous: Option<SpewOutputFn>,

	/// How the watch left tier0's chain, once it has.
	ended: Option<Stopped>,

	/// Whether no thread was left inside the watch's function after it ended.
	finished: bool,
	_main_thread: PhantomData<*const ()>,
}

impl SpewWatch {
	/// Takes the watch's function out of tier0's chain the first time, then
	/// waits for the threads still inside it.
	fn end(&mut self) -> Stopped {
		let ended = match self.ended {
			Some(ended) => ended,

			None => {
				let ended = self.leave();

				self.ended = Some(ended);
				ended
			}
		};

		if self.finished {
			return ended;
		}

		let start = Instant::now();

		loop {
			while RUNNING.load(Ordering::SeqCst) != 0 {
				if start.elapsed() >= STOP_WAIT {
					return Stopped::Busy;
				}

				std::thread::yield_now();
			}

			// A call that was between the stub and its count counts itself in by
			// now, or has returned, unless its thread was held up all this time.
			std::thread::sleep(SETTLE);

			if RUNNING.load(Ordering::SeqCst) == 0 {
				break;
			}
		}

		self.finished = true;
		WATCHING.store(false, Ordering::SeqCst);
		ended
	}

	/// Points the stub at the function the watch replaced, makes that tier0's
	/// spew function again if the stub still is, and stops calling back.
	fn leave(&mut self) -> Stopped {
		let api = self.api;

		// New calls through the stub skip the watch from here on.
		self.stub
			.retarget(function_address(previous_or_default(api, self.previous)));

		// SAFETY: tier0's functions, called on the main thread, where the
		// watch started.
		let stopped = if unsafe { (api.output)() }
			.is_some_and(|current| function_address(current) == self.stub.entry())
		{
			// SAFETY: As above; the function the watch replaced was tier0's
			// spew function, and is still callable.
			unsafe { (api.set_output)(self.previous) };
			Stopped::Restored
		} else {
			Stopped::Bypassed
		};

		CALLBACK.store(ptr::null_mut(), Ordering::SeqCst);
		stopped
	}

	/// Stops calling the watch's callback, and takes its function out of
	/// tier0's chain. See the [module](self) for how.
	///
	/// Lines other threads are printing through the watch's function
	/// meanwhile finish first, waiting at most a second, then a tenth of a
	/// second more for calls on their way in or out of it. If one is still
	/// inside it then, this returns [`Stopped::Busy`]: the watch is out of the
	/// chain, but the library must stay loaded until a later call returns how
	/// the watch ended. Do not stop a watch from inside its callback.
	pub fn stop(&mut self) -> Stopped {
		self.end()
	}
}

impl Drop for SpewWatch {
	fn drop(&mut self) {
		// A dropped watch cannot be stopped again, so a later one may start,
		// whose own stop waits for whatever thread is left inside.
		if !self.finished && self.end() == Stopped::Busy {
			WATCHING.store(false, Ordering::SeqCst);
		}
	}
}

/// How stopping a watch went.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stopped {
	/// The function the watch replaced is tier0's spew function again.
	Restored,

	/// Another function replaced the watch's after it started, so tier0's
	/// spew function stays as it is. The watch's stub, which that function
	/// may call, now jumps straight to the function the watch replaced.
	Bypassed,

	/// The watch is out of tier0's chain, as for [`Self::Restored`] or
	/// [`Self::Bypassed`], but a thread was still inside its function after a
	/// second. Keep the library loaded, and stop the watch again later.
	Busy,
}

/// Why console output could not be watched.
#[derive(Debug, thiserror::Error)]
pub enum WatchError {
	/// tier0 is not loaded, or lacks the spew functions.
	#[error("tier0's spew functions were not found")]
	Unavailable,

	/// A watch of this library's already exists.
	#[error("console output is already being watched")]
	AlreadyWatching,

	/// The jump stub could not be allocated.
	#[error("the jump stub could not be allocated: {0}")]
	Stub(#[from] io::Error),
}

/// The tier0 functions of the latest watch.
fn current_api() -> Option<SpewApi> {
	let api = API.load(Ordering::SeqCst);

	// SAFETY: `watch_with` leaks each value it stores, so it is never freed.
	(!api.is_null()).then(|| unsafe { *api })
}

/// The address of a spew function.
fn function_address(function: SpewOutputFn) -> NonNull<c_void> {
	NonNull::new(function as *mut c_void).expect("functions have addresses")
}

/// Passes a line to the callback, unless this thread is already running it.
///
/// # Safety
///
/// `message` must be null or point to a string that lasts for the call.
unsafe fn notify(kind: SpewType, message: *const c_char) {
	let callback = CALLBACK.load(Ordering::SeqCst);

	if callback.is_null() || message.is_null() {
		return;
	}

	// A thread being torn down counts as inside, and is skipped.
	if IN_CALLBACK.try_with(|inside| inside.replace(true)) != Ok(false) {
		return;
	}

	// SAFETY: `watch` stored a `SpewFn`.
	let callback = unsafe { std::mem::transmute::<*mut (), SpewFn>(callback) };

	// SAFETY: As the caller promises.
	let text = unsafe { CStr::from_ptr(message) };

	// SAFETY: tier0's functions, valid inside a spew function for its line.
	let (group, level, color) = match current_api() {
		Some(api) => unsafe {
			let group = (api.group)();
			let color = (api.color)();

			(
				if group.is_null() {
					c""
				} else {
					CStr::from_ptr(group)
				},
				(api.level)(),
				color.as_ref().copied(),
			)
		},

		None => (c"", 0, None),
	};

	let spew = Spew {
		kind: kind.into(),
		text,
		group,
		level,
		color,
	};

	catch_unwind(AssertUnwindSafe(|| callback(&spew))).ok();

	IN_CALLBACK.try_with(|inside| inside.set(false)).ok();
}

/// The watch's spew function, which the stub jumps to while the watch runs.
unsafe extern "C" fn output(kind: SpewType, message: *const c_char) -> SpewRetval {
	RUNNING.fetch_add(1, Ordering::SeqCst);

	let _running = Running;

	// SAFETY: tier0 passes the line it is spewing, which lasts for the call.
	unsafe { notify(kind, message) };

	let Some(api) = current_api() else {
		return SpewRetval::CONTINUE;
	};

	let previous = PREVIOUS.load(Ordering::SeqCst);

	let previous = if previous.is_null() {
		api.default
	} else {
		// SAFETY: `watch` stored the spew function it replaced.
		unsafe { std::mem::transmute::<*mut c_void, SpewOutputFn>(previous) }
	};

	// SAFETY: The function the watch replaced, called as tier0 called this
	// one. Its answer, such as breaking into the debugger for an assertion, is
	// passed back unchanged.
	unsafe { previous(kind, message) }
}

/// `previous`, or tier0's default spew function, which tier0 calls in place
/// of a null one.
fn previous_or_default(api: SpewApi, previous: Option<SpewOutputFn>) -> SpewOutputFn {
	previous.unwrap_or(api.default)
}

/// Calls `callback` for each piece of console output from now until the
/// returned watch stops, on whichever thread printed it, then passes the
/// output on to the spew function it replaced.
///
/// One watch per library may exist at a time. See the [module](self) for what
/// it sees, and how it shares tier0 with other plugins.
pub fn watch(server: Server<'_>, callback: SpewFn) -> Result<SpewWatch, WatchError> {
	// The server proves the call is on the main thread, where plugins change
	// tier0's spew function.
	let _ = server;

	// SAFETY: tier0's own functions.
	unsafe { watch_with(spew_api().ok_or(WatchError::Unavailable)?, callback) }
}

/// [`watch`], through the spew functions of `api`.
///
/// # Safety
///
/// `api` must hold functions with tier0's behavior and signatures, callable
/// from any thread for as long as the process runs, and the call must be on
/// the thread that changes the spew function.
unsafe fn watch_with(api: SpewApi, callback: SpewFn) -> Result<SpewWatch, WatchError> {
	if WATCHING.swap(true, Ordering::SeqCst) {
		return Err(WatchError::AlreadyWatching);
	}

	let stub = match JumpStub::new(function_address(output)) {
		Ok(stub) => stub,

		Err(error) => {
			WATCHING.store(false, Ordering::SeqCst);
			return Err(error.into());
		}
	};

	// SAFETY: tier0's functions, called on the main thread.
	let previous = unsafe { (api.output)() };

	PREVIOUS.store(
		previous.map_or(ptr::null_mut(), |function| {
			function_address(function).as_ptr()
		}),
		Ordering::SeqCst,
	);

	API.store(Box::into_raw(Box::new(api)), Ordering::SeqCst);
	CALLBACK.store(callback as *mut (), Ordering::SeqCst);

	// SAFETY: As above. The stub jumps to `output`, a spew function, until the
	// watch retargets it to the function it replaced, and is never freed.
	unsafe {
		(api.set_output)(Some(std::mem::transmute::<*mut c_void, SpewOutputFn>(
			stub.entry().as_ptr(),
		)));
	}

	Ok(SpewWatch {
		api,
		stub,
		previous,
		ended: None,
		finished: false,
		_main_thread: PhantomData,
	})
}
