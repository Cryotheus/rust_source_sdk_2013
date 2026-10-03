//! `INetworkStringTableContainer`, the string tables the server replicates to clients.
//!
//! # Adding strings
//!
//! The engine recreates its tables for every level, so strings a plugin adds
//! last only until the level ends, and the plugin must add them again for the
//! next level, such as when it initializes. While a level runs, the game
//! unlocks the tables around its own additions and then restores their
//! previous state: wrap additions in
//! [`ValveEngine::with_unlocked_string_tables`], which does the same.
//! [`add_downloadable`] does so for the [`DOWNLOADABLES`] table.
//!
//! [`ValveEngine::with_unlocked_string_tables`]: crate::interfaces::ValveEngine::with_unlocked_string_tables

use crate::NotThreadSafe;
use crate::server::{InterfaceError, Server};
use sdk_raw::interfaces::network_string_tables::{INVALID_STRING_INDEX, UNKNOWN_STRING_LENGTH};
use sdk_raw::util::cstr::{borrow_cstr, copy_cstr};
use sdk_raw::vcall;
use std::ffi::{CStr, CString, c_int};
use std::marker::PhantomData;
use std::ptr::{self, NonNull};

/// The name of the table listing the files clients download while they
/// connect.
pub const DOWNLOADABLES: &CStr = c"downloadables";

/// The name of the table listing the precached sounds, which clients load
/// before playing them.
pub const SOUND_PRECACHE: &CStr = c"soundprecache";

interface! {
	/// The string tables the server replicates to clients (`INetworkStringTableContainer`).
	#[doc(alias = "INetworkStringTableContainer")]
	pub struct NetworkStringTables(sys::INetworkStringTableContainer) = Engine sdk_raw::interfaces::network_string_tables::VERSION;
}

/// A file could not be added to the [`DOWNLOADABLES`] table, as
/// [`add_downloadable`] reports.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AddDownloadableError {
	/// The engine does not export an interface this needs.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The engine has no table by that name, as before a level loads.
	#[error("the engine has no `downloadables` string table")]
	MissingTable,

	/// The table refused the file, as it does once it is full.
	#[error("the `downloadables` string table refused the file")]
	Refused,
}

/// A string table refused a string, as [`NetworkStringTable::add`] reports.
///
/// The engine refuses new strings, such as once a table holds as many as it
/// was created for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the string table refused the string")]
pub struct AddStringError;

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

	/// Adds a string to the table and returns its index, which is the index it
	/// already had if the table contains it.
	///
	/// The engine copies the string and replicates it to clients. A new string
	/// gets no user data, and a string the table already contains keeps its
	/// own. Fails if the engine refuses the string, as it does once the table
	/// is full.
	///
	/// The string lasts only until the level ends, so add it again for each
	/// level. While a level runs, add strings inside
	/// [`ValveEngine::with_unlocked_string_tables`], as the game unlocks the
	/// tables around its own additions.
	///
	/// Tables the engine fills itself, such as `modelprecache`,
	/// `soundprecache`, `userinfo`, and `instancebaseline`, keep records beside
	/// their strings that this bypasses, so prefer the engine's own ways of
	/// adding to those. Clients act on some tables' strings as they arrive, so
	/// add only strings a table's readers expect.
	///
	/// [`ValveEngine::with_unlocked_string_tables`]: crate::interfaces::ValveEngine::with_unlocked_string_tables
	#[doc(alias = "AddString")]
	#[expect(
		clippy::should_implement_trait,
		reason = "adds to the table, unlike `Add::add`"
	)]
	pub fn add(self, string: &CStr) -> Result<usize, AddStringError> {
		// SAFETY: As for `name`. The engine copies the string, which only needs
		// to live for the call. As in the game's own additions, such as
		// `PrecacheMaterial`, the length is the header's default and no user
		// data is passed, so there is no buffer for the engine to read.
		let index = unsafe {
			vcall!(self.as_ptr() => INetworkStringTable_AddString(true, string.as_ptr(), UNKNOWN_STRING_LENGTH, ptr::null()))
		};

		usize::try_from(index)
			.ok()
			.filter(|_| index != c_int::from(INVALID_STRING_INDEX))
			.ok_or(AddStringError)
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
			.filter(|_| index != c_int::from(INVALID_STRING_INDEX))
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

	/// Finds a table by name, such as `modelprecache` or [`DOWNLOADABLES`], or
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

	/// Whether a sample, such as `vo/scout_thanks01.mp3`, is in the
	/// [`SOUND_PRECACHE`] table, which the engine plays sounds from.
	///
	/// The sample must be spelled as it was precached, including any of the
	/// engine's leading sound characters. Unlike the engine's own
	/// [`EngineSound::is_sound_precached`], which TF2's 64-bit Windows engine
	/// was observed to answer `true` for a sample it had not precached, this reads the table the engine looks samples up
	/// in. `false` if the level has no such table.
	///
	/// [`EngineSound::is_sound_precached`]: crate::interfaces::EngineSound::is_sound_precached
	#[doc(alias = "IsSoundPrecached")]
	pub fn is_sound_precached(self, sample: &CStr) -> bool {
		self.find(SOUND_PRECACHE)
			.is_some_and(|table| table.find(sample).is_some())
	}

	/// The number of tables, which bounds the IDs [`Self::get`] takes.
	#[doc(alias = "GetNumTables")]
	pub fn len(self) -> usize {
		usize::try_from(self.count()).unwrap_or(0)
	}
}

