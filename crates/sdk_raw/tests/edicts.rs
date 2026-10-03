//! Tests of marking edicts changed through the engine's change tracking.

use source_sdk_2013_raw::edicts::{
	FL_EDICT_CHANGED, FL_FULL_EDICT_CHANGED, MAX_CHANGE_OFFSETS, state_changed,
};
use source_sdk_2013_raw::test_support::edicts::mock_edict;
use std::ptr::NonNull;

#[test]
fn missing_change_tracking_marks_the_edict_fully_changed() {
	let mut slot = mock_edict(4, false);
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
	// SAFETY: The edict is a local, and no change info is given.
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

	let mut slot = mock_edict(4, false);
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
