//! Hand-written ABI of vote issues Rust implements for TF2's vote
//! controllers: `CBaseIssue` objects of `game/server/vote_controller.h` whose
//! vtables mix functions of the game's own issues with Rust's.
//!
//! An issue's vtable is built from a game issue's, [`live_vtable`] replacing
//! the methods a custom issue implements, and [`dead_vtable`] making a table
//! of the game's functions alone, to switch an issue to before the module
//! with its Rust methods unloads. Both are [`TaggedVtable`]s, which keep the
//! game issue's RTTI before their methods, and the issues that use them carry
//! [`ISSUE_TAG`] after their type string, so that a module loaded later finds
//! the issues an earlier one left behind.

use crate::entities::INVALID_EHANDLE_INDEX;
use std::ffi::{CStr, c_char, c_int};
use std::mem::{offset_of, transmute};
use std::ptr;

/// An issue's destructor slots: MSVC's deleting destructor, which destroys
/// the issue, then frees it if bit 0 of its flags is set, and returns it; or
/// the Itanium ABI's complete and deleting destructors.
#[cfg(target_os = "linux")]
pub type IssueDestructors = (
	unsafe extern "C" fn(this: *mut sys::CBaseIssue),
	unsafe extern "C" fn(this: *mut sys::CBaseIssue),
);

/// An issue's destructor slots: MSVC's deleting destructor, which destroys
/// the issue, then frees it if bit 0 of its flags is set, and returns it; or
/// the Itanium ABI's complete and deleting destructors.
#[cfg(target_os = "windows")]
pub type IssueDestructors = unsafe extern "C" fn(
	this: *mut sys::CBaseIssue,
	flags: std::ffi::c_uint,
) -> *mut std::ffi::c_void;

/// A vote controller's list of issues, `CUtlVector<CBaseIssue *>`.
pub type IssueList = sys::CUtlVector<*mut sys::CBaseIssue, sys::CUtlMemory<*mut sys::CBaseIssue>>;

/// An issue's vtable, as the game calls it.
pub type IssueVtable = sys::CBaseIssue__bindgen_vtable;

/// The signature of `CBaseIssue::ProcessResults`, which decides whether a
/// vote that ended passed, from its options, the votes for each, the voters'
/// choices, the option with the most votes, and the votes cast and possible.
#[doc(alias("ProcessResults"))]
pub type ProcessResultsFn = unsafe extern "C" fn(
	this: *mut sys::CBaseIssue,
	options: *const sys::CUtlVector<*const c_char, sys::CUtlMemory<*const c_char>>,
	counts: *const c_int,
	votes: *const sys::__BindgenOpaqueArray8<[u8; 40]>,
	highest: c_int,
	total: c_int,
	potential: c_int,
) -> sys::CBaseIssue_EVoteAction;

/// What precedes the methods of a C++ vtable, which `dynamic_cast` and
/// `typeid` read: MSVC's pointer to the class's RTTI Complete Object Locator.
#[cfg(target_os = "windows")]
pub type VtableRtti = [usize; 1];

/// What precedes the methods of a C++ vtable, which `dynamic_cast` and
/// `typeid` read: the Itanium ABI's offset from the table's object to the
/// complete object, and pointer to the class's `type_info`.
#[cfg(target_os = "linux")]
pub type VtableRtti = [usize; 2];

// The MSVC binding describes the controller's fields.
#[cfg(target_os = "windows")]
const _: () = assert!(
	offset_of!(sys::CVoteController, m_iActiveIssueIndex) == ACTIVE_ISSUE_INDEX_OFFSET
		&& offset_of!(sys::CVoteController, m_nVoteIdx) == VOTE_INDEX_OFFSET
		&& offset_of!(sys::CVoteController, m_potentialIssues) == POTENTIAL_ISSUES_OFFSET
		&& offset_of!(sys::CVoteController, m_potentialIssues) % align_of::<IssueList>() == 0
);

// The game reads the RTTI right before the methods.
const _: () = assert!(offset_of!(TaggedVtable, methods) == size_of::<VtableRtti>());

// The generated method has the signature of `ProcessResultsFn`.
const _: fn(&IssueVtable) -> ProcessResultsFn = |vtable| vtable.CBaseIssue_ProcessResults;

/// The offset of a vote controller's `m_iActiveIssueIndex`, its first
/// variable after its `CBaseEntity`, which TF2 networks.
///
/// `CVoteController`'s own variables are laid out alike under both ABIs, while
/// the `CBaseEntity` before them is not, and the Itanium binding of the
/// controller is opaque, so the offsets are counted back from its end.
pub const ACTIVE_ISSUE_INDEX_OFFSET: usize = size_of::<sys::CVoteController>() - 240;

