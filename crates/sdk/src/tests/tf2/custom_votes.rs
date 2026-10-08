//! Tests of `crate::tf2::custom_votes`: custom issues in a mock vote
//! controller, whose restart vote's methods are stand-ins for TF2's.

use super::*;
use crate::test_support::server::mock_binding;
use sdk_raw::tf2::custom_votes::{IssueList, VtableRtti, tagged_type_string};
use std::cell::Cell;

#[cfg(target_os = "windows")]
use std::ffi::{c_uint, c_void};

use std::ptr;

thread_local! {
	/// What the mock restart vote's `ProcessResults` returns.
	static RESULT: Cell<sys::CBaseIssue_EVoteAction> = const { Cell::new(VOTE_ACTION_PASS) };
}

/// The mock restart vote's RTTI, which custom issues keep.
const RESTART_RTTI: VtableRtti = [0x5254_5449; size_of::<VtableRtti>() / size_of::<usize>()];

/// The mock restart vote's vtable, after its RTTI, with `CBaseIssue`'s
/// behavior where the dead table borrows it.
static RESTART_VTABLE: RttiVtable = RttiVtable {
	rtti: RESTART_RTTI,
	methods: IssueVtable {
		#[cfg(target_os = "windows")]
		CBaseIssue_destructor: base_destructor,
		#[cfg(target_os = "linux")]
		CBaseIssue_complete_destructor: base_nothing,
		#[cfg(target_os = "linux")]
		CBaseIssue_deleting_destructor: base_nothing,
		CBaseIssue_GetTypeStringLocalized: base_empty,
		CBaseIssue_GetDetailsString: base_details,
		CBaseIssue_SetIssueDetails: base_set_details,
		CBaseIssue_OnVoteFailed: base_with_int,
		CBaseIssue_OnVoteStarted: base_nothing,
		CBaseIssue_IsEnabled: base_true,
		CBaseIssue_CanTeamCallVote: base_team,
		CBaseIssue_RequestCallVote: base_request,
		CBaseIssue_IsTeamRestrictedVote: base_false,
		CBaseIssue_GetDisplayString: base_restart_text,
		CBaseIssue_ExecuteCommand: base_nothing,
		CBaseIssue_ListIssueDetails: base_list,
		CBaseIssue_GetVotePassedString: base_restart_text,
		CBaseIssue_CountPotentialVoters: base_two,
		CBaseIssue_GetNumberVoteOptions: base_two,
		CBaseIssue_IsYesNoVote: base_true,
		CBaseIssue_GetVoteOptions: base_options,
		CBaseIssue_BRecordVoteFailureEventForEntity: base_team,
		CBaseIssue_GetQuorumRatio: base_ratio,
		CBaseIssue_ProcessResults: base_process,
		CBaseIssue_OnVoteEnded: base_nothing,
		CBaseIssue_OnPlayerDisconnected: base_list,
	},
};

/// A mock vote controller.
struct Controller {
	raw: NonNull<sys::CVoteController>,
	restart: NonNull<sys::CBaseIssue>,
}

impl Controller {
	/// Adds an issue after the others.
	fn add(&self, issue: *mut sys::CBaseIssue) {
		unsafe {
			let list = potential_issues(self.raw.as_ptr());
			(*list)
				.m_Memory
				.m_pMemory
				.add((*list).m_Size as usize)
				.write(issue);
			(*list).m_Size += 1;
		}
	}

	/// Attaches the installed votes to the controller.
	fn attach(&self) -> Result<(), CustomVoteError> {
		unsafe { attach_to(self.raw, self.restart) }
	}

	/// The controller's issues.
	fn issues(&self) -> Vec<*mut sys::CBaseIssue> {
		unsafe {
			let list = potential_issues(self.raw.as_ptr());
			std::slice::from_raw_parts((*list).m_Memory.m_pMemory, (*list).m_Size as usize).to_vec()
		}
	}

