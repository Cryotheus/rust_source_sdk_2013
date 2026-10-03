//! Values from `public/edict.h` and `public/const.h` describing the engine's
//! edict table, which the generated bindings omit, and a port of the inline
//! `CBaseEdict::StateChanged` methods, which record changed networked
//! variables in the engine's change tracking.

use std::ffi::c_int;
use std::mem::MaybeUninit;
use std::ptr::NonNull;

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

const _: () = assert!(MAX_EDICTS == 1 << MAX_EDICT_BITS);

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

/// Number of bits an edict index needs.
///
/// This is `MAX_EDICT_BITS` from `public/const.h`.
pub const MAX_EDICT_BITS: u32 = 11;

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

/// Records that the edict's entity changed as a whole, so the engine compares
/// all of its networked variables, as `CBaseEdict::StateChanged()` does.
///
/// The edict is flagged [changed](FL_EDICT_CHANGED) and
/// [fully changed](FL_FULL_EDICT_CHANGED). Given the edict's change accessor,
/// the change info it claimed this frame is released, so that it claims a new
/// one once it is no longer fully changed.
///
/// # Safety
///
/// `edict` must point to a slot of the engine's edict table, and `accessor`,
/// if any, to the engine's change accessor for that slot
/// (`IVEngineServer::GetChangeAccessor`). The call must be made on the
/// server's main thread, the only one that accesses them.
#[doc(alias = "StateChanged")]
pub unsafe fn full_state_changed(
	edict: *mut sys::edict_t,
	accessor: Option<NonNull<sys::IChangeInfoAccessor>>,
) {
	// SAFETY: The edict and accessor are the engine's, accessed by no other
	// thread, and their fields are read and written without forming
	// references, as the engine's own pointers may alias them.
	unsafe {
		let flags = &raw mut (*edict)._base.m_fStateFlags;

		flags.write(flags.read() | FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED);

		if let Some(accessor) = accessor {
			(&raw mut (*accessor.as_ptr()).m_iChangeInfoSerialNumber).write(0);
		}
	}
}