/// What a [`TaggedVtable`] holds after its methods when its issues only call
/// the game's functions, and do nothing.
pub const DEAD_VTABLE: u64 = u64::from_le_bytes(*b"rsvote:D");

/// The bytes at the end of a custom issue's type string, past the name and its
/// NUL, which mark it as an issue Rust made. The game only reads the type
/// string up to its NUL, and nothing writes past it after the issue is made.
pub const ISSUE_TAG: [u8; 16] = *b"\0rs-sdk-issue:1\0";

/// Where [`ISSUE_TAG`] starts in a custom issue's type string.
pub const ISSUE_TAG_OFFSET: usize = MAX_VOTE_DETAILS_LENGTH - ISSUE_TAG.len();

/// What a [`TaggedVtable`] holds after its methods when its issues call into
/// a loaded module.
pub const LIVE_VTABLE: u64 = u64::from_le_bytes(*b"rsvote:L");

/// The longest name a tagged issue's type string holds: the bytes before
/// [`ISSUE_TAG_OFFSET`], less the name's NUL.
pub const MAX_ISSUE_NAME_LEN: usize = ISSUE_TAG_OFFSET - 1;

/// The length of an issue's type and details strings, their NUL included
/// (`MAX_VOTE_DETAILS_LENGTH` of `game/shared/shareddefs.h`).
pub const MAX_VOTE_DETAILS_LENGTH: usize = 64;

/// The offset of a vote controller's `m_potentialIssues`, its list of
/// issues, which TF2 does not network: 44 bytes after `m_nVoteIdx`, past the
/// map of votes by Steam ID.
pub const POTENTIAL_ISSUES_OFFSET: usize = size_of::<sys::CVoteController>() - 104;

/// `CBaseIssue::EVoteAction::eVoteAction_Fail`, what `ProcessResults` returns
/// for a vote that failed.
#[doc(alias("eVoteAction_Fail"))]
pub const VOTE_ACTION_FAIL: sys::CBaseIssue_EVoteAction =
	sys::CBaseIssue_EVoteAction_eVoteAction_Fail;

/// `CBaseIssue::EVoteAction::eVoteAction_Pass`, what `ProcessResults` returns
/// for a vote that passed.
#[doc(alias("eVoteAction_Pass"))]
pub const VOTE_ACTION_PASS: sys::CBaseIssue_EVoteAction =
	sys::CBaseIssue_EVoteAction_eVoteAction_Pass;

/// The offset of a vote controller's `m_nVoteIdx`, which TF2 networks.
pub const VOTE_INDEX_OFFSET: usize = size_of::<sys::CVoteController>() - 148;

/// The methods of `CBaseIssue` a custom issue implements, which
/// [`live_vtable`] puts in place of a game issue's.
#[derive(Debug, Clone, Copy)]
pub struct IssueMethods {
	/// The destructors, which the vote controller calls as it deletes its
	/// issues, at the end of the level.
	pub destructors: IssueDestructors,

	/// `const char *GetTypeStringLocalized()`: the localization token of the
	/// issue's row in the Call Vote menu, or empty for `#Vote_` and its type
	/// string.
	pub get_type_string_localized:
		unsafe extern "C" fn(this: *mut sys::CBaseIssue) -> *const c_char,

	/// `void SetIssueDetails(const char *details)`, which the controller calls
	/// with the vote's argument once `RequestCallVote` accepted it.
	pub set_issue_details: unsafe extern "C" fn(this: *mut sys::CBaseIssue, details: *const c_char),

	/// `void OnVoteFailed(int iEntityHoldingVote)`, as a vote fails.
	pub on_vote_failed: unsafe extern "C" fn(this: *mut sys::CBaseIssue, caller: c_int),

	/// `bool IsEnabled()`: whether the menu lists the issue.
	pub is_enabled: unsafe extern "C" fn(this: *mut sys::CBaseIssue) -> bool,

	/// `bool RequestCallVote(int iEntIndex, const char *pszDetails,
	/// vote_create_failed_t &nFailCode, int &nTime)`.
	pub request_call_vote: unsafe extern "C" fn(
		this: *mut sys::CBaseIssue,
		caller: c_int,
		details: *const c_char,
		failure: *mut sys::vote_create_failed_t,
		time: *mut c_int,
	) -> bool,