	/// Removes the last issue, as deleting it.
	fn remove_last(&self) {
		unsafe { (*potential_issues(self.raw.as_ptr())).m_Size -= 1 };
	}
}

/// A vote that only has a name.
struct NamedVote(&'static CStr);

impl CustomVote for NamedVote {
	fn call(&self, _: Server<'_>, _: VoteCall<'_>) -> Result<VoteText, VoteRefusal> {
		Err(VoteRefusal::Generic)
	}

	fn label(&self) -> &'static CStr {
		c"#Vote_RestartGame"
	}

	fn name(&self) -> &'static CStr {
		self.0
	}

	fn offered(&self, _: Server<'_>) -> bool {
		true
	}

	fn pass(&self, _: Server<'_>, _: &CStr) {}
}

/// A vtable after its RTTI, as compilers lay them out.
#[repr(C)]
struct RttiVtable {
	rtti: VtableRtti,
	methods: IssueVtable,
}

/// A difficulty vote that offers itself as `offered` says, and notes the
/// arguments it passed with.
#[derive(Default)]
struct TestVote {
	offered: Rc<Cell<bool>>,
	passed: Rc<RefCell<Vec<CString>>>,
}

impl CustomVote for TestVote {
	fn call(&self, _: Server<'_>, call: VoteCall<'_>) -> Result<VoteText, VoteRefusal> {
		match call.argument.to_bytes() {
			b"" | b"veteran" => Ok(VoteText {
				argument: c"veteran".to_owned(),
				question: c"Change the difficulty to Veteran?".to_owned(),
				passed: c"The difficulty will be Veteran from the next round.".to_owned(),
			}),

			_ => Err(VoteRefusal::InvalidArgument),
		}
	}

	fn label(&self) -> &'static CStr {
		c"#TF_MvM_Difficulty"
	}

	fn name(&self) -> &'static CStr {
		c"Difficulty"
	}

	fn offered(&self, _: Server<'_>) -> bool {
		self.offered.get()
	}

	fn pass(&self, _: Server<'_>, argument: &CStr) {
		self.passed.borrow_mut().push(argument.to_owned());
	}
}

#[test]
fn a_full_controller_takes_nothing() {
	let controller = controller(1);
	let (_votes, _calls) = install();

	assert!(matches!(
		controller.attach(),
		Err(CustomVoteError::NoRoom { needed: 1, free: 0 })
	));
	assert_eq!(controller.issues().len(), 1);
}

#[test]
fn another_issue_with_the_name_is_refused() {
	let controller = controller(16);
	let (_votes, _calls) = install();
	let mut other = new_issue(c"difficulty", restart_vtable(), controller.raw.as_ptr()).unwrap();

	other.m_szTypeString[ISSUE_TAG_OFFSET..].fill(0);
	controller.add(Box::leak(Box::new(other)));

	assert!(matches!(
		controller.attach(),
		Err(CustomVoteError::NameTaken(name)) if name.as_c_str() == c"Difficulty"
	));
	assert_eq!(controller.issues().len(), 2);
}

#[test]
fn attaching_adds_tagged_live_issues_and_keeps_them() {
	let controller = controller(16);
	let (votes, _calls) = install();

	controller.attach().unwrap();

	let issues = controller.issues();
	assert_eq!(issues.len(), 2);
	assert!(unsafe { is_tagged(issues[1]) });
	assert_eq!(
		unsafe { type_string(NonNull::new(issues[1]).unwrap()) }
			.unwrap()
			.as_c_str(),
		c"Difficulty"
	);
	assert_eq!(state(issues[1]), LIVE_VTABLE);
	assert_eq!(
		unsafe { (*issues[1]).m_pVoteController },
		controller.raw.as_ptr()
	);
	assert!(votes.is_attached());

	// Attaching again adds nothing.
	controller.attach().unwrap();
	assert_eq!(controller.issues().len(), 2);
}

