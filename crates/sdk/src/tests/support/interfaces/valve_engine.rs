//! The lock of the engine's network string tables, as a mock
//! `IVEngineServer` keeps it.

use std::cell::{Cell, RefCell};

thread_local! {
	/// Whether the mock engine's string tables are locked.
	static LOCKED: Cell<bool> = const { Cell::new(true) };

	/// Every engine and state passed to [`lock_network_string_tables`], in
	/// order.
	static REQUESTS: RefCell<Vec<(*mut sys::IVEngineServer, bool)>> =
		const { RefCell::new(Vec::new()) };
}

/// `IVEngineServer::LockNetworkStringTables`, which records the call for
/// [`lock_requests`], and sets the lock, returning its previous state.
///
/// # Safety
///
/// None: it only records its arguments. It is `unsafe` to fit the vtable
/// slot.
pub unsafe extern "C" fn lock_network_string_tables(
	this: *mut sys::IVEngineServer,
	lock: bool,
) -> bool {
	REQUESTS.with_borrow_mut(|requests| requests.push((this, lock)));
	LOCKED.replace(lock)
}

/// Every engine and state passed to [`lock_network_string_tables`] on this
/// thread so far, in order.
///
/// For tests only.
pub fn lock_requests() -> Vec<(*mut sys::IVEngineServer, bool)> {
	REQUESTS.with_borrow(Clone::clone)
}

/// Sets whether the mock engine's string tables are locked on this thread,
/// which they are at first.
///
/// For tests only.
pub fn set_tables_locked(locked: bool) {
	LOCKED.set(locked);
}

/// Whether the mock engine's string tables are locked on this thread.
///
/// For tests only.
pub fn tables_locked() -> bool {
	LOCKED.get()
}
