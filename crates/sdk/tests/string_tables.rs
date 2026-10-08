//! Tests of the engine's network string tables
//! (`INetworkStringTableContainer`): additions, lookups, and the lock around
//! additions to `downloadables`.

use sdk_raw::interfaces::network_string_tables::INVALID_STRING_INDEX;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use source_sdk_2013::Module;
use source_sdk_2013::interfaces::network_string_tables::{
	AddDownloadableError, AddStringError, DOWNLOADABLES, MODEL_PRECACHE, SOUND_PRECACHE,
	add_downloadable,
};
use source_sdk_2013::interfaces::{NetworkStringTables, ValveEngine};
use source_sdk_2013::test_support::interfaces::valve_engine::{
	lock_network_string_tables, lock_requests, tables_locked,
};
use source_sdk_2013::test_support::leak;
use source_sdk_2013::test_support::server::{export, mock_server};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr::{null, null_mut};

thread_local! {
	/// Every call to the mock table's `AddString`, in order.
	static ADDITIONS: RefCell<Vec<Addition>> = const { RefCell::new(Vec::new()) };

	/// What the mock table's `AddString` returns.
	static ADD_RESULT: Cell<c_int> = const { Cell::new(0) };

	/// Every container and name passed to the mock container's `FindTable`,
	/// in order.
	static FIND_REQUESTS: RefCell<Vec<(*const sys::INetworkStringTableContainer, CString)>> =
		const { RefCell::new(Vec::new()) };

	/// What the mock container's `FindTable` returns.
	static FOUND_TABLE: Cell<*mut sys::INetworkStringTable> = const { Cell::new(null_mut()) };

	/// The strings the mock table's `FindStringIndex` finds, by index.
	static KNOWN_STRINGS: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };

	/// What the mock table's `GetMaxStrings` returns.
	static MAX_STRINGS: Cell<c_int> = const { Cell::new(0) };
}

/// A call to the mock table's `AddString`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Addition {
	this: *mut sys::INetworkStringTable,
	is_server: bool,
	string: *const c_char,
	length: c_int,
	user_data: *const c_void,
	/// Whether the mock engine's tables were locked during the call.
	locked: bool,
}

/// A leaked mock engine, string table container, and table.
struct Mocks {
	container: *mut sys::INetworkStringTableContainer,
	engine: *mut sys::IVEngineServer,
	table: *mut sys::INetworkStringTable,
}

impl Mocks {
	/// Builds the mocks and exports the engine and container from the engine
	/// factory of [`mock_server`].
	fn exported() -> Self {
		// SAFETY: The vtables hold only function pointers, `unexpected_call`
		// aborts whichever slot reaches it, and the patches only write slots of
		// the vtable being built.
		let (container_vtable, engine_vtable, table_vtable) = unsafe {
			(
				mock_vtable::<sys::INetworkStringTableContainer__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).INetworkStringTableContainer_FindTable)
							.write(find_table);
					},
				),
				mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IVEngineServer_LockNetworkStringTables)
							.write(lock_network_string_tables);
					},
				),
				mock_vtable::<sys::INetworkStringTable__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).INetworkStringTable_AddString).write(add_string);
						(&raw mut (*vtable).INetworkStringTable_FindStringIndex)
							.write(find_string_index);
						(&raw mut (*vtable).INetworkStringTable_GetMaxStrings).write(max_strings);
					},
				),
			)
		};

		let mocks = Self {
			container: leak(sys::INetworkStringTableContainer {
				vtable_: Box::leak(container_vtable),
			}),
			engine: leak(sys::IVEngineServer {
				vtable_: Box::leak(engine_vtable),
			}),
			table: leak(sys::INetworkStringTable {
				vtable_: Box::leak(table_vtable),
			}),
		};

		export(
			Module::Engine,
			NetworkStringTables::VERSION,
			mocks.container,
		);
		export(Module::Engine, ValveEngine::VERSION, mocks.engine);
		mocks
	}
}

/// `INetworkStringTable::AddString`, which records the call and returns
/// [`ADD_RESULT`].
unsafe extern "C" fn add_string(
	this: *mut sys::INetworkStringTable,
	is_server: bool,
	string: *const c_char,
	length: c_int,
	user_data: *const c_void,
) -> c_int {
	ADDITIONS.with_borrow_mut(|additions| {
		additions.push(Addition {
			this,
			is_server,
			string,
			length,
			user_data,
			locked: tables_locked(),
		});
	});

	ADD_RESULT.get()
}

/// Every call to the mock table's `AddString` so far, in order.
fn additions() -> Vec<Addition> {
	ADDITIONS.with_borrow(Clone::clone)
}

#[test]
fn additions_are_server_additions_without_user_data() {
	let mocks = Mocks::exported();
	let scope = ();
	let string = c"materials/example/overlay.vmt";

	FOUND_TABLE.set(mocks.table);
	ADD_RESULT.set(3);

	let table = mock_server(&scope)
		.network_string_tables()
		.unwrap()
		.find(DOWNLOADABLES)
		.unwrap();

	assert_eq!(table.add(string), Ok(3));
	assert_eq!(
		additions(),
		[Addition {
			this: mocks.table,
			is_server: true,
			string: string.as_ptr(),
			length: -1,
			user_data: null(),
			locked: true,
		}]
	);
}

