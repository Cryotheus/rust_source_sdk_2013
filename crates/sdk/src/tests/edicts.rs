//! Tests of edicts' reads of the engine's table, and of their state changes
//! reaching the engine's change tracking.

use super::*;
use crate::test_support::edicts::{edict_table, set_change_accessor, set_shared_change_info};
use crate::test_support::sdk_core::change_tracking_engine;
use sdk_raw::edicts::{FL_EDICT_CHANGED, FL_FULL_EDICT_CHANGED};
use sdk_raw::test_support::edicts::mock_edict;
use std::ptr::null_mut;

#[test]
fn reads_the_cached_index_and_free_flag() {
	let mut table = edict_table(3, |slot| slot == 1);
	let base = table.as_mut_ptr();
	// SAFETY: The slot lies within the table, which outlives every handle.
	let edict = |slot: usize| unsafe { Edict::from_raw(NonNull::new(base.add(slot)).unwrap()) };

	assert_eq!(edict(0).index(), 0);
	assert_eq!(edict(2).index(), 2);
	assert!(!edict(0).is_free());
	assert!(edict(1).is_free());
	assert_eq!(edict(1).entity(), None);
	assert_eq!(edict(0).class_name(), None);
}

/// The algorithm itself is tested in `sdk_raw::edicts`; this checks that the
/// engine's change tracking reaches it.
#[test]
fn state_changes_reach_the_engines_change_tracking() {
	let engine = change_tracking_engine();
	let mut accessor = sys::IChangeInfoAccessor {
		m_iChangeInfo: 0,
		m_iChangeInfoSerialNumber: 0,
	};
	// SAFETY: Zero is valid for every field of `CSharedEdictChangeInfo`.
	let mut shared = Box::new(unsafe { std::mem::zeroed::<sys::CSharedEdictChangeInfo>() });

	shared.m_iSerialNumber = 7;
	shared.m_nChangeInfos = 3;
	set_change_accessor(&raw mut accessor);
	set_shared_change_info(&raw mut *shared);

	let mut slot = mock_edict(4, false);
	// SAFETY: The slot is a local that outlives the handle.
	let edict = unsafe { Edict::from_raw(NonNull::from(&mut slot)) };

	// The first change this frame claims the next free change info.
	edict.state_changed(engine, 40);
	assert_eq!(slot._base.m_fStateFlags, FL_EDICT_CHANGED);
	assert_eq!(
		(accessor.m_iChangeInfo, accessor.m_iChangeInfoSerialNumber),
		(3, 7)
	);
	assert_eq!(shared.m_nChangeInfos, 4);
	assert_eq!(shared.m_ChangeInfos[3].m_nChangeOffsets, 1);
	assert_eq!(shared.m_ChangeInfos[3].m_ChangeOffsets[0], 40);

	// A full change releases the change info through the accessor.
	edict.full_state_changed(engine);
	assert_eq!(
		slot._base.m_fStateFlags,
		FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED
	);
	assert_eq!(accessor.m_iChangeInfoSerialNumber, 0);

	// Without the engine's change tracking, every change is a full one.
	set_change_accessor(null_mut());
	set_shared_change_info(null_mut());

	let mut slot = mock_edict(5, false);
	// SAFETY: As for the first slot.
	let edict = unsafe { Edict::from_raw(NonNull::from(&mut slot)) };

	edict.state_changed(engine, 40);
	assert_eq!(
		slot._base.m_fStateFlags,
		FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED
	);
}
