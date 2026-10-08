//! Metamod's raw `ISmmPluginManager`, from `IPluginManager.h`, which both
//! supported builds declare alike.

use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::{align_of, offset_of, size_of};

const _: () = {
	const SLOT: usize = size_of::<*const ()>();

	macro_rules! assert_slot {
		($field:ident, $slot:expr) => {
			assert!(offset_of!(ISmmPluginManagerVtable, $field) == $slot * SLOT);
		};
	}

	assert!(size_of::<ISmmPluginManager>() == SLOT);
	assert!(align_of::<ISmmPluginManager>() == align_of::<*const ()>());

	assert_slot!(load, 0);
	assert_slot!(unload, 1);
	assert_slot!(pause, 2);
	assert_slot!(unpause, 3);
	assert_slot!(unload_all, 4);
	assert_slot!(query, 5);
	assert_slot!(query_running, 6);
	assert_slot!(query_handle, 7);
	assert!(size_of::<ISmmPluginManagerVtable>() == 8 * SLOT);
};

/// The name `ISmmAPI::MetaFactory` gives the plugin manager by:
/// `MMIFACE_PLMANAGER`.
pub const MMIFACE_PLMANAGER: &CStr = c"IPluginManager";

/// `PluginId`: a plugin's number, which Metamod gives each load in turn, from
/// [`PL_MIN_ID`], and never gives again.
pub type PluginId = c_int;

/// `Pl_BadLoad`: the ID of a load that failed.
pub const PL_BAD_LOAD: PluginId = 0;

/// `Pl_Console`: the source of a plugin loaded from the console.
pub const PL_CONSOLE: PluginId = -1;

/// `Pl_File`: the source of a plugin loaded from Metamod's plugin files.
pub const PL_FILE: PluginId = -2;

/// `Pl_MinId`: the first plugin's ID.
pub const PL_MIN_ID: PluginId = 1;

/// `Pl_Status`: whether a plugin runs.
pub type PlStatus = c_int;

/// `Pl_NotFound`: its file was not found.
pub const PL_NOT_FOUND: PlStatus = -4;

/// `Pl_Error`: its file could not be loaded as a plugin.
pub const PL_ERROR: PlStatus = -3;

/// `Pl_Refused`: its `Load` refused.
pub const PL_REFUSED: PlStatus = -2;

/// `Pl_Paused`: paused, so its hooks are skipped.
pub const PL_PAUSED: PlStatus = -1;

/// `Pl_Running`: loaded and running.
pub const PL_RUNNING: PlStatus = 0;

/// A C++ `ISmmPluginManager`, owned by Metamod.
///
/// The interface has no virtual destructor, so its first vtable entry is
/// `Load`.
#[repr(C)]
pub struct ISmmPluginManager {
	pub vtable: *const ISmmPluginManagerVtable,
}

#[repr(C)]
pub struct ISmmPluginManagerVtable {
	/// `Load(file, source, already, error, maxlen)`; `already` is a `bool &`.
	pub load: unsafe extern "C" fn(
		*mut ISmmPluginManager,
		*const c_char,
		PluginId,
		*mut bool,
		*mut c_char,
		usize,
	) -> PluginId,
	pub unload:
		unsafe extern "C" fn(*mut ISmmPluginManager, PluginId, bool, *mut c_char, usize) -> bool,
	pub pause: unsafe extern "C" fn(*mut ISmmPluginManager, PluginId, *mut c_char, usize) -> bool,
	pub unpause: unsafe extern "C" fn(*mut ISmmPluginManager, PluginId, *mut c_char, usize) -> bool,
	pub unload_all: unsafe extern "C" fn(*mut ISmmPluginManager) -> bool,

	/// `Query(id, file, status, source)`, each out-parameter optional.
	pub query: unsafe extern "C" fn(
		*mut ISmmPluginManager,
		PluginId,
		*mut *const c_char,
		*mut PlStatus,
		*mut PluginId,
	) -> bool,
	pub query_running:
		unsafe extern "C" fn(*mut ISmmPluginManager, PluginId, *mut c_char, usize) -> bool,
	pub query_handle:
		unsafe extern "C" fn(*mut ISmmPluginManager, PluginId, *mut *mut c_void) -> bool,
}