#[test]
fn downloadables_are_added_while_the_tables_are_unlocked() {
	let mocks = Mocks::exported();
	let scope = ();
	let path = c"materials/example/overlay.vtf";

	FOUND_TABLE.set(mocks.table);
	ADD_RESULT.set(5);

	assert_eq!(add_downloadable(mock_server(&scope), path), Ok(5));
	assert_eq!(
		find_requests(),
		[(mocks.container.cast_const(), DOWNLOADABLES.to_owned())]
	);
	assert_eq!(
		lock_requests(),
		[(mocks.engine, false), (mocks.engine, true)]
	);
	assert!(tables_locked());

	let additions = additions();

	assert_eq!(additions.len(), 1);
	assert_eq!(additions[0].this, mocks.table);
	assert_eq!(additions[0].string, path.as_ptr());
	assert!(!additions[0].locked);
}

/// Every container and name passed to the mock container's `FindTable` so
/// far, in order.
fn find_requests() -> Vec<(*const sys::INetworkStringTableContainer, CString)> {
	FIND_REQUESTS.with_borrow(Clone::clone)
}

/// `INetworkStringTable::FindStringIndex`, which finds [`KNOWN_STRINGS`].
unsafe extern "C" fn find_string_index(
	_: *mut sys::INetworkStringTable,
	string: *const c_char,
) -> c_int {
	// SAFETY: The wrappers pass NUL-terminated strings.
	let string = unsafe { CStr::from_ptr(string) };

	KNOWN_STRINGS.with_borrow(|known| {
		known
			.iter()
			.position(|known| known.as_c_str() == string)
			.map_or(INVALID_STRING_INDEX.into(), |index| index as c_int)
	})
}

/// `INetworkStringTableContainer::FindTable`, which records the call and
/// returns [`FOUND_TABLE`].
unsafe extern "C" fn find_table(
	this: *const sys::INetworkStringTableContainer,
	name: *const c_char,
) -> *mut sys::INetworkStringTable {
	// SAFETY: The wrappers pass NUL-terminated names.
	let name = unsafe { CStr::from_ptr(name) }.to_owned();

	FIND_REQUESTS.with_borrow_mut(|requests| requests.push((this, name)));
	FOUND_TABLE.get()
}

/// `INetworkStringTable::GetMaxStrings`, which returns [`MAX_STRINGS`].
unsafe extern "C" fn max_strings(_: *const sys::INetworkStringTable) -> c_int {
	MAX_STRINGS.get()
}

#[test]
fn capacities_are_the_tables_own() {
	let mocks = Mocks::exported();
	let scope = ();

	FOUND_TABLE.set(mocks.table);
	MAX_STRINGS.set(4096);

	let table = mock_server(&scope)
		.network_string_tables()
		.unwrap()
		.find(MODEL_PRECACHE)
		.unwrap();

	assert_eq!(table.max_len(), 4096);
	assert_eq!(
		find_requests(),
		[(mocks.container.cast_const(), c"modelprecache".to_owned())]
	);
}

#[test]
fn missing_downloadables_tables_leave_the_lock_alone() {
	let scope = ();

	// Nothing is looked up without the engine's interfaces.
	assert!(matches!(
		add_downloadable(mock_server(&scope), c"sound/example.wav"),
		Err(AddDownloadableError::Interface(_))
	));

	let mocks = Mocks::exported();

	assert_eq!(
		add_downloadable(mock_server(&scope), c"sound/example.wav"),
		Err(AddDownloadableError::MissingTable)
	);
	assert_eq!(
		find_requests(),
		[(mocks.container.cast_const(), DOWNLOADABLES.to_owned())]
	);
	assert!(lock_requests().is_empty());
	assert!(additions().is_empty());
}

#[test]
fn precached_sounds_are_found_in_the_sound_precache_table() {
	let mocks = Mocks::exported();
	let scope = ();
	let tables = mock_server(&scope).network_string_tables().unwrap();

	// A level without the table has no precached sounds.
	assert!(!tables.is_sound_precached(c"vo/scout_thanks01.mp3"));

	FOUND_TABLE.set(mocks.table);
	KNOWN_STRINGS.set(vec![c"vo/scout_thanks01.mp3".to_owned()]);

	assert!(tables.is_sound_precached(c"vo/scout_thanks01.mp3"));
	assert!(!tables.is_sound_precached(c"vo/does_not_exist.wav"));
	assert!(find_requests().iter().all(|(container, name)| {
		*container == mocks.container.cast_const() && name.as_c_str() == SOUND_PRECACHE
	}));
}

#[test]
fn refused_strings_are_errors() {
	let mocks = Mocks::exported();
	let scope = ();

	FOUND_TABLE.set(mocks.table);

	let table = mock_server(&scope)
		.network_string_tables()
		.unwrap()
		.find(DOWNLOADABLES)
		.unwrap();

	for refusal in [INVALID_STRING_INDEX.into(), -1] {
		ADD_RESULT.set(refusal);

		assert_eq!(table.add(c"example"), Err(AddStringError));
	}

	ADD_RESULT.set(0);

	assert_eq!(table.add(c"example"), Ok(0));
}
