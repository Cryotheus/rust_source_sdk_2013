//! Tests of precaching through the engine, which checks names and the room in
//! each table first.

use super::*;
use crate::interfaces::{NetworkStringTables, ValveEngine};
use crate::server::{Game, Module};
use crate::test_support::leak;
use crate::test_support::server::{export, mock_server, null_server};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::{Cell, RefCell};
use std::ffi::{CString, c_char, c_int};
use std::ptr::null_mut;

/// A precache table, as the mock container holds them.
#[repr(C)]
struct TableObject {
	interface: sys::INetworkStringTable,
	name: &'static CStr,
	strings: RefCell<Vec<CString>>,
	max: c_int,
}

thread_local! {
	/// What each `Precache*` call was given: the table, the name, and whether
	/// to preload, in order.
	static PRECACHED: RefCell<Vec<(&'static CStr, CString, bool)>> = const { RefCell::new(Vec::new()) };

	/// Whether the engine's `Precache*` methods report failure.
	static REFUSES: Cell<bool> = const { Cell::new(false) };

	/// The mock container's tables.
	static TABLES: RefCell<Vec<*mut TableObject>> = const { RefCell::new(Vec::new()) };
}

/// Adds `name` to `table`, as the engine's `Precache*` methods do, and
/// returns its index, or -1 if [`REFUSES`] says to.
fn add(table: &'static CStr, name: *const c_char, preload: bool) -> c_int {
	// SAFETY: The wrappers pass a NUL-terminated name.
	let name = unsafe { CStr::from_ptr(name) }.to_owned();

	PRECACHED.with_borrow_mut(|precached| precached.push((table, name.clone(), preload)));

	if REFUSES.get() {
		return -1;
	}

	let table = find(table).expect("the wrapper found the table first");

	// SAFETY: Tables are leaked.
	let mut strings = unsafe { (*table).strings.borrow_mut() };
	let index = strings
		.iter()
		.position(|string| *string == name)
		.unwrap_or_else(|| {
			strings.push(name);
			strings.len() - 1
		});

	c_int::try_from(index).unwrap()
}

/// The table named `name`, or null.
fn find(name: &CStr) -> Option<*mut TableObject> {
	TABLES.with_borrow(|tables| {
		tables
			.iter()
			.copied()
			// SAFETY: Tables are leaked.
			.find(|&table| unsafe { (*table).name } == name)
	})
}

/// `INetworkStringTableContainer::FindTable`.
unsafe extern "C" fn find_table(
	_: *const sys::INetworkStringTableContainer,
	name: *const c_char,
) -> *mut sys::INetworkStringTable {
	// SAFETY: The wrapper passes a NUL-terminated name.
	find(unsafe { CStr::from_ptr(name) }).map_or(null_mut(), |table| table.cast())
}

/// Exports an engine and a container holding a table of at most `max`
/// strings for each of `tables`, holding `initial`.
fn mock_engine(tables: &[(&'static CStr, c_int)], initial: &[&CStr]) {
	// SAFETY: The vtables hold only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and each patch only writes slots of
	// the vtable being built.
	let (engine_vtable, container_vtable, table_vtable) = unsafe {
		(
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_PrecacheDecal).write(precache_decal);
					(&raw mut (*vtable).IVEngineServer_PrecacheGeneric).write(precache_generic);
					(&raw mut (*vtable).IVEngineServer_PrecacheModel).write(precache_model);
				},
			),
			mock_vtable::<sys::INetworkStringTableContainer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).INetworkStringTableContainer_FindTable).write(find_table);
				},
			),
			mock_vtable::<sys::INetworkStringTable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).INetworkStringTable_FindStringIndex).write(table_find);
					(&raw mut (*vtable).INetworkStringTable_GetMaxStrings).write(table_max);
					(&raw mut (*vtable).INetworkStringTable_GetNumStrings).write(table_len);
				},
			),
		)
	};

	let table_vtable = Box::leak(table_vtable);

	let tables = tables
		.iter()
		.map(|&(name, max)| {
			leak(TableObject {
				interface: sys::INetworkStringTable {
					vtable_: &raw const *table_vtable,
				},
				name,
				strings: RefCell::new(initial.iter().map(|&string| string.to_owned()).collect()),
				max,
			})
		})
		.collect();

	TABLES.set(tables);
	PRECACHED.take();
	REFUSES.set(false);

	export(
		Module::Engine,
		ValveEngine::VERSION,
		leak(sys::IVEngineServer {
			vtable_: Box::leak(engine_vtable),
		}),
	);
	export(
		Module::Engine,
		NetworkStringTables::VERSION,
		leak(sys::INetworkStringTableContainer {
			vtable_: Box::leak(container_vtable),
		}),
	);
}