/// Adds a file, such as `materials/example/overlay.vmt`, to the files clients
/// download while they connect, and returns its index in the
/// [`DOWNLOADABLES`] table.
///
/// The path is relative to the game's search paths, such as the game
/// directory. A client downloads a file only if it lacks it, the server
/// offers it, which depends on settings such as `sv_allowdownload` and
/// `sv_downloadurl`, and the client's own download settings, such as
/// `cl_allowdownload`, allow it. Each file is listed on its own, so a
/// material's textures need adding too.
///
/// Clients only download files while they connect, so add files for each
/// level before clients connect to it, such as when it initializes; a file
/// added later only reaches clients that connect afterwards. The tables are
/// unlocked for the addition, then returned to their previous state.
///
/// Fails if the engine does not export an interface this needs, has no
/// `downloadables` table, or the table refuses the file.
pub fn add_downloadable(server: Server<'_>, path: &CStr) -> Result<usize, AddDownloadableError> {
	let engine = server.valve_engine()?;

	let table = server
		.network_string_tables()?
		.find(DOWNLOADABLES)
		.ok_or(AddDownloadableError::MissingTable)?;

	engine
		.with_unlocked_string_tables(|| table.add(path))
		.map_err(|AddStringError| AddDownloadableError::Refused)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::interfaces::ValveEngine;
	use crate::server::Module;
	use crate::server::test_support::{export, mock_server};
	use sdk_raw::util::mock::{mock_vtable, unexpected_call};
	use std::cell::{Cell, RefCell};
	use std::ffi::{c_char, c_void};
	use std::ptr::null_mut;

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

	/// A mock engine, string table container, and table, kept alive together.
	struct Mocks {
		_container_vtable: Box<sys::INetworkStringTableContainer__bindgen_vtable>,
		_engine_vtable: Box<sys::IVEngineServer__bindgen_vtable>,
		_table_vtable: Box<sys::INetworkStringTable__bindgen_vtable>,
		container: Box<sys::INetworkStringTableContainer>,
		engine: Box<sys::IVEngineServer>,
		table: Box<sys::INetworkStringTable>,
	}

	impl Mocks {
		/// Builds the mocks and exports the engine and container from the
		/// engine factory of [`mock_server`].
		fn exported() -> Self {
			let container_vtable = unsafe {
				mock_vtable::<sys::INetworkStringTableContainer__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).INetworkStringTableContainer_FindTable)
							.write(find_table);
					},
				)
			};

			let engine_vtable = unsafe {
				mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
					unexpected_call as *const (),
					|vtable| {
						(&raw mut (*vtable).IVEngineServer_LockNetworkStringTables).write(lock);
					},
				)
			};

			let (table_vtable, table) = mock_table();

			let mut mocks = Self {
				container: Box::new(sys::INetworkStringTableContainer {
					vtable_: &raw const *container_vtable,
				}),
				engine: Box::new(sys::IVEngineServer {
					vtable_: &raw const *engine_vtable,
				}),
				table,
				_container_vtable: container_vtable,
				_engine_vtable: engine_vtable,
				_table_vtable: table_vtable,
			};

			export(
				Module::Engine,
				NetworkStringTables::VERSION,
				&raw mut *mocks.container,
			);
			export(Module::Engine, ValveEngine::VERSION, &raw mut *mocks.engine);

			mocks
		}
	}

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

		/// Every engine and state passed to the mock engine's
		/// `LockNetworkStringTables`, in order.
		static LOCK_REQUESTS: RefCell<Vec<(*mut sys::IVEngineServer, bool)>> =
			const { RefCell::new(Vec::new()) };

		/// Whether the mock engine's string tables are locked.
		static LOCKED: Cell<bool> = const { Cell::new(true) };
	}

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
				locked: LOCKED.get(),
			});
		});

		ADD_RESULT.get()
	}

	fn additions() -> Vec<Addition> {
		ADDITIONS.with_borrow(Clone::clone)
	}

	#[test]
	fn additions_are_server_additions_without_user_data() {
		let (_vtable, mut raw) = mock_table();
		let raw_pointer = &raw mut *raw;
		let table = unsafe { NetworkStringTable::from_raw(NonNull::new(raw_pointer).unwrap()) };
		let string = c"materials/example/overlay.vmt";
		ADD_RESULT.set(3);

		assert_eq!(table.add(string), Ok(3));
		assert_eq!(
			additions(),
			[Addition {
				this: raw_pointer,
				is_server: true,
				string: string.as_ptr(),
				length: -1,
				user_data: ptr::null(),
				locked: true,
			}]
		);
	}

	#[test]
	fn downloadables_are_added_while_the_tables_are_unlocked() {
		let mut mocks = Mocks::exported();
		let scope = ();
		let path = c"materials/example/overlay.vtf";
		FOUND_TABLE.set(&raw mut *mocks.table);
		ADD_RESULT.set(5);

		assert_eq!(add_downloadable(mock_server(&scope), path), Ok(5));
		assert_eq!(
			find_requests(),
			[(&raw const *mocks.container, DOWNLOADABLES.to_owned())]
		);

		let engine = &raw mut *mocks.engine;

		assert_eq!(lock_requests(), [(engine, false), (engine, true)]);
		assert!(LOCKED.get());

		let additions = additions();

		assert_eq!(additions.len(), 1);
		assert_eq!(additions[0].this, &raw mut *mocks.table);
		assert_eq!(additions[0].string, path.as_ptr());
		assert!(!additions[0].locked);
	}

	fn find_requests() -> Vec<(*const sys::INetworkStringTableContainer, CString)> {
		FIND_REQUESTS.with_borrow(Clone::clone)
	}

	unsafe extern "C" fn find_string_index(
		_: *mut sys::INetworkStringTable,
		string: *const c_char,
	) -> c_int {
		let string = unsafe { CStr::from_ptr(string) };

		KNOWN_STRINGS.with_borrow(|known| {
			known
				.iter()
				.position(|known| known.as_c_str() == string)
				.map_or(INVALID_STRING_INDEX.into(), |index| index as c_int)
		})
	}

	unsafe extern "C" fn find_table(
		this: *const sys::INetworkStringTableContainer,
		name: *const c_char,
	) -> *mut sys::INetworkStringTable {
		let name = unsafe { CStr::from_ptr(name) }.to_owned();

		FIND_REQUESTS.with_borrow_mut(|requests| requests.push((this, name)));
		FOUND_TABLE.get()
	}

	#[test]
	fn full_downloadables_tables_still_restore_the_lock() {
		let mut mocks = Mocks::exported();
		let scope = ();
		FOUND_TABLE.set(&raw mut *mocks.table);
		ADD_RESULT.set(INVALID_STRING_INDEX.into());

		assert_eq!(
			add_downloadable(mock_server(&scope), c"sound/example.wav"),
			Err(AddDownloadableError::Refused)
		);

		let engine = &raw mut *mocks.engine;

		assert_eq!(lock_requests(), [(engine, false), (engine, true)]);
		assert!(LOCKED.get());
	}

	unsafe extern "C" fn lock(this: *mut sys::IVEngineServer, lock: bool) -> bool {
		LOCK_REQUESTS.with_borrow_mut(|requests| requests.push((this, lock)));
		LOCKED.replace(lock)
	}

	fn lock_requests() -> Vec<(*mut sys::IVEngineServer, bool)> {
		LOCK_REQUESTS.with_borrow(Clone::clone)
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
			[(&raw const *mocks.container, DOWNLOADABLES.to_owned())]
		);
		assert!(lock_requests().is_empty());
		assert!(additions().is_empty());
	}

	fn mock_table() -> (
		Box<sys::INetworkStringTable__bindgen_vtable>,
		Box<sys::INetworkStringTable>,
	) {
		let vtable = unsafe {
			mock_vtable::<sys::INetworkStringTable__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).INetworkStringTable_AddString).write(add_string);
					(&raw mut (*vtable).INetworkStringTable_FindStringIndex)
						.write(find_string_index);
				},
			)
		};

		let table = Box::new(sys::INetworkStringTable {
			vtable_: &raw const *vtable,
		});

		(vtable, table)
	}

	#[test]
	fn precached_sounds_are_found_in_the_sound_precache_table() {
		let mut mocks = Mocks::exported();
		let scope = ();
		let tables = mock_server(&scope).network_string_tables().unwrap();

		// A level without the table has no precached sounds.
		assert!(!tables.is_sound_precached(c"vo/scout_thanks01.mp3"));

		FOUND_TABLE.set(&raw mut *mocks.table);
		KNOWN_STRINGS.set(vec![c"vo/scout_thanks01.mp3".to_owned()]);

		assert!(tables.is_sound_precached(c"vo/scout_thanks01.mp3"));
		assert!(!tables.is_sound_precached(c"vo/does_not_exist.wav"));
		assert!(find_requests().iter().all(|(container, name)| *container
			== &raw const *mocks.container
			&& name.as_c_str() == SOUND_PRECACHE));
	}

	#[test]
	fn refused_strings_are_errors() {
		let (_vtable, mut raw) = mock_table();
		let table = unsafe { NetworkStringTable::from_raw(NonNull::from(&mut *raw)) };

		for refusal in [INVALID_STRING_INDEX.into(), -1] {
			ADD_RESULT.set(refusal);

			assert_eq!(table.add(c"example"), Err(AddStringError));
		}

		ADD_RESULT.set(0);

		assert_eq!(table.add(c"example"), Ok(0));
	}
}
