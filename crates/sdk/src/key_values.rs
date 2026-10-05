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
	/// `raw` must point to live key values that stay allocated, and keep their
	/// name, for `'k`, laid out as TF2's engine and game lay them out (see
	/// [`sdk_raw::key_values`]), used only on the server's main thread.
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

	/// The key values' name, such as the command a client sent, or `None` if it
	/// cannot be found: vstdlib, whose symbol table holds most names, is not
	/// loaded, or the key values keep their name in a table of their own that is
	/// not laid out as TF2's. Symbol tables keep the first spelling of each name
	/// they were given, and match names ignoring case, so compare names ignoring
	/// ASCII case.
	#[doc(alias("GetName"))]
	pub fn name(self) -> Option<&'k CStr> {
		let system = sdk_raw::key_values::key_values_system();

		// SAFETY: The key values are live, laid out as TF2's, and keep their name
		// for `'k`. The process's system keeps its names for as long as it runs.
		let name = unsafe { sdk_raw::key_values::key_name(system, self.raw) };

		// SAFETY: A name that is found is NUL-terminated, and lives for `'k`.
		unsafe { sdk_raw::util::cstr::borrow_cstr(name) }
	}
}
