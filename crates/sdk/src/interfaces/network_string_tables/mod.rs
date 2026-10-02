//! `INetworkStringTableContainer`, the string tables the server replicates to clients.

use crate::ffi::{NotThreadSafe, borrow_cstr, copy_cstr, vcall};
use std::ffi::{CStr, CString, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// `INVALID_STRING_INDEX` from `public/networkstringtabledefs.h`.
const INVALID_STRING_INDEX: c_int = u16::MAX as c_int;

interface! {
	/// The string tables the server replicates to clients (`INetworkStringTableContainer`).
	#[doc(alias = "INetworkStringTableContainer")]
	pub struct NetworkStringTables(sys::INetworkStringTableContainer) = Engine c"VEngineServerStringTable001";
}

/// One of the server's network string tables (`INetworkStringTable`).
///
/// The engine recreates its tables for every level.
#[doc(alias = "INetworkStringTable")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetworkStringTable<'s> {
	raw: NonNull<sys::INetworkStringTable>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> NetworkStringTable<'s> {
	/// Wraps a table the engine returned.
	///
	/// # Safety
	///
	/// `raw` must point to one of the engine's tables for the current level,
	/// the level must not end during `'s`, and the handle must be used only on
	/// the server's main thread.
	const unsafe fn from_raw(raw: NonNull<sys::INetworkStringTable>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::INetworkStringTable {
		self.raw.as_ptr()
	}

	/// The index of a string in the table, or `None` if the table does not
	/// contain it.
	#[doc(alias = "FindStringIndex")]
	pub fn find(self, string: &CStr) -> Option<usize> {
		// SAFETY: As for `name`.
		let index = unsafe {
			vcall!(self.as_ptr() => INetworkStringTable_FindStringIndex(string.as_ptr()))
		};

		usize::try_from(index)
			.ok()
			.filter(|_| index != INVALID_STRING_INDEX)
	}

	/// Whether the table holds no strings.
	pub fn is_empty(self) -> bool {
		self.len() == 0
	}

	/// The number of strings in the table.
	#[doc(alias = "GetNumStrings")]
	pub fn len(self) -> usize {
		// SAFETY: As for `name`.
		usize::try_from(unsafe { vcall!(self.as_ptr() => INetworkStringTable_GetNumStrings()) })
			.unwrap_or(0)
	}

	/// The table's name, such as `modelprecache`, or an empty string if the
	/// engine reports none.
	#[doc(alias = "GetTableName")]
	pub fn name(self) -> &'s CStr {
		// SAFETY: Tables live until the level ends, which it does not during
		// `'s`, and never rename themselves.
		unsafe { borrow_cstr(vcall!(self.as_ptr() => INetworkStringTable_GetTableName())) }
			.unwrap_or_default()
	}

	/// The string at an index, which ranges up to [`Self::len`], or `None` if
	/// the index is out of range or the engine returns no string.
	#[doc(alias = "GetString")]
	pub fn string(self, index: usize) -> Option<CString> {
		if index >= self.len() {
			return None;
		}

		let index = c_int::try_from(index).ok()?;

		// SAFETY: As for `name`, and the index is in range. The string is
		// copied, since adding strings may move the table's storage.
		unsafe { copy_cstr(vcall!(self.as_ptr() => INetworkStringTable_GetString(index))) }
	}
}

impl<'s> NetworkStringTables<'s> {
	/// The number of tables, as the engine reports it.
	fn count(self) -> c_int {
		// SAFETY: As for `find`.
		unsafe { vcall!(self.as_ptr() => INetworkStringTableContainer_GetNumTables()) }
	}

	/// Finds a table by name, such as `modelprecache` or `downloadables`, or
	/// returns `None` if no table has that name.
	#[doc(alias = "FindTable")]
	pub fn find(self, name: &CStr) -> Option<NetworkStringTable<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let table = unsafe {
			vcall!(self.as_ptr() => INetworkStringTableContainer_FindTable(name.as_ptr()))
		};

		// SAFETY: The engine returns null or one of its tables for the current
		// level, which `Server::new` guarantees does not end during `'s`, and
		// the caller is on the main thread.
		NonNull::new(table).map(|table| unsafe { NetworkStringTable::from_raw(table) })
	}

	/// The table with an ID, which ranges up to [`Self::len`], or `None` if the
	/// ID is out of range.
	#[doc(alias = "GetTable")]
	pub fn get(self, id: usize) -> Option<NetworkStringTable<'s>> {
		let id = c_int::try_from(id).ok().filter(|&id| id < self.count())?;

		// SAFETY: As for `find`, and the ID is in range.
		let table = unsafe { vcall!(self.as_ptr() => INetworkStringTableContainer_GetTable(id)) };

		// SAFETY: As for `find`.
		NonNull::new(table).map(|table| unsafe { NetworkStringTable::from_raw(table) })
	}

	/// Whether the server has no tables.
	pub fn is_empty(self) -> bool {
		self.len() == 0
	}

	/// The number of tables, which bounds the IDs [`Self::get`] takes.
	#[doc(alias = "GetNumTables")]
	pub fn len(self) -> usize {
		usize::try_from(self.count()).unwrap_or(0)
	}
}