	/// `const char *GetDisplayString()`: the poll's question, filled with the
	/// details string as `%s1`.
	pub get_display_string: unsafe extern "C" fn(this: *mut sys::CBaseIssue) -> *const c_char,

	/// `void ExecuteCommand()`, a while after the vote passed.
	pub execute_command: unsafe extern "C" fn(this: *mut sys::CBaseIssue),

	/// `void ListIssueDetails(CBasePlayer *pForWhom)`, for `listissues`.
	pub list_issue_details:
		unsafe extern "C" fn(this: *mut sys::CBaseIssue, player: *mut sys::CBasePlayer),

	/// `const char *GetVotePassedString()`: the passed line, filled with the
	/// details string as `%s1`.
	pub get_vote_passed_string: unsafe extern "C" fn(this: *mut sys::CBaseIssue) -> *const c_char,

	/// `EVoteAction ProcessResults(...)`, as the vote ends.
	pub process_results: ProcessResultsFn,
}

/// A vtable an SDK made for custom issues, after the RTTI of the game issue
/// it was made from, and followed by [`LIVE_VTABLE`] or [`DEAD_VTABLE`], so
/// that an issue's state can be read from the table it points to.
///
/// Issues point to the table's methods, as [`TaggedVtable::issue_vtable`]
/// gives them, and the game reads the RTTI right before them as it casts an
/// issue with `dynamic_cast` or reads its `typeid`, as TF2 does to every vote
/// that passes, to tell whether it is a kick vote.
#[repr(C)]
pub struct TaggedVtable {
	/// The RTTI of the game issue the table was made from, which the game then
	/// takes custom issues for.
	pub rtti: VtableRtti,

	/// The methods, which the game calls.
	pub methods: IssueVtable,

	/// [`LIVE_VTABLE`] or [`DEAD_VTABLE`].
	pub state: u64,
}

impl TaggedVtable {
	/// The table whose methods are at `vtable`, the vtable of an issue using
	/// it.
	pub fn from_issue_vtable(vtable: *const IssueVtable) -> *const Self {
		vtable.wrapping_byte_sub(offset_of!(Self, methods)).cast()
	}

	/// The vtable of an issue using the table at `this`: its methods.
	pub fn issue_vtable(this: *const Self) -> *const IssueVtable {
		this.wrapping_byte_add(offset_of!(Self, methods)).cast()
	}
}

