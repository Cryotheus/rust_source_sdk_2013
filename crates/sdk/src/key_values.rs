//! Key values the engine or game made (`KeyValues`), such as the commands
//! clients send as key values, read by name.

use crate::NotThreadSafe;
use std::ffi::CStr;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// Key values another module owns, readable for `'k` (`KeyValues`).
///
/// Hooks on `IServerGameClients::ClientCommandKeyValues` get one, for the
/// command a client sent. Like other handles, it frees nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyValues<'k> {
	raw: NonNull<sys::KeyValues>,
	_scope: PhantomData<&'k ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'k> KeyValues<'k> {
	/// Wraps key values another module owns.
	///
	/// # Safety
	///
	/// `raw` must point to live key values that stay allocated for `'k`, named
	/// by a symbol of the process's key values system as the engine's and the
	/// game's are, used only on the server's main thread.
	pub const unsafe fn from_raw(raw: NonNull<sys::KeyValues>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the key values pointer, for calls this crate does not wrap.
	pub const fn as_ptr(self) -> *mut sys::KeyValues {
		self.raw.as_ptr()
	}

	/// The key values' name, such as the command a client sent, or `None` if
	/// vstdlib, whose symbol table holds it, is not loaded. The table keeps
	/// the first spelling of each name it was given, and matches names
	/// ignoring case, so compare names ignoring ASCII case.
	#[doc(alias("GetName"))]
	pub fn name(self) -> Option<&'k CStr> {
		let system = sdk_raw::key_values::key_values_system()?;

		// SAFETY: The key values are live, and named by a symbol of the
		// process's system, which keeps its names for as long as it runs.
		let name = unsafe { sdk_raw::key_values::key_name(system, self.raw) };

		// SAFETY: The system returns a NUL-terminated name from its table.
		unsafe { sdk_raw::util::cstr::borrow_cstr(name) }
	}
}
