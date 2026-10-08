//! Tests of edicts' reads of the engine's table, and of their state changes
//! reaching the engine's change tracking.

use super::*;
use crate::Game;

use crate::test_support::edicts::{
	edict_table, set_change_accessor, set_shared_change_info, take_flag_changes,
};

use crate::test_support::sdk_core::change_tracking_engine;
use sdk_raw::edicts::{FL_EDICT_CHANGED, FL_EDICT_FULL, FL_FULL_EDICT_CHANGED, MAX_EDICTS};
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

#[test]
fn setting_the_transmit_state_keeps_the_other_flags() {
	let engine = change_tracking_engine();
	let mut slot = mock_edict(6, false);

	slot._base.m_fStateFlags = FL_EDICT_FULL | FL_EDICT_CHANGED | FL_EDICT_ALWAYS;

	// SAFETY: The slot is a local that outlives the handle.
	let edict = unsafe { Edict::from_raw(NonNull::from(&mut slot)) };

	take_flag_changes();
	assert_eq!(edict.transmit_state(), TransmitState::Always);

	edict.set_transmit_state(engine, TransmitState::PvsCheck);
	assert_eq!(edict.transmit_state(), TransmitState::PvsCheck);
	assert_eq!(
		slot._base.m_fStateFlags,
		FL_EDICT_FULL | FL_EDICT_CHANGED | FL_EDICT_PVSCHECK
	);
	assert_eq!(take_flag_changes(), []);

	// The engine is told when the entity stops or starts being sent at all.
	edict.set_transmit_state(engine, TransmitState::DontSend);
	edict.set_transmit_state(engine, TransmitState::DontSend);
	assert_eq!(take_flag_changes(), [6]);

	edict.set_transmit_state(engine, TransmitState::FullCheck);
	assert_eq!(slot._base.m_fStateFlags, FL_EDICT_FULL | FL_EDICT_CHANGED);
	assert_eq!(take_flag_changes(), [6]);

	// A free slot is left alone.
	let mut free = mock_edict(7, true);
	let flags = free._base.m_fStateFlags;
	// SAFETY: As above.
	let edict = unsafe { Edict::from_raw(NonNull::from(&mut free)) };

	edict.set_transmit_state(engine, TransmitState::DontSend);
	assert_eq!(free._base.m_fStateFlags, flags);
	assert_eq!(take_flag_changes(), []);
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

#[test]
fn transmit_checks_read_the_client_and_the_edicts_sent() {
	let mut client = mock_edict(1, false);
	let mut sent = [0_u32; MAX_EDICTS as usize / 32];

	sent[0] = 1 << 1 | 1 << 9;
	sent[2] = 1 << 4;

	// SAFETY: The record is plain data, for which zero is valid.
	let mut info: Box<sys::CCheckTransmitInfo> = Box::new(unsafe { std::mem::zeroed() });

	info.m_pClientEnt = &raw mut client;
	info.m_pTransmitEdict = sent.as_mut_ptr().cast();

	let scope = ();
	let server = crate::test_support::server::null_server(Game::TeamFortress2, &scope);
	// SAFETY: The record, the client and the set are locals that outlive the
	// handle.
	let check = unsafe { TransmitCheck::from_live(server, NonNull::from(&mut *info)) };

	assert_eq!(check.client().map(Edict::index), Some(1));
	assert!(check.is_sent(1));
	assert!(check.is_sent(9));
	assert!(check.is_sent(68));
	assert!(!check.is_sent(2));
	assert!(!check.is_sent(MAX_EDICTS));

	info.m_pClientEnt = null_mut();
	info.m_pTransmitEdict = null_mut();
	assert_eq!(check.client(), None);
	assert!(!check.is_sent(1));
}

#[test]
fn transmit_states_are_read_as_the_game_checks_them() {
	use TransmitState::*;

	for (flags, state) in [
		(0, FullCheck),
		(FL_EDICT_FULL | FL_EDICT_CHANGED, FullCheck),
		(FL_EDICT_ALWAYS, Always),
		(FL_EDICT_PVSCHECK, PvsCheck),
		(FL_EDICT_DONTSEND, DontSend),
		// The game checks `FL_EDICT_DONTSEND` first, then `FL_EDICT_ALWAYS`.
		(FL_EDICT_DONTSEND | FL_EDICT_ALWAYS, DontSend),
		(FL_EDICT_ALWAYS | FL_EDICT_PVSCHECK, Always),
	] {
		assert_eq!(TransmitState::from_flags(flags), state, "{flags:#x}");
		assert_eq!(TransmitState::from_flags(state.flag()), state);
	}
}