/// A table of `base`'s functions alone, for issues whose module unloads,
/// which leaves them in their controller's menu, hidden and refusing every
/// call, doing nothing as they pass, fail, or are deleted, and failing a vote
/// on them that is under way as it ends.
///
/// - The destructors, `OnVoteFailed`, `ExecuteCommand` and
///   `ListIssueDetails` are `OnVoteStarted`, which does nothing. The issue is
///   never freed.
/// - `IsEnabled` and `RequestCallVote` are `IsTeamRestrictedVote`, which
///   returns false.
/// - `ProcessResults` is `GetNumberVoteOptions`, which returns 2,
///   [`VOTE_ACTION_FAIL`].
/// - The rest are `base`'s: its `GetTypeStringLocalized` returns an empty
///   string, and its display and passed strings are its own.
///
/// # Safety
///
/// `base` must be the vtable of an issue whose class does not override
/// `CBaseIssue`'s `OnVoteStarted`, `IsTeamRestrictedVote`,
/// `GetNumberVoteOptions`, and `GetTypeStringLocalized`, such as TF2's
/// `CRestartGameIssue`, in a module that outlives the issues using the table.
/// Under the x86-64 calling conventions of both targets, the caller passes
/// the arguments and cleans them up, so a function that takes fewer, and
/// returns what the caller ignores or the same integer type, can stand in for
/// one that takes more.
pub unsafe fn dead_vtable(base: &IssueVtable) -> IssueVtable {
	let nothing = base.CBaseIssue_OnVoteStarted;
	let refuse = base.CBaseIssue_IsTeamRestrictedVote;

	// SAFETY: The functions take the issue first and ignore what follows, as
	// the caller promises of `base`. The destructors' return value is ignored
	// by `delete`, `RequestCallVote` and `IsEnabled` return a `bool` as
	// `IsTeamRestrictedVote` does, and `ProcessResults` an `int` enumeration as
	// `GetNumberVoteOptions` returns an `int`.
	unsafe {
		IssueVtable {
			#[cfg(target_os = "windows")]
			CBaseIssue_destructor: transmute::<
				unsafe extern "C" fn(*mut sys::CBaseIssue),
				IssueDestructors,
			>(nothing),
			#[cfg(target_os = "linux")]
			CBaseIssue_complete_destructor: nothing,
			#[cfg(target_os = "linux")]
			CBaseIssue_deleting_destructor: nothing,
			CBaseIssue_GetTypeStringLocalized: base.CBaseIssue_GetTypeStringLocalized,
			CBaseIssue_GetDetailsString: base.CBaseIssue_GetDetailsString,
			CBaseIssue_SetIssueDetails: base.CBaseIssue_SetIssueDetails,
			CBaseIssue_OnVoteFailed: transmute::<
				unsafe extern "C" fn(*mut sys::CBaseIssue),
				unsafe extern "C" fn(*mut sys::CBaseIssue, c_int),
			>(nothing),
			CBaseIssue_OnVoteStarted: nothing,
			CBaseIssue_IsEnabled: refuse,
			CBaseIssue_CanTeamCallVote: base.CBaseIssue_CanTeamCallVote,
			CBaseIssue_RequestCallVote: transmute::<
				unsafe extern "C" fn(*mut sys::CBaseIssue) -> bool,
				unsafe extern "C" fn(
					*mut sys::CBaseIssue,
					c_int,
					*const c_char,
					*mut sys::vote_create_failed_t,
					*mut c_int,
				) -> bool,
			>(refuse),
			CBaseIssue_IsTeamRestrictedVote: refuse,
			CBaseIssue_GetDisplayString: base.CBaseIssue_GetDisplayString,
			CBaseIssue_ExecuteCommand: nothing,
			CBaseIssue_ListIssueDetails: transmute::<
				unsafe extern "C" fn(*mut sys::CBaseIssue),
				unsafe extern "C" fn(*mut sys::CBaseIssue, *mut sys::CBasePlayer),
			>(nothing),
			CBaseIssue_GetVotePassedString: base.CBaseIssue_GetVotePassedString,
			CBaseIssue_CountPotentialVoters: base.CBaseIssue_CountPotentialVoters,
			CBaseIssue_GetNumberVoteOptions: base.CBaseIssue_GetNumberVoteOptions,
			CBaseIssue_IsYesNoVote: base.CBaseIssue_IsYesNoVote,
			CBaseIssue_GetVoteOptions: base.CBaseIssue_GetVoteOptions,
			CBaseIssue_BRecordVoteFailureEventForEntity: base
				.CBaseIssue_BRecordVoteFailureEventForEntity,
			CBaseIssue_GetQuorumRatio: base.CBaseIssue_GetQuorumRatio,
			CBaseIssue_ProcessResults: transmute::<
				unsafe extern "C" fn(*mut sys::CBaseIssue) -> c_int,
				ProcessResultsFn,
			>(base.CBaseIssue_GetNumberVoteOptions),
			CBaseIssue_OnVoteEnded: base.CBaseIssue_OnVoteEnded,
			CBaseIssue_OnPlayerDisconnected: base.CBaseIssue_OnPlayerDisconnected,
		}
	}
}

/// Whether the issue at `issue` carries [`ISSUE_TAG`] after its type string.
///
/// # Safety
///
/// `issue` must point to a live `CBaseIssue`.
pub unsafe fn is_tagged(issue: *const sys::CBaseIssue) -> bool {
	// SAFETY: As the caller promises; the bytes are read without forming a
	// reference to the issue.
	let type_string = unsafe { ptr::read(&raw const (*issue).m_szTypeString) };

	type_string[ISSUE_TAG_OFFSET..]
		.iter()
		.map(|&byte| byte as u8)
		.eq(ISSUE_TAG)
}

