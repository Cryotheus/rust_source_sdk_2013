//! Metamod's plugin list (`ISmmPluginManager`): the plugins it has loaded,
//! from which files, and whether each runs.

#[cfg(test)]
#[path = "tests/plugins.rs"]
mod tests;

use crate::api::MetamodApi;
use crate::sys::plugin_manager::{self as raw, ISmmPluginManager, PlStatus};
use std::ffi::{CStr, CString, c_char, c_int};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};

pub use crate::sys::plugin_manager::PluginId;

/// How many free IDs in a row [`PluginManager::plugins`] passes before it
/// stops looking.
const FREE_RUN: PluginId = 4096;

/// The longest reason [`PluginManager::query_running`] reads, with its NUL.
const REASON_BYTES: usize = 256;

/// Whether a plugin runs, as Metamod's `Pl_Status` says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginState {
	/// `Pl_Running`.
	Running,

	/// `Pl_Paused`: Metamod skips its hooks until it is unpaused.
	Paused,

	/// `Pl_Refused`: its `Load` refused, so it holds nothing.
	Refused,

	/// `Pl_Error`: its file is no plugin Metamod can load.
	Error,

	/// `Pl_NotFound`: its file was not found.
	NotFound,

	/// A status this crate does not know.
	Other(c_int),
}

impl PluginState {
	pub const fn from_raw(raw: PlStatus) -> Self {
		match raw {
			raw::PL_RUNNING => Self::Running,
			raw::PL_PAUSED => Self::Paused,
			raw::PL_REFUSED => Self::Refused,
			raw::PL_ERROR => Self::Error,
			raw::PL_NOT_FOUND => Self::NotFound,
			other => Self::Other(other),
		}
	}

	pub const fn to_raw(self) -> PlStatus {
		match self {
			Self::Running => raw::PL_RUNNING,
			Self::Paused => raw::PL_PAUSED,
			Self::Refused => raw::PL_REFUSED,
			Self::Error => raw::PL_ERROR,
			Self::NotFound => raw::PL_NOT_FOUND,
			Self::Other(other) => other,
		}
	}

	/// The state as `meta list` shows it, but for a running plugin, which it
	/// shows without one: `RUNNING`, `PAUSED`, `FAILED`, `ERROR`, `NOFILE`, or
	/// `-` for another.
	pub const fn name(self) -> &'static str {
		match self {
			Self::Running => "RUNNING",
			Self::Paused => "PAUSED",
			Self::Refused => "FAILED",
			Self::Error => "ERROR",
			Self::NotFound => "NOFILE",
			Self::Other(_) => "-",
		}
	}
}

/// What loaded a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PluginSource {
	/// `Pl_Console`: `meta load`.
	Console,

	/// `Pl_File`: Metamod's plugin files, as it starts.
	File,

	/// Another plugin, by its ID.
	Plugin(PluginId),

	/// A source this crate does not know.
	Other(PluginId),
}

impl PluginSource {
	pub const fn from_raw(raw: PluginId) -> Self {
		match raw {
			raw::PL_CONSOLE => Self::Console,
			raw::PL_FILE => Self::File,
			id if id >= raw::PL_MIN_ID => Self::Plugin(id),
			other => Self::Other(other),
		}
	}
}

/// A plugin in Metamod's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginEntry {
	pub id: PluginId,

	/// The file Metamod loaded it from, with the path it resolved.
	pub file: CString,
	pub state: PluginState,
	pub source: PluginSource,
}

/// Metamod's plugin manager, which lists, loads, pauses and unloads plugins.
///
/// Metamod keeps it for as long as it is loaded itself, which outlasts every
/// plugin, so a plugin may keep it until it unloads. Like the rest of
/// Metamod, it only works on the server's main thread, so it is neither
/// `Send` nor `Sync`.
#[derive(Debug, Clone, Copy)]
pub struct PluginManager {
	this: NonNull<ISmmPluginManager>,
	_not_send_or_sync: PhantomData<*mut ()>,
}

