//! The game's `CGlobalVars`, as a mock `IPlayerInfoManager` returns them.

use super::super::leak;
use std::cell::Cell;
use std::ffi::c_int;
use std::mem::MaybeUninit;
use std::ptr::null_mut;

thread_local! {
	/// What [`global_vars`] returns.
	static GLOBALS: Cell<*mut sys::CGlobalVars> = const { Cell::new(null_mut()) };
}

/// `IPlayerInfoManager::GetGlobalVars`, which returns the globals
/// [`serve_global_vars`] made on this thread, or null.
///
/// # Safety
///
/// None: it reads no argument. It is `unsafe` to fit the vtable slot.
pub unsafe extern "C" fn global_vars(_: *mut sys::IPlayerInfoManager) -> *mut sys::CGlobalVars {
	GLOBALS.get()
}

/// Makes [`global_vars`] return leaked globals on this thread, zeroed but for
/// the client limit `maxClients`, and returns them.
///
/// For tests only.
pub fn serve_global_vars(max_clients: c_int) -> *mut sys::CGlobalVars {
	let globals = leak(MaybeUninit::<sys::CGlobalVars>::zeroed()).cast::<sys::CGlobalVars>();

	// SAFETY: The globals are leaked and zeroed, which is valid for every
	// field.
	unsafe { (&raw mut (*globals)._base.maxClients).write(max_clients) };
	GLOBALS.set(globals);
	globals
}
