//! Key values the engine or game made (`KeyValues`), such as the commands
//! clients send as key values, read by name, along with the key values they
//! hold and their string values.

use crate::NotThreadSafe;
use std::ffi::CStr;
use std::iter::FusedIterator;
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
	/// `raw` must point to live key values that stay allocated, unchanged, for
	/// `'k`, along with the key values they hold, laid out as TF2's engine and
	/// game lay them out (see [`sdk_raw::key_values`]), used only on the
	/// server's main thread.
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

	/// The first of their own key values named `name`, ignoring ASCII case as
	/// tier1 does, or `None` if they hold none of that name. Key values whose
	/// name cannot be found (see [`Self::name`]) are skipped.
	#[doc(alias("FindKey"))]
	pub fn find_key(self, name: &CStr) -> Option<KeyValues<'k>> {
		self.sub_keys().find(|key| {
			key.name()
				.is_some_and(|found| found.to_bytes().eq_ignore_ascii_case(name.to_bytes()))
		})
	}

	/// The first of their own key values, or `None` if they hold none.
	#[doc(alias("GetFirstSubKey"))]
	pub fn first_sub_key(self) -> Option<KeyValues<'k>> {
		// SAFETY: The key values are live, laid out as TF2's, and keep their own
		// key values for `'k`.
		let key = unsafe { sdk_raw::key_values::first_sub_key(self.raw) };

		// SAFETY: Key values of the tree live as long as their parent.
		NonNull::new(key).map(|key| unsafe { Self::from_raw(key) })
	}

	/// The key values after these in their parent's list, or `None` after the
	/// last.
	#[doc(alias("GetNextKey"))]
	pub fn next_key(self) -> Option<KeyValues<'k>> {
		// SAFETY: As for `first_sub_key`.
		let key = unsafe { sdk_raw::key_values::next_key(self.raw) };

		// SAFETY: As for `first_sub_key`.
		NonNull::new(key).map(|key| unsafe { Self::from_raw(key) })
	}

	/// Their value, if it is a string. tier1's `GetString` would also convert a
	/// number or a pointer, and keep the string in its place; this only reads.
	#[doc(alias("GetString"))]
	pub fn string(self) -> Option<&'k CStr> {
		// SAFETY: As for `first_sub_key`, with their value kept for `'k`.
		let string = unsafe { sdk_raw::key_values::string_value(self.raw) };

		// SAFETY: A string value is NUL-terminated, and lives for `'k`.
		unsafe { sdk_raw::util::cstr::borrow_cstr(string) }
	}

	/// Their own key values, in order.
	pub fn sub_keys(self) -> SubKeys<'k> {
		SubKeys {
			next: self.first_sub_key(),
		}
	}
}

/// The key values that key values hold, in order, from
/// [`KeyValues::sub_keys`].
#[derive(Debug, Clone)]
pub struct SubKeys<'k> {
	next: Option<KeyValues<'k>>,
}

impl FusedIterator for SubKeys<'_> {}

impl<'k> Iterator for SubKeys<'k> {
	type Item = KeyValues<'k>;

	fn next(&mut self) -> Option<Self::Item> {
		let key = self.next?;

		self.next = key.next_key();
		Some(key)
	}
}