/// MSVC's destructor, which does nothing.
#[cfg(target_os = "windows")]
unsafe extern "C" fn base_destructor(this: *mut sys::CBaseIssue, _: c_uint) -> *mut c_void {
	this.cast()
}

unsafe extern "C" fn base_details(this: *mut sys::CBaseIssue) -> *const c_char {
	unsafe { (&raw const (*this).m_szDetailsString).cast() }
}

unsafe extern "C" fn base_empty(_: *mut sys::CBaseIssue) -> *const c_char {
	c"".as_ptr()
}

unsafe extern "C" fn base_false(_: *mut sys::CBaseIssue) -> bool {
	false
}

unsafe extern "C" fn base_list(_: *mut sys::CBaseIssue, _: *mut sys::CBasePlayer) {}

/// `OnVoteStarted`, `ExecuteCommand`, `OnVoteEnded`, and the Itanium
/// destructors: nothing.
unsafe extern "C" fn base_nothing(_: *mut sys::CBaseIssue) {}

unsafe extern "C" fn base_options(
	_: *mut sys::CBaseIssue,
	_: *mut sys::CUtlVector<*const c_char, sys::CUtlMemory<*const c_char>>,
) -> bool {
	true
}

unsafe extern "C" fn base_process(
	_: *mut sys::CBaseIssue,
	_: *const sys::CUtlVector<*const c_char, sys::CUtlMemory<*const c_char>>,
	_: *const c_int,
	_: *const sys::__BindgenOpaqueArray8<[u8; 40]>,
	_: c_int,
	_: c_int,
	_: c_int,
) -> sys::CBaseIssue_EVoteAction {
	RESULT.get()
}

unsafe extern "C" fn base_ratio(_: *mut sys::CBaseIssue) -> f32 {
	0.6
}

/// As `CRestartGameIssue`'s, which refuses calls while the issue is not
/// enabled.
unsafe extern "C" fn base_request(
	this: *mut sys::CBaseIssue,
	_: c_int,
	_: *const c_char,
	failure: *mut sys::vote_create_failed_t,
	_: *mut c_int,
) -> bool {
	let enabled = unsafe { ((*(*this).vtable_).CBaseIssue_IsEnabled)(this) };

	if !enabled {
		unsafe { failure.write(sys::vote_create_failed_t_VOTE_FAILED_ISSUE_DISABLED) };
	}

	enabled
}

unsafe extern "C" fn base_restart_text(_: *mut sys::CBaseIssue) -> *const c_char {
	c"#TF_vote_restart_game".as_ptr()
}

unsafe extern "C" fn base_set_details(this: *mut sys::CBaseIssue, details: *const c_char) {
	unsafe { write_details(this, CStr::from_ptr(details)) };
}

unsafe extern "C" fn base_team(_: *const sys::CBaseIssue, _: c_int) -> bool {
	true
}

unsafe extern "C" fn base_true(_: *mut sys::CBaseIssue) -> bool {
	true
}

unsafe extern "C" fn base_two(_: *mut sys::CBaseIssue) -> c_int {
	2
}

unsafe extern "C" fn base_with_int(_: *mut sys::CBaseIssue, _: c_int) {}

/// A controller with the mock restart vote, and room for `capacity` issues,
/// leaked as the game's are while issues use them.
fn controller(capacity: usize) -> Controller {
	// SAFETY: Zero is a valid controller as far as these tests read it.
	let raw = NonNull::from(Box::leak(unsafe {
		Box::<sys::CVoteController>::new_zeroed().assume_init()
	}));
	let memory = Box::leak(vec![ptr::null_mut::<sys::CBaseIssue>(); capacity].into_boxed_slice())
		.as_mut_ptr();
	let mut restart = new_issue(c"RestartGame", restart_vtable(), raw.as_ptr()).unwrap();

	// The mock restart vote is not tagged.
	restart.m_szTypeString[ISSUE_TAG_OFFSET..].fill(0);

	let restart = NonNull::from(Box::leak(Box::new(restart)));

	unsafe {
		memory.write(restart.as_ptr());

		let list: *mut IssueList = potential_issues(raw.as_ptr());
		(*list).m_Memory.m_pMemory = memory;
		(*list).m_Memory.m_nAllocationCount = capacity as c_int;
		(*list).m_Size = 1;
		(*list).m_pElements = memory;
	}

	Controller { raw, restart }
}

