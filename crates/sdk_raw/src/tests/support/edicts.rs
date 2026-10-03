//! Builders of the engine's edicts.

use crate::edicts::FL_EDICT_FREE;
use std::ffi::c_int;
use std::ptr::null_mut;

/// An edict as the engine lays out slot `index` of its table, marked free if
/// `free` is set.
///
/// For tests only. The edict has no networkable or entity.
///
/// # Panics
///
/// If `index` is negative.
pub fn mock_edict(index: c_int, free: bool) -> sys::edict_t {
	sys::edict_t {
		_base: sys::CBaseEdict {
			m_fStateFlags: if free { FL_EDICT_FREE } else { 0 },
			m_NetworkSerialNumber: 0,
			m_EdictIndex: index.try_into().unwrap(),
			m_pNetworkable: null_mut(),
			m_pUnk: null_mut(),
		},
		freetime: 0.0,
	}
}