#[test]
fn names_are_precached_into_their_tables() {
	mock_engine(
		&[
			(DECAL_PRECACHE, 8),
			(GENERIC_PRECACHE, 8),
			(MODEL_PRECACHE, 8),
		],
		&[c"world"],
	);

	let scope = ();
	let server = mock_server(&scope);

	assert_eq!(server.precache_model(c"models/box.mdl", true), Ok(1));
	assert_eq!(server.precache_model(c"models/box.mdl", false), Ok(1));
	assert_eq!(server.precache_model(c"world", false), Ok(0));
	assert_eq!(server.precache_decal(c"decals/scorch1", false), Ok(1));
	assert_eq!(server.precache_generic(c"particles/fire.pcf", true), Ok(1));
	assert_eq!(
		PRECACHED.take(),
		[
			(MODEL_PRECACHE, c"models/box.mdl".to_owned(), true),
			(MODEL_PRECACHE, c"models/box.mdl".to_owned(), false),
			(MODEL_PRECACHE, c"world".to_owned(), false),
			(DECAL_PRECACHE, c"decals/scorch1".to_owned(), false),
			(GENERIC_PRECACHE, c"particles/fire.pcf".to_owned(), true),
		]
	);

	REFUSES.set(true);
	assert_eq!(
		server.precache_model(c"models/box.mdl", false),
		Err(PrecacheError::Refused)
	);
}

#[test]
fn names_the_engine_stops_the_server_for_are_refused() {
	mock_engine(&[(MODEL_PRECACHE, 2)], &[c"world", c"models/box.mdl"]);

	let scope = ();
	let server = mock_server(&scope);

	for name in [
		c"",
		c" models/box.mdl",
		c"\tmodels/box.mdl",
		c"\xC3\xA9.mdl",
	] {
		assert_eq!(
			server.precache_model(name, false),
			Err(PrecacheError::InvalidName),
			"{name:?}"
		);
	}

	// The table is full, but holds the model already.
	assert_eq!(
		server.precache_model(c"models/crate.mdl", false),
		Err(PrecacheError::TableFull)
	);
	assert_eq!(server.precache_model(c"models/box.mdl", false), Ok(1));

	// Without a level loaded, there are no tables.
	assert_eq!(
		server.precache_decal(c"decals/scorch1", false),
		Err(PrecacheError::NoTable)
	);
	assert_eq!(PRECACHED.take().len(), 1);

	let scope = ();

	assert!(matches!(
		null_server(Game::TeamFortress2, &scope).precache_generic(c"particles/fire.pcf", false),
		Err(PrecacheError::Interface(_))
	));
}

/// `IVEngineServer::PrecacheDecal`.
unsafe extern "C" fn precache_decal(
	_: *mut sys::IVEngineServer,
	name: *const c_char,
	preload: bool,
) -> c_int {
	add(DECAL_PRECACHE, name, preload)
}

/// `IVEngineServer::PrecacheGeneric`.
unsafe extern "C" fn precache_generic(
	_: *mut sys::IVEngineServer,
	name: *const c_char,
	preload: bool,
) -> c_int {
	add(GENERIC_PRECACHE, name, preload)
}

/// `IVEngineServer::PrecacheModel`.
unsafe extern "C" fn precache_model(
	_: *mut sys::IVEngineServer,
	name: *const c_char,
	preload: bool,
) -> c_int {
	add(MODEL_PRECACHE, name, preload)
}

/// `INetworkStringTable::FindStringIndex`.
unsafe extern "C" fn table_find(this: *mut sys::INetworkStringTable, name: *const c_char) -> c_int {
	// SAFETY: Every table the container returns is a `TableObject`, and the
	// wrapper passes a NUL-terminated name.
	let (table, name) = unsafe { (&*this.cast::<TableObject>(), CStr::from_ptr(name)) };

	table
		.strings
		.borrow()
		.iter()
		.position(|string| string.as_c_str() == name)
		.map_or(
			c_int::from(sdk_raw::interfaces::network_string_tables::INVALID_STRING_INDEX),
			|index| c_int::try_from(index).unwrap(),
		)
}

/// `INetworkStringTable::GetNumStrings`.
unsafe extern "C" fn table_len(this: *const sys::INetworkStringTable) -> c_int {
	// SAFETY: As for `table_find`.
	let table = unsafe { &*this.cast::<TableObject>() };

	c_int::try_from(table.strings.borrow().len()).unwrap()
}

/// `INetworkStringTable::GetMaxStrings`.
unsafe extern "C" fn table_max(this: *const sys::INetworkStringTable) -> c_int {
	// SAFETY: As for `table_find`.
	unsafe { (*this.cast::<TableObject>()).max }
}
