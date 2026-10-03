//! Hand-written ABI of TF2's vote issues and their vtables: the vote
//! `#define`s of `game/shared/shareddefs.h`, `CBaseIssue::RequestCallVote`'s
//! slot and signature, and the search for each issue class's vtable.

use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, Image};
use crate::vtable_slot;
use std::ffi::{c_char, c_int, c_void};
use std::ptr::NonNull;

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

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its vote issue classes.
#[derive(Debug, Clone)]
pub struct IssueVtables(Image);

impl IssueVtables {
	/// Snapshots the module whose `CreateInterface` export is `factory`, such
	/// as the game server module.
	///
	/// # Safety
	///
	/// `factory` must be the `CreateInterface` export of a module that stays
	/// loaded throughout this call.
	pub unsafe fn load(factory: CreateInterfaceFn) -> Result<Self, util::Error> {
		// SAFETY: The factory is an executable address in its module, which the
		// caller keeps loaded while it is inspected.
		unsafe { Image::load(factory as usize) }.map(Self)
	}

	/// The unique primary vtable of the global C++ class named `class`, such
	/// as `CKickIssue`, whose [`REQUEST_CALL_VOTE_SLOT`] entry is executable,
	/// from its run-time type information. Returns `None` if there is no such
	/// table or more than one.
	///
	/// The address is metadata from the snapshot: it does not keep the module
	/// loaded, and the table is the class's only while the module that
	/// [`Self::load`] snapshot stays loaded.
	pub fn find(&self, class: &str) -> Option<NonNull<*mut c_void>> {
		NonNull::new(self.0.primary_vtable(class, REQUEST_CALL_VOTE_SLOT)? as *mut *mut c_void)
	}
}
