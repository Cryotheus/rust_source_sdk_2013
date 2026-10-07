//! Hand-written ABI of TF2's dominations: the count of unanswered kills that
//! starts one, and the vtable slots of `CTFPlayer::SetNumberofDominations` and
//! `CTFPlayer::GetNumberofDominations`, which keep how many players a player
//! dominates.
//!
//! `CTFGameRules::PlayerKilled` (`game/shared/tf/tf_gamerules.cpp`) calls
//! `CTFGameRules::CalcDominationAndRevenge` for the killer of each player, and
//! for their assister, before the game counts the kill. It starts a domination
//! when the kill makes the victim's
//! [`KillStats::killed_by_unanswered`](crate::tf2::scoreboard::KillStats::killed_by_unanswered)
//! count for the killer [`TF_KILLS_DOMINATION`], and the kill of a player who
//! dominates the killer is a revenge, which ends that domination. Either marks
//! or unmarks the pair in the dominating player's `m_Shared.m_bPlayerDominated`
//! and the dominated player's `m_Shared.m_bPlayerDominatingMe`, one `bool` per
//! entity index, which each player's own client receives, and changes the
//! dominating player's count, which the player resource copies into
//! `m_iActiveDominations` for the scoreboard. The function is not virtual. It
//! does nothing in Mannpower, competitive matches, and Mann vs. Machine.

use crate::abi::CppDestructors;
use crate::vtable_slot;
use std::ffi::c_int;

/// `CTFPlayer::GetNumberofDominations`, which returns how many players the
/// player dominates.
///
/// The generated method takes a `CTFPlayer` receiver. It is the player's
/// primary base, `CBaseEntity`, at the same address, so the method can be
/// called with an entity receiver.
#[doc(alias("GetNumberofDominations"))]
pub type GetNumberOfDominationsFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity) -> c_int;

/// `CTFPlayer::SetNumberofDominations(int)`, which sets how many players the
/// player dominates, clamped from 0 to `MAX_PLAYERS - 1`, 100.
///
/// The receiver is as [`GetNumberOfDominationsFn`]'s.
#[doc(alias("SetNumberofDominations"))]
pub type SetNumberOfDominationsFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, dominations: c_int);

// The generated methods take and return the count as an `int`.
const _: fn(&sys::CTFPlayer__bindgen_vtable) -> unsafe extern "C" fn(*mut sys::CTFPlayer) -> c_int =
	|vtable| vtable.CTFPlayer_GetNumberofDominations;

const _: fn(&sys::CTFPlayer__bindgen_vtable) -> unsafe extern "C" fn(*mut sys::CTFPlayer, c_int) =
	|vtable| vtable.CTFPlayer_SetNumberofDominations;

// The pair is among the virtual functions `CTFPlayer` itself declares
// (`game/server/tf/tf_player.h`), 30 slots after `CommitSuicide`, whose 454
// SourceMod's `sdktools.games/game.tf.txt` gamedata lists under both ABIs. No outside source lists
// these slots: Itanium vtables start with two destructor slots, MSVC's with
// one, and the overloads of `CommitSuicide` sit in reverse order under MSVC.
const _: () = {
	let commit_suicide = vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_CommitSuicide);

	assert!(SET_NUMBER_OF_DOMINATIONS_SLOT == 482 + CppDestructors::VTABLE_SLOTS);
	assert!(GET_NUMBER_OF_DOMINATIONS_SLOT == SET_NUMBER_OF_DOMINATIONS_SLOT + 1);
	assert!(commit_suicide == 454 && commit_suicide < SET_NUMBER_OF_DOMINATIONS_SLOT);
};

/// The slot of `CTFPlayer::GetNumberofDominations` in a TF2 player's primary
/// vtable, from the generated binding: 484 on Windows, 485 on Linux. `CTFBot`
/// keeps the same function at the slot.
#[doc(alias("GetNumberofDominations"))]
pub const GET_NUMBER_OF_DOMINATIONS_SLOT: usize = vtable_slot!(
	sys::CTFPlayer__bindgen_vtable,
	CTFPlayer_GetNumberofDominations
);

/// The slot of `CTFPlayer::SetNumberofDominations` in a TF2 player's primary
/// vtable, from the generated binding: 483 on Windows, 484 on Linux. `CTFBot`
/// keeps the same function at the slot.
#[doc(alias("SetNumberofDominations"))]
pub const SET_NUMBER_OF_DOMINATIONS_SLOT: usize = vtable_slot!(
	sys::CTFPlayer__bindgen_vtable,
	CTFPlayer_SetNumberofDominations
);

/// The unanswered kills of one player by another that make the killer
/// dominate them, `TF_KILLS_DOMINATION` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_KILLS_DOMINATION: c_int = 4;