/// A copy of `base` in which the methods of a custom issue are `methods`, and
/// the rest the game's own.
pub fn live_vtable(base: &IssueVtable, methods: &IssueMethods) -> IssueVtable {
	IssueVtable {
		#[cfg(target_os = "windows")]
		CBaseIssue_destructor: methods.destructors,
		#[cfg(target_os = "linux")]
		CBaseIssue_complete_destructor: methods.destructors.0,
		#[cfg(target_os = "linux")]
		CBaseIssue_deleting_destructor: methods.destructors.1,
		CBaseIssue_GetTypeStringLocalized: methods.get_type_string_localized,
		CBaseIssue_GetDetailsString: base.CBaseIssue_GetDetailsString,
		CBaseIssue_SetIssueDetails: methods.set_issue_details,
		CBaseIssue_OnVoteFailed: methods.on_vote_failed,
		CBaseIssue_OnVoteStarted: base.CBaseIssue_OnVoteStarted,
		CBaseIssue_IsEnabled: methods.is_enabled,
		CBaseIssue_CanTeamCallVote: base.CBaseIssue_CanTeamCallVote,
		CBaseIssue_RequestCallVote: methods.request_call_vote,
		CBaseIssue_IsTeamRestrictedVote: base.CBaseIssue_IsTeamRestrictedVote,
		CBaseIssue_GetDisplayString: methods.get_display_string,
		CBaseIssue_ExecuteCommand: methods.execute_command,
		CBaseIssue_ListIssueDetails: methods.list_issue_details,
		CBaseIssue_GetVotePassedString: methods.get_vote_passed_string,
		CBaseIssue_CountPotentialVoters: base.CBaseIssue_CountPotentialVoters,
		CBaseIssue_GetNumberVoteOptions: base.CBaseIssue_GetNumberVoteOptions,
		CBaseIssue_IsYesNoVote: base.CBaseIssue_IsYesNoVote,
		CBaseIssue_GetVoteOptions: base.CBaseIssue_GetVoteOptions,
		CBaseIssue_BRecordVoteFailureEventForEntity: base
			.CBaseIssue_BRecordVoteFailureEventForEntity,
		CBaseIssue_GetQuorumRatio: base.CBaseIssue_GetQuorumRatio,
		CBaseIssue_ProcessResults: methods.process_results,
		CBaseIssue_OnVoteEnded: base.CBaseIssue_OnVoteEnded,
		CBaseIssue_OnPlayerDisconnected: base.CBaseIssue_OnPlayerDisconnected,
	}
}

/// A new custom issue named `name` of `controller`, using `vtable`: an
/// issue as `CBaseIssue`'s constructor leaves one, with no target player, no
/// failed votes, and [`ISSUE_TAG`] after its name. It is not registered with
/// the controller.
///
/// Returns `None` if the name is longer than [`MAX_ISSUE_NAME_LEN`].
pub fn new_issue(
	name: &CStr,
	vtable: *const IssueVtable,
	controller: *mut sys::CVoteController,
) -> Option<sys::CBaseIssue> {
	let type_string = tagged_type_string(name)?;

	// SAFETY: Every field of an issue is an integer, a float, a raw pointer, or
	// an array or structure of them, for which zero is valid. An empty
	// `CUtlVector` has no memory, and no growth size.
	let mut issue: sys::CBaseIssue = unsafe { std::mem::zeroed() };

	issue.vtable_ = vtable;
	issue.m_hPlayerTarget._base.m_Index = INVALID_EHANDLE_INDEX;
	issue.m_szTypeString = type_string;
	issue.m_flNextCallTime = -1.0;
	issue.m_pVoteController = controller;

	Some(issue)
}

/// The list of issues of the vote controller at `controller`.
///
/// # Safety
///
/// `controller` must point to a live `CVoteController`, laid out as the
/// generated binding's size and [`POTENTIAL_ISSUES_OFFSET`] describe.
pub const unsafe fn potential_issues(controller: *mut sys::CVoteController) -> *mut IssueList {
	// SAFETY: As the caller promises, the list is within the controller.
	unsafe { controller.byte_add(POTENTIAL_ISSUES_OFFSET).cast() }
}

/// The type string of a custom issue named `name`: the name, its NUL, and
/// [`ISSUE_TAG`] at [`ISSUE_TAG_OFFSET`], or `None` if the name is longer than
/// [`MAX_ISSUE_NAME_LEN`].
pub fn tagged_type_string(name: &CStr) -> Option<[c_char; MAX_VOTE_DETAILS_LENGTH]> {
	let name = name.to_bytes();

	if name.len() > MAX_ISSUE_NAME_LEN {
		return None;
	}

	let mut type_string = [0; MAX_VOTE_DETAILS_LENGTH];

	for (to, &from) in type_string.iter_mut().zip(name) {
		*to = from as c_char;
	}

	for (to, &from) in type_string[ISSUE_TAG_OFFSET..].iter_mut().zip(&ISSUE_TAG) {
		*to = from as c_char;
	}

	Some(type_string)
}

/// The RTTI before the methods of the C++ vtable at `vtable`.
///
/// # Safety
///
/// `vtable` must be the vtable of an object of a polymorphic C++ class, as
/// the compiler made it, which its RTTI precedes.
pub unsafe fn vtable_rtti(vtable: *const IssueVtable) -> VtableRtti {
	// SAFETY: As the caller promises.
	unsafe { vtable.cast::<VtableRtti>().sub(1).read() }
}
