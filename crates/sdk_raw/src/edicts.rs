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

/// `FL_EDICT_ALWAYS` from `public/edict.h`: the entity is sent to every
/// client, wherever it is.
pub const FL_EDICT_ALWAYS: c_int = 1 << 3;

/// `FL_EDICT_CHANGED` from `public/edict.h`, set when a networked variable changes.
pub const FL_EDICT_CHANGED: c_int = 1 << 0;

/// `FL_EDICT_DIRTY_PVS_INFORMATION` from `public/edict.h`, set when the
/// areas and clusters the entity is in must be computed anew, as
/// `CServerNetworkProperty::MarkPVSInformationDirty` does.
pub const FL_EDICT_DIRTY_PVS_INFORMATION: c_int = 1 << 7;

/// `FL_EDICT_DONTSEND` from `public/edict.h`: the entity is sent to no
/// client. `CBaseEntity::UpdateTransmitState` gives it to an entity drawn
/// with `EF_NODRAW`, unless another entity moves with it.
pub const FL_EDICT_DONTSEND: c_int = 1 << 4;

/// `FL_EDICT_FREE` from `public/edict.h`, set while a slot holds no entity.
pub const FL_EDICT_FREE: c_int = 1 << 1;

/// `FL_EDICT_FULL` from `public/edict.h`: the slot holds a full server
/// entity.
pub const FL_EDICT_FULL: c_int = 1 << 2;

/// `FL_EDICT_FULLCHECK` from `public/edict.h`, the absence of the other
/// transmit flags: the entity's `ShouldTransmit` decides for each client
/// whether it is sent.
pub const FL_EDICT_FULLCHECK: c_int = 0;

/// `FL_EDICT_PVSCHECK` from `public/edict.h`: the entity is sent to the
/// clients whose potentially visible set holds it.
pub const FL_EDICT_PVSCHECK: c_int = 1 << 5;

/// The flags of an edict that say which clients its entity is sent to, which
/// `CBaseEdict::ClearTransmitState` clears.
pub const FL_EDICT_TRANSMIT_STATE: c_int = FL_EDICT_ALWAYS | FL_EDICT_DONTSEND | FL_EDICT_PVSCHECK;

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
#[doc(alias("StateChanged"))]
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
#[doc(alias("StateChanged"))]
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

	let Some((tracking, shared)) = change_info() else {
		// SAFETY: The caller upholds the contract.
		return unsafe { full_state_changed(edict, None) };
	};

	let accessor = tracking.as_ptr();
	let shared = shared.as_ptr();

	// SAFETY: Both structures are the engine's, accessed by no other thread.
	// Indices are checked against the fixed array lengths before use, and no
	// references are formed.
	unsafe {
		let serial_number = (&raw const (*shared).m_iSerialNumber).read();
		let infos = (&raw mut (*shared).m_ChangeInfos).cast::<sys::CEdictChangeInfo>();
		let fully_changed = || full_state_changed(edict, Some(tracking));

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