#[test]
fn cuts_long_text_at_a_character() {
	assert_eq!(cut_text(b"short"), b"short");

	let long = "é".repeat(40);
	let cut = cut_text(long.as_bytes());

	assert_eq!(cut.len(), 62);
	assert!(str::from_utf8(cut).is_ok());
	assert_eq!(cut_text(&[0xFF; 80]).len(), MAX_TEXT_LEN);
}

#[test]
fn custom_issues_keep_the_restart_votes_rtti() {
	let controller = controller(16);
	let (votes, _calls) = install();

	controller.attach().unwrap();

	let issue = controller.issues()[1];
	let rtti = || unsafe { vtable_rtti((*issue).vtable_) };

	assert_eq!(rtti(), RESTART_RTTI);
	drop(votes);
	assert_eq!(state(issue), DEAD_VTABLE);
	assert_eq!(rtti(), RESTART_RTTI);
}

#[test]
fn deleting_an_issue_forgets_it() {
	let controller = controller(16);
	let (votes, _calls) = install();

	controller.attach().unwrap();

	let issue = controller.issues()[1];
	let vtable = unsafe { &*(*issue).vtable_ };

	controller.remove_last();

	unsafe {
		#[cfg(target_os = "windows")]
		(vtable.CBaseIssue_destructor)(issue, 1);
		#[cfg(target_os = "linux")]
		(vtable.CBaseIssue_deleting_destructor)(issue);
	}

	assert_eq!(with_registry(|registry| registry.issues.len()), Some(0));
	assert!(!votes.is_attached());

	// The next level's controller gets a new issue.
	controller.attach().unwrap();
	assert_eq!(controller.issues().len(), 2);
	assert!(votes.is_attached());
}

#[test]
fn detached_issues_refuse_everything_and_revive_in_place() {
	let controller = controller(16);
	let (votes, calls) = install();

	controller.attach().unwrap();

	let issue = controller.issues()[1];

	votes.detach();
	assert_eq!(state(issue), DEAD_VTABLE);
	assert!(!votes.is_attached());

	let vtable = unsafe { &*(*issue).vtable_ };
	let mut failure = sys::vote_create_failed_t_VOTE_FAILED_GENERIC;
	let mut time = 0;

	unsafe {
		assert!(!(vtable.CBaseIssue_IsEnabled)(issue));
		assert!(!(vtable.CBaseIssue_RequestCallVote)(
			issue,
			1,
			c"".as_ptr(),
			&raw mut failure,
			&raw mut time
		));
		assert_eq!(process(issue), VOTE_ACTION_FAIL);
		(vtable.CBaseIssue_ExecuteCommand)(issue);
	}

	assert_eq!(
		failure,
		sys::vote_create_failed_t_VOTE_FAILED_ISSUE_DISABLED
	);
	assert!(calls.borrow().is_empty());

	controller.attach().unwrap();
	assert_eq!(controller.issues(), [controller.restart.as_ptr(), issue]);
	assert_eq!(state(issue), LIVE_VTABLE);
	assert!(votes.is_attached());
}

/// The details string of an issue.
fn details(issue: *mut sys::CBaseIssue) -> CString {
	unsafe { CStr::from_ptr((&raw const (*issue).m_szDetailsString).cast()) }.to_owned()
}

