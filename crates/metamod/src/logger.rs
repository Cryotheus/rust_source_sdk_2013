//! Reusable logging with owned messages and host-provided Rust sinks.
//!
//! Enable `logger` for message-only output, or `logger_pretty` for the optional
//! console renderer. Neither feature installs a global logger or touches the
//! engine. A sink must be safe on every logging thread. Use [`queue::LogQueue`]
//! for worker logs, then drain it from a current main-thread callback.
//!
//! The host owns callback TLS, generation checks, pause policy and unload. Join
//! workers and retire any global forwarding registration before unloading code;
//! `log::set_logger` retains a static reference and cannot unregister a plugin.
//! Create a new queue for every load and close the old queue before reuse. Do not
//! place engine handles or callback borrows in a sink or an owned record.
//!
//! A callback can forward drained records without storing engine access:
//!
//! ```
//! use metamod_source::{MetamodApi, logger::queue::LogQueue};
//! use std::ffi::c_void;
//! use std::ptr::NonNull;
//!
//! fn drain_in_callback(
//!     queue: &LogQueue,
//!     api: MetamodApi<'_>,
//!     plugin: NonNull<c_void>,
//! ) -> usize {
//!     // The queue lock is already released before calling the engine.
//!     let mut rejected = 0;
//!     for record in queue.drain() {
//!         match record.into_c_string() {
//!             Ok(message) => api.log_cstr(plugin, &message),
//!             Err(_) => rejected += 1, // Host chooses how to report interior NULs.
//!         }
//!     }
//!     rejected
//! }
//! ```
//!
//! Metamod's log sink can replicate to destinations without a verified terminal
//! identity. Keep those records plain. Pretty console output is for an independently
//! routed console sink; stdout capability is not an RCON client's capability.

pub mod queue;

#[cfg(feature = "logger_pretty")]
#[cfg_attr(docsrs, doc(cfg(feature = "logger_pretty")))]
pub mod pretty;

#[cfg(test)]
#[path = "tests/logger.rs"]
mod tests;

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::ffi::{CString, NulError};
use std::sync::Arc;

/// A direct logger that owns its Rust sink and does not install itself globally.
pub struct Logger<S> {
	sink: S,
	max_level: LevelFilter,
	rendering: Rendering,
}

impl<S> Logger<S> {
	/// Creates a logger emitting exactly the formatted message, without metadata
	/// or styling. The level limit is local; the host also manages the facade's
	/// global maximum level when it explicitly installs a forwarding logger.
	pub const fn new(sink: S, max_level: LevelFilter) -> Self {
		Self {
			sink,
			max_level,
			rendering: Rendering::Plain,
		}
	}

	/// Creates a logger using destination-aware console formatting.
	///
	/// Unknown and RCON destinations always use message-only output until a host
	/// can independently route a verified client. Console color policy never
	/// changes process environment or terminal modes.
	#[cfg(feature = "logger_pretty")]
	pub fn pretty(sink: S, max_level: LevelFilter, options: pretty::PrettyOptions) -> Self {
		Self {
			sink,
			max_level,
			rendering: Rendering::Pretty(options),
		}
	}

	/// Returns the host-owned sink, for example to inspect or drain a queue.
	pub const fn sink(&self) -> &S {
		&self.sink
	}
}

impl<S: Sink> Log for Logger<S> {
	fn enabled(&self, metadata: &Metadata<'_>) -> bool {
		metadata.level() <= self.max_level
	}

	fn flush(&self) {
		self.sink.flush();
	}

	fn log(&self, record: &Record<'_>) {
		if !self.enabled(record.metadata()) {
			return;
		}

		let text = match &self.rendering {
			Rendering::Plain => record.args().to_string(),

			#[cfg(feature = "logger_pretty")]
			Rendering::Pretty(options) => options.render(record),
		};
		self.sink.write(OwnedRecord {
			level: record.level(),
			text,
		});
	}
}

/// An owned message that contains no `fmt::Arguments`, engine pointer or
/// callback lifetime. Hosts may move it through bounded queues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedRecord {
	/// Original severity, retained independently of presentation.
	pub level: Level,
	/// Rendered text, without an automatically appended newline.
	pub text: String,
}

impl OwnedRecord {
	/// Converts the text for a native log sink, rejecting interior NULs.
	///
	/// This leaves error handling to the host instead of silently truncating a
	/// C string or panicking inside an engine callback.
	pub fn into_c_string(self) -> Result<CString, NulError> {
		CString::new(self.text)
	}
}

enum Rendering {
	Plain,
	#[cfg(feature = "logger_pretty")]
	Pretty(pretty::PrettyOptions),
}

/// A host-provided Rust destination safe to invoke from every logging thread.
///
/// A sink must not call an engine interface using saved callback state. Queue
/// owned records instead, then acquire current engine access when draining.
/// Avoid recursively logging to the same logger from `write` or `flush`.
pub trait Sink: Send + Sync {
	/// Flushes Rust output only. This may be called from a worker thread; engine
	/// queues must be drained explicitly in a main-thread callback.
	fn flush(&self) {}

	/// Accepts ownership of the message. Queueing sinks may drop records when
	/// full; their own observable statistics should disclose that.
	fn write(&self, record: OwnedRecord);
}

impl<S: Sink + ?Sized> Sink for Arc<S> {
	fn flush(&self) {
		(**self).flush();
	}

	fn write(&self, record: OwnedRecord) {
		(**self).write(record);
	}
}
