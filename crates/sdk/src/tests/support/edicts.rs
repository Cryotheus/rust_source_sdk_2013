//! The engine's edict table and its change tracking, as mock engines serve
//! them through `IVEngineServer`.

use sdk_raw::test_support::edicts::mock_edict;
use std::cell::{Cell, RefCell};
use std::ffi::c_int;
use std::ptr::null_mut;

thread_local! {
	/// What [`change_accessor`] returns.
	static ACCESSOR: Cell<*mut sys::IChangeInfoAccessor> = const { Cell::new(null_mut()) };

	/// The table [`edict_of_index`] serves, and its length.
	static EDICTS: Cell<(*mut sys::edict_t, usize)> = const { Cell::new((null_mut(), 0)) };

	/// The edicts [`notify_edict_flags_change`] was told of, in order.
	static FLAG_CHANGES: RefCell<Vec<c_int>> = const { RefCell::new(Vec::new()) };

	/// What [`shared_change_info`] returns.
	static SHARED: Cell<*mut sys::CSharedEdictChangeInfo> = const { Cell::new(null_mut()) };
}

/// `IVEngineServer::GetChangeAccessor`, which returns the accessor
/// [`set_change_accessor`] set on this thread, for every edict, or null, so
/// that every change is a full one.
///
/// # Safety
///
/// None: it reads neither argument. It is `unsafe` to fit the vtable slot.
pub unsafe extern "C" fn change_accessor(
	_: *mut sys::IVEngineServer,
	_: *const sys::edict_t,
) -> *mut sys::IChangeInfoAccessor {
	ACCESSOR.get()
}

/// `IVEngineServer::PEntityOfEntIndex`, which returns the slot of the table
/// [`serve_edicts`] set on this thread, or null past its end.
///
/// # Safety
///
/// The table set on this thread must still be alive.
pub unsafe extern "C" fn edict_of_index(
	_: *mut sys::IVEngineServer,
	index: c_int,
) -> *mut sys::edict_t {
	let (table, len) = EDICTS.get();

	match usize::try_from(index) {
		// SAFETY: The slot lies within the table, which the caller keeps alive.
		Ok(slot) if slot < len => unsafe { table.add(slot) },

		_ => null_mut(),
	}
}

/// An edict table of `len` slots as the engine lays it out, each slot free
/// where `free` says so.
///
/// For tests only.
///
/// # Panics
///
/// If `len` exceeds the number of edicts a `c_int` can index.
pub fn edict_table(len: usize, free: impl Fn(usize) -> bool) -> Box<[sys::edict_t]> {
	(0..len)
		.map(|slot| mock_edict(c_int::try_from(slot).unwrap(), free(slot)))
		.collect()
}

/// `IVEngineServer::NotifyEdictFlagsChange`, which notes the edict on this
/// thread, for [`take_flag_changes`].
///
/// # Safety
///
/// None: it reads no pointer. It is `unsafe` to fit the vtable slot.
pub unsafe extern "C" fn notify_edict_flags_change(_: *mut sys::IVEngineServer, edict: c_int) {
	FLAG_CHANGES.with_borrow_mut(|changes| changes.push(edict));
}

/// Makes [`edict_of_index`] serve the `len` slots at `table` on this thread,
/// which must stay alive while it does.
///
/// For tests only.
pub fn serve_edicts(table: *mut sys::edict_t, len: usize) {
	EDICTS.set((table, len));
}

/// Sets the accessor [`change_accessor`] returns on this thread, or null for
/// none.
///
/// For tests only.
pub fn set_change_accessor(accessor: *mut sys::IChangeInfoAccessor) {
	ACCESSOR.set(accessor);
}

/// Sets the shared change info [`shared_change_info`] returns on this
/// thread, or null for none.
///
/// For tests only.
pub fn set_shared_change_info(shared: *mut sys::CSharedEdictChangeInfo) {
	SHARED.set(shared);
}

/// `IVEngineServer::GetSharedEdictChangeInfo`, which returns the shared
/// change info [`set_shared_change_info`] set on this thread, or null, so
/// that every change is a full one.
///
/// # Safety
///
/// None: it reads no argument. It is `unsafe` to fit the vtable slot.
pub unsafe extern "C" fn shared_change_info(
	_: *mut sys::IVEngineServer,
) -> *mut sys::CSharedEdictChangeInfo {
	SHARED.get()
}

/// The edicts [`notify_edict_flags_change`] was told of on this thread since
/// the last call, in order.
///
/// For tests only.
pub fn take_flag_changes() -> Vec<c_int> {
	FLAG_CHANGES.take()
}
