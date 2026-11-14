//! Timing of hook handlers, to find what slows a server's frames.
//!
//! While [`set_timing`] has it on, every handler's calls on the main thread
//! are counted and timed, by the handler's [`name`](super::Handler::name).
//! A handler's time includes that of the handlers of the hooked functions it
//! calls, directly or through the game.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// The handlers' times since timing was last turned on.
static TIMES: Mutex<Vec<HandlerTime>> = Mutex::new(Vec::new());

/// Whether handlers are timed.
static TIMING: AtomicBool = AtomicBool::new(false);

/// How long the calls of one handler took while timing was on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandlerTime {
	/// The handler's [`name`](super::Handler::name).
	pub name: &'static str,

	/// How many calls it handled.
	pub calls: u64,

	/// How long they took together.
	pub total: Duration,

	/// How long the longest of them took.
	pub longest: Duration,
}

/// The handlers' times since timing was last turned on, the longest total
/// first.
pub fn handler_times() -> Vec<HandlerTime> {
	let mut times = TIMES.lock().unwrap_or_else(PoisonError::into_inner).clone();

	times.sort_by_key(|time| std::cmp::Reverse(time.total));
	times
}

/// Whether handlers are timed.
pub fn is_timing() -> bool {
	TIMING.load(Ordering::Relaxed)
}

/// Turns timing on, starting from no calls, or off, keeping the times so
/// far.
pub fn set_timing(on: bool) {
	if on {
		TIMES.lock().unwrap_or_else(PoisonError::into_inner).clear();
	}

	TIMING.store(on, Ordering::Relaxed);
}

/// Runs `call`, a call of the handler `name` names, timing it while timing is
/// on.
pub(super) fn time<R>(name: impl FnOnce() -> &'static str, call: impl FnOnce() -> R) -> R {
	if !is_timing() {
		return call();
	}

	let started = Instant::now();
	let result = call();
	let took = started.elapsed();
	let name = name();
	let mut times = TIMES.lock().unwrap_or_else(PoisonError::into_inner);

	match times.iter_mut().find(|time| time.name == name) {
		Some(time) => {
			time.calls += 1;
			time.total += took;
			time.longest = time.longest.max(took);
		}

		None => times.push(HandlerTime {
			name,
			calls: 1,
			total: took,
			longest: took,
		}),
	}

	result
}