impl MetamodApi<'_> {
	/// `MetaFactory(MMIFACE_PLMANAGER)`: Metamod's plugin manager.
	pub fn plugin_manager(&self) -> Option<PluginManager> {
		let this = self.meta_interface(raw::MMIFACE_PLMANAGER)?;

		// SAFETY: Metamod gives its plugin manager by this name, and keeps it
		// while it is loaded, on the thread that called the callback.
		Some(unsafe { PluginManager::from_raw(this.cast()) })
	}
}

impl PluginManager {
	/// # Safety
	///
	/// `this` must be a live `ISmmPluginManager`, which stays live while the
	/// value or a copy of it is used, and is only used on the thread it was
	/// made on.
	pub const unsafe fn from_raw(this: NonNull<ISmmPluginManager>) -> Self {
		Self {
			this,
			_not_send_or_sync: PhantomData,
		}
	}

	pub const fn as_ptr(self) -> *mut ISmmPluginManager {
		self.this.as_ptr()
	}

	/// The plugin with `id`, or `None` when Metamod has none by it: none was
	/// loaded with it, or it was unloaded.
	#[doc(alias("Query"))]
	pub fn query(self, id: PluginId) -> Option<PluginEntry> {
		let mut file: *const c_char = ptr::null();
		let mut status = raw::PL_NOT_FOUND;
		let mut source = raw::PL_BAD_LOAD;

		// SAFETY: `from_raw`'s caller keeps the manager live. Fields are read
		// without forming references.
		let found = unsafe {
			let query = (&raw const (*(*self.as_ptr()).vtable).query).read();

			query(self.as_ptr(), id, &mut file, &mut status, &mut source)
		};

		if !found {
			return None;
		}

		let file = match file.is_null() {
			true => CString::default(),

			// SAFETY: Metamod's string for the plugin, copied at once.
			false => unsafe { CStr::from_ptr(file) }.to_owned(),
		};

		Some(PluginEntry {
			id,
			file,
			state: PluginState::from_raw(status),
			source: PluginSource::from_raw(source),
		})
	}

	/// Asks the plugin with `id` whether it runs, as its own `QueryRunning`
	/// answers, with the reason it gives when it does not, up to 255 bytes.
	/// Also fails, with Metamod's reason, when Metamod has no plugin by the
	/// ID, or the plugin has no plugin object, as when its file failed to
	/// load.
	#[doc(alias("QueryRunning"))]
	pub fn query_running(self, id: PluginId) -> Result<(), CString> {
		let mut reason = [0_u8; REASON_BYTES];

		// SAFETY: As for `query`, and the buffer's length is passed.
		let running = unsafe {
			let query_running = (&raw const (*(*self.as_ptr()).vtable).query_running).read();

			query_running(self.as_ptr(), id, reason.as_mut_ptr().cast(), REASON_BYTES)
		};

		if running {
			return Ok(());
		}

		// The last byte stays NUL, so a reason cut short still ends.
		reason[REASON_BYTES - 1] = 0;

		Err(CStr::from_bytes_until_nul(&reason)
			.unwrap_or_default()
			.to_owned())
	}

	/// Every plugin in Metamod's list, in the order of their IDs.
	///
	/// The interface lists no IDs, so this asks for each from the first, and
	/// stops once 4096 in a row are free. Each load takes the next ID, a
	/// failed one included, so plugins past a gap of more loads than that
	/// since are missed.
	pub fn plugins(self) -> Vec<PluginEntry> {
		let mut plugins = Vec::new();
		let mut free = 0;
		let mut id = raw::PL_MIN_ID;

		while free < FREE_RUN {
			match self.query(id) {
				Some(plugin) => {
					plugins.push(plugin);
					free = 0;
				}

				None => free += 1,
			}

			let Some(next) = id.checked_add(1) else {
				break;
			};

			id = next;
		}

		plugins
	}
}
