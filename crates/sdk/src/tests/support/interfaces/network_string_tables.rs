//! A mock `INetworkStringTableContainer` whose tables only count their
//! strings, which is enough for code that reads the room a table has left.

use super::super::leak;
use super::super::server::export;
use crate::interfaces::NetworkStringTables;
use crate::server::Module;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char, c_int};
use std::ptr::null_mut;

/// A table of the mock container, which the container returns as its
/// interface, the first field, so that the table's methods can cast it back.
#[repr(C)]
struct TableObject {
	interface: sys::INetworkStringTable,
	name: &'static CStr,
	len: Cell<c_int>,
	max: c_int,
}

thread_local! {
	/// The tables [`export_tables`] made on this thread.
	static TABLES: RefCell<Vec<*mut TableObject>> = const { RefCell::new(Vec::new()) };
}

/// `INetworkStringTableContainer::FindTable`.
unsafe extern "C" fn find_table(
	_: *const sys::INetworkStringTableContainer,
	name: *const c_char,
) -> *mut sys::INetworkStringTable {
	// SAFETY: The wrapper passes a NUL-terminated name.
	let name = unsafe { CStr::from_ptr(name) };

	TABLES.with_borrow(|tables| {
		tables
			.iter()
			.copied()
			// SAFETY: Tables are leaked.
			.find(|&table| unsafe { (*table).name } == name)
			.map_or(null_mut(), |table| table.cast())
	})
}

/// `INetworkStringTable::GetMaxStrings`.
unsafe extern "C" fn table_max(this: *const sys::INetworkStringTable) -> c_int {
	// SAFETY: Every table the container returns is a `TableObject`.
	unsafe { (*this.cast::<TableObject>()).max }
}

/// `INetworkStringTable::GetNumStrings`.
unsafe extern "C" fn table_len(this: *const sys::INetworkStringTable) -> c_int {
	// SAFETY: As for `table_max`.
	unsafe { (*this.cast::<TableObject>()).len.get() }
}

/// Exports a container of the tables `(name, strings held, most strings)`
/// names, to the engine of this thread's mock servers.
///
/// A table answers only `GetNumStrings` and `GetMaxStrings`, with the counts
/// given, which [`set_table_len`] changes, and any other call aborts the test.
/// A name that none of the tables has finds none.
///
/// For tests only.
pub fn export_tables(tables: &[(&'static CStr, c_int, c_int)]) {
	// SAFETY: The vtables hold only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and each patch only writes slots of
	// the vtable being built.
	let (container_vtable, table_vtable) = unsafe {
		(
			mock_vtable::<sys::INetworkStringTableContainer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).INetworkStringTableContainer_FindTable).write(find_table);
				},
			),
			mock_vtable::<sys::INetworkStringTable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).INetworkStringTable_GetMaxStrings).write(table_max);
					(&raw mut (*vtable).INetworkStringTable_GetNumStrings).write(table_len);
				},
			),
		)
	};

	let table_vtable = Box::leak(table_vtable);

	TABLES.set(
		tables
			.iter()
			.map(|&(name, len, max)| {
				leak(TableObject {
					interface: sys::INetworkStringTable {
						vtable_: &raw const *table_vtable,
					},
					name,
					len: Cell::new(len),
					max,
				})
			})
			.collect(),
	);

	export(
		Module::Engine,
		NetworkStringTables::VERSION,
		leak(sys::INetworkStringTableContainer {
			vtable_: Box::leak(container_vtable),
		}),
	);
}

/// Sets how many strings the table `name` holds, of those [`export_tables`]
/// made on this thread.
///
/// For tests only.
///
/// # Panics
///
/// If no such table exists.
pub fn set_table_len(name: &CStr, len: c_int) {
	TABLES.with_borrow(|tables| {
		let table = tables
			.iter()
			.copied()
			// SAFETY: Tables are leaked.
			.find(|&table| unsafe { (*table).name } == name)
			.expect("the mock container has the table");

		// SAFETY: As above.
		unsafe { (*table).len.set(len) };
	});
}
