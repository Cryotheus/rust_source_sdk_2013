//! Hand-written ABI of which entities the engine sends each client: the
//! vtable slots and signatures of `CBaseEntity`'s transmit methods, and reads
//! of the sets of `CCheckTransmitInfo` they mark entities in.
//!
//! For each client, at each snapshot, the engine has the game's
//! `CServerGameEnts::CheckTransmit` mark the entities the client is sent in
//! the client's `CCheckTransmitInfo`, on the main thread. It goes through the
//! edicts by their transmit flags ([`FL_EDICT_TRANSMIT_STATE`]):
//!
//! 1. An edict flagged [`FL_EDICT_DONTSEND`], or already marked, is skipped.
//! 2. An edict flagged [`FL_EDICT_ALWAYS`] is marked, with the edicts of the
//!    entities it moves with, without calling `SetTransmit`.
//! 3. An edict with none of the flags ([`FL_EDICT_FULLCHECK`]) has its
//!    entity's `ShouldTransmit` return the flag to use for the client. With
//!    [`FL_EDICT_ALWAYS`], the entity's `SetTransmit` marks it as sent
//!    always; without [`FL_EDICT_PVSCHECK`], it is skipped.
//! 4. An entity left to check against the client's potentially visible set
//!    (`FL_EDICT_PVSCHECK`) has its `SetTransmit` mark it if the set or the
//!    client's 3D skybox holds it, and for the clients of SourceTV and
//!    replays.
//!
//! `CBaseEntity::SetTransmit` marks the entity, then has its move parent's
//! `SetTransmit` mark the parent; the game's overrides mark more, such as a
//! character's weapons.
//!
//! [`FL_EDICT_ALWAYS`]: crate::edicts::FL_EDICT_ALWAYS
//! [`FL_EDICT_DONTSEND`]: crate::edicts::FL_EDICT_DONTSEND
//! [`FL_EDICT_FULLCHECK`]: crate::edicts::FL_EDICT_FULLCHECK
//! [`FL_EDICT_PVSCHECK`]: crate::edicts::FL_EDICT_PVSCHECK
//! [`FL_EDICT_TRANSMIT_STATE`]: crate::edicts::FL_EDICT_TRANSMIT_STATE

#[cfg(test)]
#[path = "tests/transmit.rs"]
mod tests;

use crate::edicts::MAX_EDICTS;
use crate::entities::SPAWN_SLOT;
use crate::vtable_slot;
use std::ffi::c_int;

/// The signature of `CBaseEntity::SetTransmit`,
/// `void (CCheckTransmitInfo *, bool)`, which marks the entity, and those it
/// depends on, as sent to the client `info` is for. `always` says whether the
/// entity is sent wherever it is, rather than because the client's
/// potentially visible set holds it.
#[doc(alias("SetTransmit"))]
pub type SetTransmitFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	info: *mut sys::CCheckTransmitInfo,
	always: bool,
);

/// The signature of `CBaseEntity::ShouldTransmit`,
/// `int (const CCheckTransmitInfo *)`, which returns the transmit flag to
/// use for the client `info` is for: `FL_EDICT_ALWAYS`, `FL_EDICT_DONTSEND`
/// or `FL_EDICT_PVSCHECK`.
#[doc(alias("ShouldTransmit"))]
pub type ShouldTransmitFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	info: *const sys::CCheckTransmitInfo,
) -> c_int;

/// The signature of `CBaseEntity::UpdateTransmitState`, `int ()`, which
/// gives the entity's edict the transmit flags its state calls for, and
/// returns the edict's flags.
#[doc(alias("UpdateTransmitState"))]
pub type UpdateTransmitStateFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity) -> c_int;

// `ShouldTransmit`, `UpdateTransmitState`, `SetTransmit` and `GetTracerType`
// directly precede `Spawn`, whose slot is the same for every game, so theirs
// are too.
const _: () = {
	assert!(SHOULD_TRANSMIT_SLOT + 4 == SPAWN_SLOT);
	assert!(UPDATE_TRANSMIT_STATE_SLOT + 3 == SPAWN_SLOT);
	assert!(SET_TRANSMIT_SLOT + 2 == SPAWN_SLOT);
};

// The generated methods have these signatures.
const _: () = {
	use sys::CBaseEntity__bindgen_vtable as Vtable;

	let _: fn(&Vtable) -> SetTransmitFn = |vtable| vtable.CBaseEntity_SetTransmit;
	let _: fn(&Vtable) -> ShouldTransmitFn = |vtable| vtable.CBaseEntity_ShouldTransmit;
	let _: fn(&Vtable) -> UpdateTransmitStateFn = |vtable| vtable.CBaseEntity_UpdateTransmitState;
};

/// The slot of `CBaseEntity::SetTransmit` in an entity's primary vtable,
/// from the generated binding.
#[doc(alias("SetTransmit"))]
pub const SET_TRANSMIT_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_SetTransmit);

/// The slot of `CBaseEntity::ShouldTransmit` in an entity's primary vtable,
/// from the generated binding.
#[doc(alias("ShouldTransmit"))]
pub const SHOULD_TRANSMIT_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_ShouldTransmit);

/// The slot of `CBaseEntity::UpdateTransmitState` in an entity's primary
/// vtable, from the generated binding.
#[doc(alias("UpdateTransmitState"))]
pub const UPDATE_TRANSMIT_STATE_SLOT: usize = vtable_slot!(
	sys::CBaseEntity__bindgen_vtable,
	CBaseEntity_UpdateTransmitState
);

/// Whether the `CBitVec<MAX_EDICTS>` at `bits`, such as a
/// `CCheckTransmitInfo`'s `m_pTransmitEdict`, has the bit of the edict
/// `index` set. An index outside the edict table has none.
///
/// # Safety
///
/// `bits` must be null, which holds no bit, or point to a live
/// `CBitVec<MAX_EDICTS>`, which no other thread writes to meanwhile. The set
/// is read without forming a reference, as the engine writes to it through
/// its own pointers.
pub unsafe fn has_edict_bit(bits: *const u8, index: c_int) -> bool {
	if bits.is_null() || !(0..MAX_EDICTS).contains(&index) {
		return false;
	}

	// `CBitVec` keeps its bits in 32-bit words, from the lowest bit of the
	// first.
	let index = index.cast_unsigned() as usize;

	// SAFETY: As the caller promises, the set is live, and the word lies
	// within its `MAX_EDICTS` bits, which the index was checked against.
	let word = unsafe { bits.cast::<u32>().add(index / 32).read() };

	word & (1 << (index % 32)) != 0
}
