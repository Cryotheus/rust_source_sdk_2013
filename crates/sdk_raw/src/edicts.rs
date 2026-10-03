//! Values from `public/edict.h` and `public/const.h` describing the engine's
//! edict table, which the generated bindings omit.

use std::ffi::c_int;
use std::mem::MaybeUninit;

const _: () = {
	let info = MaybeUninit::<sys::CEdictChangeInfo>::uninit();
	let shared = MaybeUninit::<sys::CSharedEdictChangeInfo>::uninit();

	// SAFETY: The places are only projected to, never read.
	let (offsets, infos) = unsafe {
		(
			&raw const (*info.as_ptr()).m_ChangeOffsets,
			&raw const (*shared.as_ptr()).m_ChangeInfos,
		)
	};

	assert!(array_len(offsets) == MAX_CHANGE_OFFSETS as usize);
	assert!(array_len(infos) == MAX_EDICT_CHANGE_INFOS as usize);
};

/// `FL_EDICT_CHANGED` from `public/edict.h`, set when a networked variable changes.
pub const FL_EDICT_CHANGED: c_int = 1 << 0;

/// `FL_EDICT_FREE` from `public/edict.h`, set while a slot holds no entity.
pub const FL_EDICT_FREE: c_int = 1 << 1;

/// `FL_FULL_EDICT_CHANGED` from `public/edict.h`, set when every networked
/// variable must be compared rather than only the recorded offsets.
pub const FL_FULL_EDICT_CHANGED: c_int = 1 << 8;

/// How many changed offsets one `CEdictChangeInfo` records, past which the
/// whole entity is compared.
///
/// This is `MAX_CHANGE_OFFSETS` from `public/edict.h`.
pub const MAX_CHANGE_OFFSETS: u16 = 19;

/// How many `CEdictChangeInfo`s the engine's `CSharedEdictChangeInfo` holds
/// per frame.
///
/// This is `MAX_EDICT_CHANGE_INFOS` from `public/edict.h`.
pub const MAX_EDICT_CHANGE_INFOS: u16 = 100;

/// Number of slots in the edict table.
///
/// This is `MAX_EDICTS` from `public/const.h`.
pub const MAX_EDICTS: c_int = 1 << 11;

/// The length of the array `_array` points to.
const fn array_len<T, const N: usize>(_array: *const [T; N]) -> usize {
	N
}
