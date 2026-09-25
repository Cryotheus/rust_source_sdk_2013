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

impl<'s> NetworkStringTables<'s> {
	/// Finds a table by name, such as `modelprecache` or `downloadables`.
	#[doc(alias = "FindTable")]
	pub fn find(self, name: &CStr) -> Option<NetworkStringTable<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live.
		NetworkStringTable::new(unsafe {
			vcall!(self.as_ptr() => INetworkStringTableContainer_FindTable(name.as_ptr()))
		})
	}

	/// The table with an ID, which ranges up to [`Self::len`].
	#[doc(alias = "GetTable")]
	pub fn get(self, id: usize) -> Option<NetworkStringTable<'s>> {
		let id = c_int::try_from(id).ok().filter(|&id| id < self.count())?;

		// SAFETY: As for `find`, and the ID is in range.
		NetworkStringTable::new(unsafe {
			vcall!(self.as_ptr() => INetworkStringTableContainer_GetTable(id))
		})
	}

	#[doc(alias = "GetNumTables")]
	pub fn len(self) -> usize {
		usize::try_from(self.count()).unwrap_or(0)
	}

	pub fn is_empty(self) -> bool {
		self.len() == 0
	}

	fn count(self) -> c_int {
		// SAFETY: As for `find`.
		unsafe { vcall!(self.as_ptr() => INetworkStringTableContainer_GetNumTables()) }
	}
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
	fn new(raw: *mut sys::INetworkStringTable) -> Option<Self> {
		Some(Self {
			raw: NonNull::new(raw)?,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		})
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::INetworkStringTable {
		self.raw.as_ptr()
	}

	#[doc(alias = "GetTableName")]
	pub fn name(self) -> &'s CStr {
		// SAFETY: Tables live until the level ends, which it does not during
		// `'s`, and never rename themselves.
		unsafe { borrow_cstr(vcall!(self.as_ptr() => INetworkStringTable_GetTableName())) }
			.unwrap_or_default()
	}

	#[doc(alias = "GetNumStrings")]
	pub fn len(self) -> usize {
		// SAFETY: As for `name`.
		usize::try_from(unsafe { vcall!(self.as_ptr() => INetworkStringTable_GetNumStrings()) })
			.unwrap_or(0)
	}

	pub fn is_empty(self) -> bool {
		self.len() == 0
	}

	/// The string at an index, which ranges up to [`Self::len`].
	#[doc(alias = "GetString")]
	pub fn string(self, index: usize) -> Option<CString> {
		let index = c_int::try_from(index)
			.ok()
			.filter(|&index| (index as usize) < self.len())?;

		// SAFETY: As for `name`, and the index is in range. The string is
		// copied, since adding strings may move the table's storage.
		unsafe { copy_cstr(vcall!(self.as_ptr() => INetworkStringTable_GetString(index))) }
	}

	/// The index of a string in the table.
	#[doc(alias = "FindStringIndex")]
	pub fn find(self, string: &CStr) -> Option<usize> {
		// SAFETY: As for `name`.
		let index = unsafe {
			vcall!(self.as_ptr() => INetworkStringTable_FindStringIndex(string.as_ptr()))
		};

		(index != INVALID_STRING_INDEX)
			.then(|| usize::try_from(index).ok())
			.flatten()
	}
}