/// Records that the networked variable `offset` bytes into the edict's entity
/// changed, so the engine sends it to clients, as
/// `CBaseEdict::StateChanged(unsigned short)` does.
///
/// Nothing is recorded for an edict already [fully
/// changed](FL_FULL_EDICT_CHANGED). Otherwise the edict is flagged
/// [changed](FL_EDICT_CHANGED), then `change_info` is called for the edict's
/// change accessor and the engine's shared change info, which hold the
/// offsets changed this frame. The offset is added to the change info the
/// edict claimed this frame, claiming one first if it has none. The edict
/// becomes [fully changed](full_state_changed) instead if `change_info` gives
/// `None`, or if [`MAX_CHANGE_OFFSETS`] offsets or
/// [`MAX_EDICT_CHANGE_INFOS`] change infos are already recorded.
///
/// # Safety
///
/// `edict` must point to a slot of the engine's edict table. If
/// `change_info` gives `Some`, it must be the engine's change accessor for
/// that slot (`IVEngineServer::GetChangeAccessor`) and its shared change info
/// (`IVEngineServer::GetSharedEdictChangeInfo`). The call must be made on the
/// server's main thread, the only one that accesses them.
#[doc(alias = "StateChanged")]
pub unsafe fn state_changed(
	edict: *mut sys::edict_t,
	offset: u16,
	change_info: impl FnOnce() -> Option<(
		NonNull<sys::IChangeInfoAccessor>,
		NonNull<sys::CSharedEdictChangeInfo>,
	)>,
) {
	// SAFETY: As for `full_state_changed`.
	let flags = unsafe { &raw mut (*edict)._base.m_fStateFlags };

	// SAFETY: As above.
	let state = unsafe { flags.read() };

	if state & FL_FULL_EDICT_CHANGED != 0 {
		return;
	}

	// SAFETY: As above.
	unsafe { flags.write(state | FL_EDICT_CHANGED) };

	let Some((accessor, shared)) = change_info() else {
		// SAFETY: The caller upholds the contract.
		return unsafe { full_state_changed(edict, None) };
	};

	let accessor = accessor.as_ptr();
	let shared = shared.as_ptr();

	// SAFETY: Both structures are the engine's, accessed by no other thread.
	// Indices are checked against the fixed array lengths before use, and no
	// references are formed.
	unsafe {
		let serial_number = (&raw const (*shared).m_iSerialNumber).read();
		let infos = (&raw mut (*shared).m_ChangeInfos).cast::<sys::CEdictChangeInfo>();
		let fully_changed = || full_state_changed(edict, NonNull::new(accessor));

		if (&raw const (*accessor).m_iChangeInfoSerialNumber).read() == serial_number {
			// The entity already has change info this frame, so add the offset.
			let index = (&raw const (*accessor).m_iChangeInfo).read();

			if index >= MAX_EDICT_CHANGE_INFOS {
				return fully_changed();
			}

			let info = infos.add(usize::from(index));
			let count = (&raw const (*info).m_nChangeOffsets).read();
			let offsets = (&raw mut (*info).m_ChangeOffsets).cast::<u16>();

			for slot in 0..count.min(MAX_CHANGE_OFFSETS) {
				if offsets.add(usize::from(slot)).read() == offset {
					return;
				}
			}

			if count >= MAX_CHANGE_OFFSETS {
				fully_changed();
			} else {
				offsets.add(usize::from(count)).write(offset);
				(&raw mut (*info).m_nChangeOffsets).write(count + 1);
			}
		} else {
			// Claim a new change info for this frame.
			let count = (&raw const (*shared).m_nChangeInfos).read();

			if count >= MAX_EDICT_CHANGE_INFOS {
				return fully_changed();
			}

			(&raw mut (*accessor).m_iChangeInfo).write(count);
			(&raw mut (*shared).m_nChangeInfos).write(count + 1);
			(&raw mut (*accessor).m_iChangeInfoSerialNumber).write(serial_number);

			let info = infos.add(usize::from(count));

			(&raw mut (*info).m_ChangeOffsets)
				.cast::<u16>()
				.write(offset);
			(&raw mut (*info).m_nChangeOffsets).write(1);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::ptr::null_mut;

	/// An edict as the engine lays out slot `index` of its table.
	fn edict(index: c_int) -> sys::edict_t {
		sys::edict_t {
			_base: sys::CBaseEdict {
				m_fStateFlags: 0,
				m_NetworkSerialNumber: 0,
				m_EdictIndex: index.try_into().unwrap(),
				m_pNetworkable: null_mut(),
				m_pUnk: null_mut(),
			},
			freetime: 0.0,
		}
	}

	#[test]
	fn missing_change_tracking_marks_the_edict_fully_changed() {
		let mut slot = edict(4);
		let mut asked = 0;

		// SAFETY: The edict is a local, and no change info is given.
		unsafe {
			state_changed(&raw mut slot, 40, || {
				asked += 1;
				None
			})
		};

		assert_eq!(
			slot._base.m_fStateFlags,
			FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED
		);

		// Fully changed edicts do not ask for change info again.
		// SAFETY: As above.
		unsafe {
			state_changed(&raw mut slot, 44, || {
				asked += 1;
				None
			})
		};
		assert_eq!(asked, 1);
	}

	#[test]
	fn state_changes_follow_the_engines_change_tracking() {
		let mut accessor = sys::IChangeInfoAccessor {
			m_iChangeInfo: 0,
			m_iChangeInfoSerialNumber: 0,
		};
		// SAFETY: Zero is valid for every field.
		let mut shared = Box::new(unsafe { std::mem::zeroed::<sys::CSharedEdictChangeInfo>() });

		shared.m_iSerialNumber = 7;
		shared.m_nChangeInfos = 3;

		let mut slot = edict(4);
		let edict = &raw mut slot;
		let tracking = (
			NonNull::new(&raw mut accessor).unwrap(),
			NonNull::new(&raw mut *shared).unwrap(),
		);
		// SAFETY: The edict and change tracking are locals, only accessed
		// through these pointers until the assertions read them.
		let change = |offset| unsafe { state_changed(edict, offset, || Some(tracking)) };

		// The first change this frame claims the next free change info.
		change(40);
		assert_eq!(slot._base.m_fStateFlags, FL_EDICT_CHANGED);
		assert_eq!(
			(accessor.m_iChangeInfo, accessor.m_iChangeInfoSerialNumber),
			(3, 7)
		);
		assert_eq!(shared.m_nChangeInfos, 4);
		assert_eq!(shared.m_ChangeInfos[3].m_nChangeOffsets, 1);
		assert_eq!(shared.m_ChangeInfos[3].m_ChangeOffsets[0], 40);

		// Later changes append new offsets once.
		change(44);
		change(40);
		assert_eq!(shared.m_ChangeInfos[3].m_nChangeOffsets, 2);
		assert_eq!(shared.m_ChangeInfos[3].m_ChangeOffsets[1], 44);

		// Overflowing the offsets falls back to a full comparison.
		for offset in 0..MAX_CHANGE_OFFSETS {
			change(100 + offset);
		}

		assert_eq!(
			slot._base.m_fStateFlags,
			FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED
		);
		assert_eq!(accessor.m_iChangeInfoSerialNumber, 0);

		// Once fully changed, offsets are no longer recorded.
		let recorded = shared.m_ChangeInfos[3];
		change(2);
		assert_eq!(
			shared.m_ChangeInfos[3].m_nChangeOffsets,
			recorded.m_nChangeOffsets
		);
	}
}