/// Installs the [`TestVote`], returning the arguments it passed with.
fn install() -> (CustomVotes, Rc<RefCell<Vec<CString>>>) {
	let calls = Rc::new(RefCell::new(Vec::new()));
	let vote = TestVote {
		offered: Rc::new(Cell::new(true)),
		passed: Rc::clone(&calls),
	};

	(
		install_with(mock_binding(), vec![Box::new(vote)], restart_vtable()),
		calls,
	)
}

#[test]
fn issues_left_by_an_earlier_load_are_revived() {
	let controller = controller(16);
	let (votes, _calls) = install();

	controller.attach().unwrap();

	let issue = controller.issues()[1];

	drop(votes);
	assert_eq!(state(issue), DEAD_VTABLE);

	let (votes, _calls) = install();

	controller.attach().unwrap();
	assert_eq!(controller.issues().len(), 2);
	assert_eq!(state(issue), LIVE_VTABLE);
	assert!(votes.is_attached());

	// The revived issue is not this load's to free.
	assert_eq!(
		with_registry(|registry| registry.issues[0].owned),
		Some(false)
	);
}

#[test]
fn lockouts_match_arguments_ignoring_case_past_a_second() {
	let lockouts = [(c"veteran".to_owned(), 100.0)];

	assert_eq!(locked_out(&lockouts, c"Veteran", 40.0), Some(60));
	assert_eq!(locked_out(&lockouts, c"normal", 40.0), None);
	assert_eq!(locked_out(&lockouts, c"veteran", 99.5), None);
	assert_eq!(locked_out(&lockouts, c"veteran", 120.0), None);
}

#[test]
fn names_are_ascii_words_that_fit_before_the_tag() {
	assert!(is_valid_name(c"RestartRound"));
	assert!(is_valid_name(c"no_man_left_behind"));
	assert!(!is_valid_name(c""));
	assert!(!is_valid_name(c"Two words"));
	assert!(!is_valid_name(c"quote\""));
	assert!(is_valid_name(
		&CString::new("a".repeat(MAX_NAME_LEN)).unwrap()
	));
	assert!(!is_valid_name(
		&CString::new("a".repeat(MAX_NAME_LEN + 1)).unwrap()
	));

	let votes: Vec<Box<dyn CustomVote>> =
		vec![Box::new(TestVote::default()), Box::new(TestVote::default())];
	assert!(matches!(
		check_names(&votes),
		Err(CustomVoteError::DuplicateName(_))
	));
}

#[test]
fn reloaded_votes_keep_their_places() {
	let controller = controller(16);
	let named = || -> Vec<Box<dyn CustomVote>> {
		[c"Alpha", c"Beta", c"Gamma"]
			.into_iter()
			.map(|name| Box::new(NamedVote(name)) as Box<dyn CustomVote>)
			.collect()
	};

	let votes = install_with(mock_binding(), named(), restart_vtable());

	controller.attach().unwrap();

	let issues = controller.issues();

	votes.detach();
	controller.attach().unwrap();
	assert_eq!(controller.issues(), issues);

	drop(votes);

	let _votes = install_with(mock_binding(), named(), restart_vtable());

	controller.attach().unwrap();
	assert_eq!(controller.issues(), issues);
}

/// Calls the issue's `ProcessResults`.
unsafe fn process(issue: *mut sys::CBaseIssue) -> sys::CBaseIssue_EVoteAction {
	unsafe {
		((*(*issue).vtable_).CBaseIssue_ProcessResults)(
			issue,
			ptr::null(),
			ptr::null(),
			ptr::null(),
			0,
			1,
			1,
		)
	}
}

/// The mock restart vote's vtable, as its issue points to it.
fn restart_vtable() -> *const IssueVtable {
	&raw const RESTART_VTABLE.methods
}

/// The state its tagged vtable gives an issue.
fn state(issue: *mut sys::CBaseIssue) -> u64 {
	unsafe { (*TaggedVtable::from_issue_vtable((*issue).vtable_)).state }
}

