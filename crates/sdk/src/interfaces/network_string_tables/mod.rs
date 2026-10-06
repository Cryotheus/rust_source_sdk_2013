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

/// The name of the table listing the precached models, whose indices are
/// entities' model indices.
///
/// The engine precaches a model by adding it to this table. Once the table
/// holds [`NetworkStringTable::max_len`] strings, TF2's engine reports a new
/// model as an overflow ("too many models") through `Host_Error`, which is
/// expected, though not verified in the current engine, to exit a dedicated
/// server: check the room left before precaching models while a level runs.
pub const MODEL_PRECACHE: &CStr = c"modelprecache";

/// The name of the table listing the precached particle systems, whose
/// indices clients receive particle effects by.
///
/// The game's `PrecacheParticleSystem` adds to it; TF2 precaches the systems
/// its players and weapons use with every level.
pub const PARTICLE_EFFECT_NAMES: &CStr = c"ParticleEffectNames";

/// The name of the table listing the precached sounds, which clients load
/// before playing them.
pub const SOUND_PRECACHE: &CStr = c"soundprecache";

interface! {
	/// The string tables the server replicates to clients (`INetworkStringTableContainer`).
	#[doc(alias("INetworkStringTableContainer"))]
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
#[doc(alias("INetworkStringTable"))]
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
	#[doc(alias("AddString"))]
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
	#[doc(alias("FindStringIndex"))]
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
	#[doc(alias("GetNumStrings"))]
	pub fn len(self) -> usize {
		// SAFETY: As for `name`.
		usize::try_from(unsafe { vcall!(self.as_ptr() => INetworkStringTable_GetNumStrings()) })
			.unwrap_or(0)
	}

	/// The most strings the table can hold, as it was created with, which
	/// bounds [`Self::len`]. The engine refuses new strings beyond it.
	#[doc(alias("GetMaxStrings"))]
	pub fn max_len(self) -> usize {
		// SAFETY: As for `name`.
		usize::try_from(unsafe { vcall!(self.as_ptr() => INetworkStringTable_GetMaxStrings()) })
			.unwrap_or(0)
	}

	/// The table's name, such as `modelprecache`, or an empty string if the
	/// engine reports none.
	#[doc(alias("GetTableName"))]
	pub fn name(self) -> &'s CStr {
		// SAFETY: Tables live until the level ends, which it does not during
		// `'s`, and never rename themselves.
		unsafe { borrow_cstr(vcall!(self.as_ptr() => INetworkStringTable_GetTableName())) }
			.unwrap_or_default()
	}

	/// The string at an index, which ranges up to [`Self::len`], or `None` if
	/// the index is out of range or the engine returns no string.
	#[doc(alias("GetString"))]
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
	#[doc(alias("FindTable"))]
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
	#[doc(alias("GetTable"))]
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
	#[doc(alias("IsSoundPrecached"))]
	pub fn is_sound_precached(self, sample: &CStr) -> bool {
		self.find(SOUND_PRECACHE)
			.is_some_and(|table| table.find(sample).is_some())
	}

	/// The number of tables, which bounds the IDs [`Self::get`] takes.
	#[doc(alias("GetNumTables"))]
	pub fn len(self) -> usize {
		usize::try_from(self.count()).unwrap_or(0)
	}

	/// The index of a precached particle system, such as TF2's
	/// `drg_fiery_death`, in the [`PARTICLE_EFFECT_NAMES`] table, which
	/// [`TempEntities::dispatch_particle_effect`] plays it by, or `None` if the
	/// level did not precache it.
	///
	/// Unlike the game's `GetParticleSystemIndex`, a missing system is not
	/// reported as index 0.
	///
	/// [`TempEntities::dispatch_particle_effect`]: crate::interfaces::TempEntities::dispatch_particle_effect
	#[doc(alias("GetParticleSystemIndex"))]
	pub fn particle_system_index(self, name: &CStr) -> Option<usize> {
		self.find(PARTICLE_EFFECT_NAMES)?.find(name)
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
