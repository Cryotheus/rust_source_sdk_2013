//! Hand-written ABI of TF2's vote issues and their vtables: the vote
//! `#define`s of `game/shared/shareddefs.h`, `CBaseIssue::RequestCallVote`'s
//! slot and signature, and the search for each issue class's vtable.

use crate::tf2::class_targets::SlotVtables;
use crate::vtable_slot;
use std::ffi::{c_char, c_int};

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its vote issue classes, such as `CKickIssue`: those
/// whose [`REQUEST_CALL_VOTE_SLOT`] entries are executable.
pub type IssueVtables = SlotVtables<REQUEST_CALL_VOTE_SLOT>;

/// The signature of `CBaseIssue::RequestCallVote`, which decides whether the
/// player with entity index `caller`, or [`DEDICATED_SERVER`], may call a
/// vote on the issue with `details`, writing why not and for how long to
/// `failure` and `time` when it may not.
#[doc(alias("RequestCallVote"))]
pub type RequestCallVoteFn = unsafe extern "C" fn(
	this: *mut sys::CBaseIssue,
	caller: c_int,
	details: *const c_char,
	failure: *mut sys::vote_create_failed_t,
	time: *mut c_int,
) -> bool;

// The generated method has the signature of `RequestCallVoteFn`.
const _: fn(&sys::CBaseIssue__bindgen_vtable) -> RequestCallVoteFn =
	|vtable| vtable.CBaseIssue_RequestCallVote;

/// The caller index TF2 uses for server-initiated and automatic votes, in
/// place of a player's entity index.
pub const DEDICATED_SERVER: c_int = 99;

/// The most options a vote can offer.
pub const MAX_VOTE_OPTIONS: usize = 5;

/// The slot of `CBaseIssue::RequestCallVote` in an issue's primary vtable,
/// from the generated binding.
#[doc(alias("RequestCallVote"))]
pub const REQUEST_CALL_VOTE_SLOT: usize =
	vtable_slot!(sys::CBaseIssue__bindgen_vtable, CBaseIssue_RequestCallVote);