#[test]
fn tags_follow_the_name_and_its_nul() {
	let type_string = tagged_type_string(c"Mode").unwrap();

	assert_eq!(
		&type_string[..5],
		&[
			b'M' as c_char,
			b'o' as c_char,
			b'd' as c_char,
			b'e' as c_char,
			0
		]
	);
	assert!(tagged_type_string(&CString::new("a".repeat(MAX_NAME_LEN + 1)).unwrap()).is_none());
	assert_eq!(
		usage_line(c"Difficulty", c"[normal|veteran]").as_c_str(),
		c"callvote Difficulty [normal|veteran]\n"
	);
	assert_eq!(
		usage_line(c"RestartRound", c"").as_c_str(),
		c"callvote RestartRound\n"
	);
}

#[test]
fn the_controller_runs_a_vote_through_the_issue() {
	let controller = controller(16);
	let (votes, calls) = install();

	controller.attach().unwrap();

	let issue = controller.issues()[1];
	let vtable = unsafe { &*(*issue).vtable_ };
	let mut failure = sys::vote_create_failed_t_VOTE_FAILED_GENERIC;
	let mut time = 0;

	unsafe {
		assert!((vtable.CBaseIssue_IsEnabled)(issue));
		assert_eq!(
			CStr::from_ptr((vtable.CBaseIssue_GetTypeStringLocalized)(issue)),
			c"#TF_MvM_Difficulty"
		);
		assert_eq!(
			CStr::from_ptr((vtable.CBaseIssue_GetDisplayString)(issue)),
			c"#TF_playerid_noteam"
		);

		// A choice the vote doesn't have is refused, with the vote's reason.
		assert!(!(vtable.CBaseIssue_RequestCallVote)(
			issue,
			1,
			c"nightmare".as_ptr(),
			&raw mut failure,
			&raw mut time
		));
		assert_eq!(
			failure,
			sys::vote_create_failed_t_VOTE_FAILED_INVALID_ARGUMENT
		);

		// TF2 calls `SetIssueDetails` with what the caller typed once the call
		// is accepted, then shows the details string.
		assert!((vtable.CBaseIssue_RequestCallVote)(
			issue,
			1,
			c"".as_ptr(),
			&raw mut failure,
			&raw mut time
		));
		(vtable.CBaseIssue_SetIssueDetails)(issue, c"".as_ptr());
		assert_eq!(
			details(issue).as_c_str(),
			c"Change the difficulty to Veteran?"
		);

		assert_eq!(process(issue), VOTE_ACTION_PASS);
		assert_eq!(
			details(issue).as_c_str(),
			c"The difficulty will be Veteran from the next round."
		);

		(vtable.CBaseIssue_ExecuteCommand)(issue);
	}

	assert_eq!(*calls.borrow(), [c"veteran".to_owned()]);
	assert!(votes.is_attached());
}

#[test]
fn unoffered_votes_are_hidden_and_refused() {
	let controller = controller(16);
	let offered = Rc::new(Cell::new(false));
	let vote = TestVote {
		offered: Rc::clone(&offered),
		passed: Rc::default(),
	};
	let _votes = install_with(mock_binding(), vec![Box::new(vote)], restart_vtable());

	controller.attach().unwrap();

	let issue = controller.issues()[1];
	let vtable = unsafe { &*(*issue).vtable_ };
	let mut failure = sys::vote_create_failed_t_VOTE_FAILED_GENERIC;
	let mut time = 0;

	unsafe {
		assert!(!(vtable.CBaseIssue_IsEnabled)(issue));
		assert!(!(vtable.CBaseIssue_RequestCallVote)(
			issue,
			1,
			c"".as_ptr(),
			&raw mut failure,
			&raw mut time
		));
	}

	assert_eq!(
		failure,
		sys::vote_create_failed_t_VOTE_FAILED_ISSUE_DISABLED
	);

	offered.set(true);
	assert!(unsafe { (vtable.CBaseIssue_IsEnabled)(issue) });
}
