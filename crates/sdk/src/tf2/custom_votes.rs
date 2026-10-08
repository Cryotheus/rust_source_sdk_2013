//! Votes of a plugin's own, which TF2 lists in its Call Vote menu beside its
//! own and runs through its own poll: the timer, the counting Yes and No, the
//! quorum, the passed and failed panels, and the lockouts after a failure.
//!
//! Each [`CustomVote`] becomes an issue of the level's global vote controller,
//! made with the generated `CBaseIssue` layout and a vtable whose methods are
//! partly Rust's and partly those of TF2's own `CRestartGameIssue`, so that
//! the voters, the quorum, and the Yes and No choices are TF2's. The vtable
//! keeps the restart vote's RTTI before its methods, so that the game's
//! `dynamic_cast`s, such as TF2's check for a kick vote as any vote passes,
//! take a custom issue for a restart vote.
//! [`CustomVotes::install`] takes the votes, and [`CustomVotes::attach`] adds
//! them to the level's controller. The controller deletes its issues as the
//! level ends, so attach the votes as each level activates, and as the plugin
//! loads or unpauses during a level. [`CustomVotes::detach`] switches them to
//! functions of the game alone, which hide them and refuse every call, telling
//! callers that the server disabled them, as the plugin pauses, and dropping
//! the votes detaches them for good, as it unloads.
//!
//! # What players see
//!
//! - **The menu's row** shows [`CustomVote::label`], a localization token
//!   TF2's clients already have, such as `#Vote_RestartGame` ("Restart Game").
//!   A server cannot add strings to its clients, and an unknown token shows a
//!   blank row, which still calls the vote.
//! - **The menu offers no choices** for custom votes, as it does for maps and
//!   players: picking the row runs `callvote <name>` with no argument. Players
//!   can type one, such as `callvote Difficulty veteran`, and `listissues`
//!   lists [`CustomVote::usage`].
//! - **The poll's question and passed line** are [`VoteText`]'s, any text of
//!   at most [`MAX_TEXT_LEN`] bytes, which TF2's `#TF_playerid_noteam` string
//!   (`%s1`) shows as it is.
//! - The menu's message lists the names and labels of every issue the menu
//!   shows in at most 255 bytes (`MAX_USER_MSG_DATA`), TF2's own enabled
//!   issues included, and the game leaves out those past it.
//!
//! # TF2's rules
//!
//! Calls go through TF2's `callvote` as its own votes do, which refuses
//! spectators (`sv_vote_allow_spectators`), calls while a vote is under way,
//! and callers who called one too recently (`sv_vote_creation_timer`), and
//! lists nothing while `sv_allow_votes` is off. A custom vote then refuses
//! calls while TF2 waits for players outside tournament mode, and while
//! [`CustomVote::offered`] does not offer it, before [`CustomVote::call`]
//! decides. A vote that fails locks its argument out for
//! `sv_vote_failure_timer` seconds, as TF2's own do, until the level ends.
//! Votes pass with TF2's quorum (`sv_vote_quorum_ratio`) and more Yes than No,
//! and [`CustomVote::pass`] runs `sv_vote_command_delay` seconds later,
//! outside entity thinks, where it may restart the round or change the level.
//!
//! `metamod_source::MetamodApi::hook_vote_starts` does not see custom votes,
//! only TF2's own.
//!
//! # Room
//!
//! Issues are only added to the controller's list within the room it already
//! has, since growing it would take the game's allocator: TF2's global
//! controller holds its 10 issues in room for 16, leaving room for 6. Issues
//! detached during a level keep their place, and attaching revives them in
//! it, also those a plugin loaded earlier left behind, so reloading a plugin
//! takes no more room.

#[cfg(test)]
#[path = "../tests/tf2/custom_votes.rs"]
mod tests;

use crate::entities::Entity;
use crate::server::InterfaceError;
use crate::tf2::game_rules::GameRules;
use crate::user_messages::messages::{TextDestination, TextMsg};
use crate::user_messages::{self, Recipients};
use crate::{Game, Server, ServerBinding};

use sdk_raw::tf2::custom_votes::{
	ACTIVE_ISSUE_INDEX_OFFSET, DEAD_VTABLE, ISSUE_TAG_OFFSET, IssueMethods, IssueVtable,
	LIVE_VTABLE, MAX_ISSUE_NAME_LEN, MAX_VOTE_DETAILS_LENGTH, ProcessResultsFn, TaggedVtable,
	VOTE_ACTION_FAIL, VOTE_ACTION_PASS, VOTE_INDEX_OFFSET, dead_vtable, is_tagged, live_vtable,
	new_issue, potential_issues, vtable_rtti,
};

use sdk_raw::tf2::voting::{DEDICATED_SERVER, IssueVtables};
use sdk_raw::util::{self, ModuleCache, ModuleKey};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_int};
use std::marker::PhantomData;
use std::mem::offset_of;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::rc::Rc;

/// The longest name a custom vote may have, in bytes.
pub const MAX_NAME_LEN: usize = MAX_ISSUE_NAME_LEN;

/// The longest question or passed line a vote shows, in bytes: what fits in
/// an issue's details string. Longer ones are cut at a character.
pub const MAX_TEXT_LEN: usize = MAX_VOTE_DETAILS_LENGTH - 1;

/// The string an issue's question and passed line are, which TF2's clients
/// have as `%s1`, to show the details string as it is.
const SHOW_DETAILS: &CStr = c"#TF_playerid_noteam";

/// The primary vtable of `CRestartGameIssue` in the game module, once found.
static RESTART_VTABLE: ModuleCache<usize> = ModuleCache::new();

thread_local! {
	/// The votes installed in this module, if any, which [`CustomVotes`]
	/// leaked and frees as it drops. The pointer has no destructor to run as
	/// the thread ends, after the module may have unloaded.
	static REGISTRY: Cell<Option<NonNull<RefCell<Registry>>>> = const { Cell::new(None) };
}

// The tag fits after the longest name and its NUL.
const _: () = assert!(MAX_NAME_LEN < ISSUE_TAG_OFFSET);

// The issues' vtables are read as `IssueVtable`s from their objects.
const _: () = assert!(offset_of!(sys::CBaseIssue, vtable_) == 0);

/// The destructors of the live table. Its methods are called on the main
/// thread, with `this` an issue, which [`lookup`] and [`with_issue`] find
/// among the installed votes' issues; an issue they don't find does nothing.
const DESTRUCTORS: sdk_raw::tf2::custom_votes::IssueDestructors =
	cfg_select! {
		target_os = "windows" => destroy,
		target_os = "linux" => (destroy_in_place, destroy_and_free),
	};

/// A vote of the plugin's own. Its methods are called on the server's main
/// thread, from TF2's vote controller; a panic in one is contained, and
/// counts as the vote not being offered, refusing the call, or doing nothing.
pub trait CustomVote: 'static {
	/// Decides whether a call may start the vote, and with which text. It
	/// should change nothing, since TF2 may still refuse the call afterwards.
	fn call(&self, server: Server<'_>, call: VoteCall<'_>) -> Result<VoteText, VoteRefusal>;

	/// The localization token of the vote's row in the Call Vote menu, which
	/// TF2's clients must already have, such as `#Vote_RestartGame`.
	fn label(&self) -> &'static CStr;

	/// The vote's name, which `callvote` takes, such as `RestartRound`: at
	/// most [`MAX_NAME_LEN`] bytes of ASCII letters, digits and underscores.
	/// TF2 matches it ignoring case, and no other issue of the controller may
	/// have it.
	fn name(&self) -> &'static CStr;

	/// Whether the vote is offered: listed in the menu, and callable. TF2 asks
	/// each time a player opens the menu, and for each call.
	fn offered(&self, server: Server<'_>) -> bool;

	/// Carries out the vote, which passed with `argument`, the one
	/// [`Self::call`] gave.
	fn pass(&self, server: Server<'_>, argument: &CStr);

	/// What `listissues` prints after `callvote` and the vote's name, such as
	/// `[normal|veteran]`, or nothing.
	fn usage(&self) -> &'static CStr {
		c""
	}
}

/// Why custom votes cannot be installed or attached.
#[derive(Debug, thiserror::Error)]
pub enum CustomVoteError {
	/// Custom votes are installed already.
	#[error("custom votes are already installed")]
	AlreadyInstalled,

	/// Two votes have the same name, ignoring case.
	#[error("two custom votes are named {0:?}")]
	DuplicateName(CString),

	/// The game module could not be read.
	#[error("the game module could not be inspected")]
	Image(#[from] std::io::Error),

	/// An interface the votes need is missing.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The game module is not an executable image the vtable search supports.
	#[error("the game module has an unsupported executable image")]
	InvalidImage,

	/// A vote's name is empty, too long, or has other characters than ASCII
	/// letters, digits and underscores.
	#[error("invalid custom vote name {0:?}")]
	InvalidName(CString),

	/// The live vote controllers are not laid out as the generated bindings
	/// describe them.
	#[error("the vote controller's layout differs from the generated one")]
	LayoutMismatch,

	/// Another issue of the global controller has a vote's name, ignoring
	/// case: one of TF2's, or another plugin's.
	#[error("the vote controller already has an issue named {0:?}")]
	NameTaken(CString),

	/// TF2's `CRestartGameIssue` has no unique primary vtable.
	#[error("TF2's restart vote has no unique primary vtable")]
	NoBaseIssue,

	/// The level has no global vote controller, as before its entities are
	/// made.
	#[error("the level has no global vote controller")]
	NoController,

	/// The controller's list of issues has too little room for the votes.
	#[error("the vote controller has room for {free} more issues, not {needed}")]
	NoRoom {
		/// The issues to add.
		needed: usize,

		/// The room the list has.
		free: usize,
	},

	/// TF2's restart vote does not behave as `CBaseIssue` does in the methods
	/// detached votes borrow from it.
	#[error("TF2's restart vote does not behave as expected")]
	UnexpectedBase,

	/// The server does not run TF2.
	#[error("custom votes require Team Fortress 2")]
	WrongGame,
}

impl From<util::Error> for CustomVoteError {
	fn from(error: util::Error) -> Self {
		match error {
			util::Error::InvalidImage => Self::InvalidImage,
			util::Error::Io(error) => Self::Image(error),
		}
	}
}

/// The votes a plugin installed. Only one set can be installed at a time in a
/// module; dropping it detaches them for good.
#[derive(Debug)]
#[must_use = "dropping the votes detaches them"]
pub struct CustomVotes {
	_not_thread_safe: PhantomData<Rc<()>>,
}

impl CustomVotes {
	/// Installs `votes`, to [attach](Self::attach) to each level.
	///
	/// Fails if votes are installed already, if a name is invalid or two
	/// votes share one, or if TF2's restart vote, whose methods the votes
	/// borrow, cannot be found. Finding it reads the game module once.
	pub fn install(
		server: Server<'_>,
		binding: ServerBinding,
		votes: Vec<Box<dyn CustomVote>>,
	) -> Result<Self, CustomVoteError> {
		if server.game() != Game::TeamFortress2 {
			return Err(CustomVoteError::WrongGame);
		}

		if with_registry(|_| ()).is_some() {
			return Err(CustomVoteError::AlreadyInstalled);
		}

		check_names(&votes)?;

		let factory = server.game_server_factory().as_raw();

		// SAFETY: The game server factory is the game module's
		// `CreateInterface`, and the Server's callback scope keeps the module
		// loaded while it is read. Source never unloads the game module while
		// plugins are loaded, as `ModuleCache` assumes.
		let restart_vtable = unsafe {
			let key = ModuleKey::of(factory as usize)?;

			RESTART_VTABLE.get_or_resolve(key, || {
				IssueVtables::load(factory)?
					.find("CRestartGameIssue")
					.map(|vtable| vtable.as_ptr() as usize)
					.ok_or(CustomVoteError::NoBaseIssue)
			})?
		};

		Ok(install_with(
			binding,
			votes,
			restart_vtable as *const IssueVtable,
		))
	}

	/// Adds the votes to the level's global vote controller, or makes those
	/// it already has, detached, theirs again: also those another load of
	/// the plugin left behind. Votes it has already stay as they are.
	///
	/// Changes nothing if it fails, such as when the level has no controller
	/// yet, or too little room for the votes.
	pub fn attach(&self, server: Server<'_>) -> Result<(), CustomVoteError> {
		if server.game() != Game::TeamFortress2 {
			return Err(CustomVoteError::WrongGame);
		}

		let restart_vtable = with_registry(|registry| registry.restart_vtable)
			.ok_or(CustomVoteError::NoController)?;
		let (controller, restart) = global_controller(server, restart_vtable)?;

		// SAFETY: The controller is the level's live global controller, and
		// `restart` its live `CRestartGameIssue`, on the main thread.
		unsafe { attach_to(controller, restart) }
	}

	/// Switches the votes to functions of the game alone, for while the
	/// plugin is paused: they stay in their place, hidden from the menu, refuse
	/// every call, and do nothing as they pass. A vote under way fails as it
	/// ends. [`Self::attach`] makes them the plugin's again.
	pub fn detach(&self) {
		detach_all();
	}

	/// Whether the votes are in the level's controller, and the plugin's.
	pub fn is_attached(&self) -> bool {
		with_registry(|registry| {
			let Some(tables) = registry.tables else {
				return false;
			};
			let live = TaggedVtable::issue_vtable(tables.live);

			!registry.issues.is_empty()
				&& registry.issues.iter().all(|issue| {
					// SAFETY: Registered issues are live, or leaked by detached
					// destructors, so readable.
					live == unsafe { (&raw const (*issue.raw.as_ptr()).vtable_).read() }
				})
		})
		.unwrap_or(false)
	}
}

impl Drop for CustomVotes {
	fn drop(&mut self) {
		detach_all();

		// The votes are dropped after the registry is emptied, in case one of
		// them reaches the registry as it drops.
		if let Some(registry) = REGISTRY.take() {
			// SAFETY: `install_with` leaked the registry, and nothing borrows it
			// past a call into it, none of which is under way as the votes drop.
			drop(unsafe { Box::from_raw(registry.as_ptr()) });
		}
	}
}

/// A custom issue in a controller, and what its vote is up to.
#[derive(Debug)]
struct Issue {
	/// The issue.
	raw: NonNull<sys::CBaseIssue>,

	/// The index of its vote in [`Registry::votes`].
	vote: usize,

	/// Whether this module allocated it, with Rust's global allocator, so
	/// may free it. Issues an earlier load of the plugin left behind are never
	/// freed.
	owned: bool,

	/// What [`CustomVote::call`] accepted, until the controller starts the
	/// vote with it.
	pending: Option<VoteText>,

	/// The vote under way, or that passed.
	current: Option<VoteText>,

	/// The arguments that failed, and until when they are locked out, in game
	/// time.
	lockouts: Vec<(CString, f32)>,
}

impl Issue {
	const fn new(raw: NonNull<sys::CBaseIssue>, vote: usize, owned: bool) -> Self {
		Self {
			raw,
			vote,
			owned,
			pending: None,
			current: None,
			lockouts: Vec::new(),
		}
	}
}

/// The votes installed in a module, and their issues.
struct Registry {
	/// The server the votes were installed in.
	binding: ServerBinding,

	/// The votes.
	votes: Vec<Rc<dyn CustomVote>>,

	/// The primary vtable of `CRestartGameIssue`.
	restart_vtable: *const IssueVtable,

	/// The tables made from it, once attached.
	tables: Option<Tables>,

	/// The issues of the votes that were attached, live or detached.
	issues: Vec<Issue>,
}

/// What [`attach_to`] does for a vote.
#[derive(Debug, Clone, Copy)]
enum Step {
	/// Makes the vote's issue live again.
	Keep(NonNull<sys::CBaseIssue>),

	/// Makes a detached issue of the vote's name, which another load left
	/// behind, the vote's.
	Revive(usize, NonNull<sys::CBaseIssue>),

	/// Adds an issue for the vote.
	New(usize),
}

/// The tables of custom issues, which are leaked: the game may hold issues
/// that use them after the module unloads.
#[derive(Debug, Clone, Copy)]
struct Tables {
	/// The table whose methods are the module's.
	live: *const TaggedVtable,

	/// The table of the game's functions alone.
	dead: *const TaggedVtable,

	/// The restart vote's `ProcessResults`.
	process_results: ProcessResultsFn,
}

/// A call of a vote: who called it, and with what.
#[derive(Debug, Clone, Copy)]
pub struct VoteCall<'a> {
	/// The caller's entity index, or 99 for the server.
	pub caller_entity_index: c_int,

	/// What the caller typed after the vote's name, or nothing, as from the
	/// menu.
	pub argument: &'a CStr,
}

impl VoteCall<'_> {
	/// Whether the server called the vote, rather than a player.
	pub const fn is_server_request(self) -> bool {
		self.caller_entity_index == DEDICATED_SERVER
	}
}

/// Why a call cannot start a vote, which TF2 tells the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoteRefusal {
	/// The vote is on cooldown for this many more seconds
	/// (`VOTE_FAILED_ON_COOLDOWN`).
	Cooldown(u16),

	/// The server disabled the vote (`VOTE_FAILED_ISSUE_DISABLED`).
	Disabled,

	/// TF2's message that the vote failed, with no reason
	/// (`VOTE_FAILED_GENERIC`).
	Generic,

	/// The vote told the caller why itself, and TF2 shows nothing
	/// (`VOTE_FAILED_REQUEST_HANDLED_BY_ISSUE`).
	Handled,

	/// What the caller typed is not a choice of the vote
	/// (`VOTE_FAILED_INVALID_ARGUMENT`).
	InvalidArgument,

	/// What the vote would change is already so
	/// (`VOTE_FAILED_MODIFICATION_ALREADY_ACTIVE`).
	///
	/// TF2's clients show its Vote Failed panel with no reason, only
	/// `%FailedReason%`, as seen on 2026-10-08, so a vote may rather say why
	/// itself, and refuse with [`VoteRefusal::Handled`].
	AlreadyActive,

	/// The game is waiting for players (`VOTE_FAILED_WAITINGFORPLAYERS`).
	WaitingForPlayers,
}

impl VoteRefusal {
	/// The failure code and time TF2 sends the caller.
	const fn to_raw(self) -> (sys::vote_create_failed_t, c_int) {
		match self {
			Self::AlreadyActive => (
				sys::vote_create_failed_t_VOTE_FAILED_MODIFICATION_ALREADY_ACTIVE,
				0,
			),

			Self::Cooldown(seconds) => (
				sys::vote_create_failed_t_VOTE_FAILED_ON_COOLDOWN,
				seconds as c_int,
			),

			Self::Disabled => (sys::vote_create_failed_t_VOTE_FAILED_ISSUE_DISABLED, 0),
			Self::Generic => (sys::vote_create_failed_t_VOTE_FAILED_GENERIC, 0),

			Self::Handled => (
				sys::vote_create_failed_t_VOTE_FAILED_REQUEST_HANDLED_BY_ISSUE,
				0,
			),

			Self::InvalidArgument => (sys::vote_create_failed_t_VOTE_FAILED_INVALID_ARGUMENT, 0),
			Self::WaitingForPlayers => (sys::vote_create_failed_t_VOTE_FAILED_WAITINGFORPLAYERS, 0),
		}
	}
}

/// What a vote a call starts is about, and says.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VoteText {
	/// The vote's choice, which [`CustomVote::pass`] gets, and which a failed
	/// vote locks out: what the caller typed, or the choice the vote made for
	/// a call without one, such as `veteran`.
	pub argument: CString,

	/// The poll's question, such as "Change the difficulty to Veteran?".
	pub question: CString,

	/// The line shown once the vote passed, such as "The difficulty will be
	/// Veteran from the next round."
	pub passed: CString,
}

/// Adds the votes to `controller`, as [`CustomVotes::attach`] does.
///
/// # Safety
///
/// `controller` must point to the level's live global vote controller, and
/// `restart` to its live `CRestartGameIssue`, on the main thread.
unsafe fn attach_to(
	controller: NonNull<sys::CVoteController>,
	restart: NonNull<sys::CBaseIssue>,
) -> Result<(), CustomVoteError> {
	let controller = controller.as_ptr();

	// SAFETY: As the caller promises; fields are read without forming
	// references to the game's objects.
	let (list, base) = unsafe {
		(
			potential_issues(controller),
			(&raw const (*restart.as_ptr()).vtable_).read(),
		)
	};

	// SAFETY: As above.
	let (memory, size, capacity) = unsafe {
		(
			(*list).m_Memory.m_pMemory,
			(*list).m_Size,
			(*list).m_Memory.m_nAllocationCount,
		)
	};

	let (Ok(size), Ok(capacity)) = (usize::try_from(size), usize::try_from(capacity)) else {
		return Err(CustomVoteError::LayoutMismatch);
	};

	if size > capacity || (memory.is_null() && capacity > 0) {
		return Err(CustomVoteError::LayoutMismatch);
	}

	let tables = match with_registry(|registry| registry.tables) {
		None => return Err(CustomVoteError::NoController),
		Some(Some(tables)) => tables,

		Some(None) => {
			// SAFETY: `restart` is TF2's restart vote, whose vtable is the game's.
			let tables = unsafe { make_tables(restart, base) }?;

			with_registry_mut(|registry| registry.tables = Some(tables));
			tables
		}
	};

	let issues: Vec<*mut sys::CBaseIssue> = if size == 0 {
		Vec::new()
	} else {
		// SAFETY: The list's first `size` elements are its issues.
		unsafe { std::slice::from_raw_parts(memory, size) }.to_vec()
	};

	let plan = with_registry_mut(|registry| {
		// Issues no longer in the controller were deleted with an earlier one,
		// while detached, which leaked them.
		registry
			.issues
			.retain(|issue| issues.contains(&issue.raw.as_ptr()));

		plan(registry, &issues, tables)
	})
	.ok_or(CustomVoteError::NoController)??;

	let needed = plan
		.iter()
		.filter(|step| matches!(step, Step::New(_)))
		.count();
	let free = capacity - size;

	if needed > free {
		return Err(CustomVoteError::NoRoom { needed, free });
	}

	let live = TaggedVtable::issue_vtable(tables.live);
	let mut size = size;

	with_registry_mut(|registry| {
		for step in plan {
			match step {
				Step::Keep(raw) => {
					// SAFETY: The issue is in the controller, so live.
					unsafe { (&raw mut (*raw.as_ptr()).vtable_).write(live) };
				}

				Step::Revive(vote, raw) => {
					// SAFETY: As above.
					unsafe { (&raw mut (*raw.as_ptr()).vtable_).write(live) };
					registry.issues.push(Issue::new(raw, vote, false));
				}

				Step::New(vote) => {
					let issue = new_issue(registry.votes[vote].name(), live, controller)
						.expect("names were checked as installed");
					let raw = NonNull::from(Box::leak(Box::new(issue)));

					// SAFETY: There is room for the issue after the list's
					// last, as checked above.
					unsafe { memory.add(size).write(raw.as_ptr()) };
					size += 1;
					registry.issues.push(Issue::new(raw, vote, true));
				}
			}
		}
	});

	// SAFETY: The list now holds `size` issues, in its own memory.
	unsafe {
		(&raw mut (*list).m_Size).write(size as c_int);
		(&raw mut (*list).m_pElements).write(memory);
	}

	Ok(())
}

/// Checks that each vote's name is valid, and that no two share one.
fn check_names(votes: &[Box<dyn CustomVote>]) -> Result<(), CustomVoteError> {
	for (index, vote) in votes.iter().enumerate() {
		let name = vote.name();

		if !is_valid_name(name) {
			return Err(CustomVoteError::InvalidName(name.to_owned()));
		}

		if votes[..index]
			.iter()
			.any(|other| names_match(other.name(), name))
		{
			return Err(CustomVoteError::DuplicateName(name.to_owned()));
		}
	}

	Ok(())
}

/// Calls `f`, containing a panic in it as `None`.
fn contain<R>(f: impl FnOnce() -> R) -> Option<R> {
	catch_unwind(AssertUnwindSafe(f)).ok()
}

/// `text` cut to [`MAX_TEXT_LEN`] bytes, at a character if it is UTF-8.
fn cut_text(text: &[u8]) -> &[u8] {
	if text.len() <= MAX_TEXT_LEN {
		return text;
	}

	let end = match str::from_utf8(text) {
		Ok(text) => text.floor_char_boundary(MAX_TEXT_LEN),
		Err(_) => MAX_TEXT_LEN,
	};

	&text[..end]
}

/// MSVC's deleting destructor: forgets the issue, then frees it if asked to
/// and this module allocated it.
#[cfg(target_os = "windows")]
unsafe extern "C" fn destroy(
	this: *mut sys::CBaseIssue,
	flags: std::ffi::c_uint,
) -> *mut std::ffi::c_void {
	// SAFETY: The controller deletes its live issues.
	unsafe { forget(this, flags & 1 != 0) };
	this.cast()
}

/// The Itanium ABI's deleting destructor: forgets the issue, then frees it if
/// this module allocated it.
#[cfg(target_os = "linux")]
unsafe extern "C" fn destroy_and_free(this: *mut sys::CBaseIssue) {
	// SAFETY: The controller deletes its live issues.
	unsafe { forget(this, true) };
}

/// The Itanium ABI's complete destructor: forgets the issue.
#[cfg(target_os = "linux")]
unsafe extern "C" fn destroy_in_place(this: *mut sys::CBaseIssue) {
	// SAFETY: The controller deletes its live issues.
	unsafe { forget(this, false) };
}

/// Detaches every issue of the installed votes.
fn detach_all() {
	with_registry(|registry| {
		let Some(tables) = registry.tables else {
			return;
		};

		let dead = TaggedVtable::issue_vtable(tables.dead);

		for issue in &registry.issues {
			// SAFETY: Registered issues are live, or leaked by detached
			// destructors, so writable.
			unsafe { (&raw mut (*issue.raw.as_ptr()).vtable_).write(dead) };
		}
	});
}

/// `ExecuteCommand`: passes the vote.
unsafe extern "C" fn execute_command(this: *mut sys::CBaseIssue) {
	let Some((binding, vote)) = lookup(this) else {
		return;
	};
	let Some(Some(text)) = with_issue(this, |issue| issue.current.take()) else {
		return;
	};
	let scope = ();

	// SAFETY: The game calls this on the main thread, from the binding's
	// server.
	let server = unsafe { binding.server(&scope) };

	contain(|| vote.pass(server, &text.argument));
}

/// Forgets the issue `this`, freeing it if `free` and this module allocated
/// it.
///
/// # Safety
///
/// `this` must be a live issue being destroyed, which nothing uses after.
unsafe fn forget(this: *mut sys::CBaseIssue, free: bool) {
	let owned = with_registry_mut(|registry| {
		let index = registry
			.issues
			.iter()
			.position(|issue| issue.raw.as_ptr() == this)?;

		Some(registry.issues.swap_remove(index).owned)
	})
	.flatten()
	.unwrap_or(false);

	if free && owned {
		// SAFETY: This module allocated the issue as a `Box`, and the game is
		// done with it.
		drop(unsafe { Box::from_raw(this) });
	}
}

/// `GetDisplayString` and `GetVotePassedString`: the string that shows the
/// details string as it is.
unsafe extern "C" fn get_display_string(_: *mut sys::CBaseIssue) -> *const c_char {
	SHOW_DETAILS.as_ptr()
}

/// `GetTypeStringLocalized`: the vote's label.
unsafe extern "C" fn get_type_string_localized(this: *mut sys::CBaseIssue) -> *const c_char {
	lookup(this).map_or(c"".as_ptr(), |(_, vote)| {
		contain(|| vote.label()).unwrap_or_default().as_ptr()
	})
}

/// `GetVotePassedString`, as [`get_display_string`].
unsafe extern "C" fn get_vote_passed_string(this: *mut sys::CBaseIssue) -> *const c_char {
	// SAFETY: The same method.
	unsafe { get_display_string(this) }
}

/// Finds the level's global vote controller, and its `CRestartGameIssue`,
/// whose vtable is `restart_vtable`, checking the controllers' layout.
fn global_controller(
	server: Server<'_>,
	restart_vtable: *const IssueVtable,
) -> Result<(NonNull<sys::CVoteController>, NonNull<sys::CBaseIssue>), CustomVoteError> {
	let tools = server.server_tools()?;
	let dll = server.server_game_dll()?;

	for entity in tools.entities_by_class(c"vote_controller") {
		let class = entity
			.server_class()
			.ok_or(CustomVoteError::LayoutMismatch)?;

		for (name, offset) in [
			(c"m_iActiveIssueIndex", ACTIVE_ISSUE_INDEX_OFFSET),
			(c"m_nVoteIdx", VOTE_INDEX_OFFSET),
		] {
			let prop = dll
				.net_prop(class, name)
				.map_err(|_| CustomVoteError::LayoutMismatch)?;

			if prop.offset() != offset {
				return Err(CustomVoteError::LayoutMismatch);
			}
		}

		let controller = NonNull::new(entity.as_ptr().cast::<sys::CVoteController>())
			.ok_or(CustomVoteError::LayoutMismatch)?;

		// SAFETY: The entity is a live vote controller, laid out as the
		// generated binding, as its networked variables show.
		if let Some(restart) = unsafe { restart_issue(controller, restart_vtable) } {
			return Ok((controller, restart));
		}
	}

	Err(CustomVoteError::NoController)
}

/// The installed votes, without their issues, as [`CustomVotes::install`]
/// installs them once it found `restart_vtable`.
fn install_with(
	binding: ServerBinding,
	votes: Vec<Box<dyn CustomVote>>,
	restart_vtable: *const IssueVtable,
) -> CustomVotes {
	let votes = votes.into_iter().map(Rc::from).collect();

	let registry = Box::leak(Box::new(RefCell::new(Registry {
		binding,
		votes,
		restart_vtable,
		tables: None,
		issues: Vec::new(),
	})));

	REGISTRY.set(Some(NonNull::from(registry)));

	CustomVotes {
		_not_thread_safe: PhantomData,
	}
}

/// `IsEnabled`: whether the vote is offered.
unsafe extern "C" fn is_enabled(this: *mut sys::CBaseIssue) -> bool {
	let Some((binding, vote)) = lookup(this) else {
		return false;
	};
	let scope = ();

	// SAFETY: As for `execute_command`.
	let server = unsafe { binding.server(&scope) };

	contain(|| vote.offered(server)).unwrap_or(false)
}

/// Whether a vote may be named `name`: [`MAX_NAME_LEN`] bytes at most of ASCII
/// letters, digits and underscores.
fn is_valid_name(name: &CStr) -> bool {
	let name = name.to_bytes();

	!name.is_empty()
		&& name.len() <= MAX_NAME_LEN
		&& name
			.iter()
			.all(|&byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Whether TF2 waits for players outside tournament mode, when it refuses to
/// start votes.
fn is_waiting_for_players(server: Server<'_>) -> bool {
	let waiting = GameRules::get(server)
		.and_then(GameRules::is_waiting_for_players)
		.unwrap_or(false);
	let tournament = server
		.cvar()
		.ok()
		.and_then(|cvar| cvar.find_var(c"mp_tournament"))
		.is_some_and(|variable| variable.int() != 0);

	waiting && !tournament
}

/// `ListIssueDetails`: prints how to call the vote, if it is offered.
unsafe extern "C" fn list_issue_details(this: *mut sys::CBaseIssue, player: *mut sys::CBasePlayer) {
	let Some((binding, vote)) = lookup(this) else {
		return;
	};
	let Some(player) = NonNull::new(player.cast::<sys::CBaseEntity>()) else {
		return;
	};
	let scope = ();

	// SAFETY: As for `execute_command`.
	let server = unsafe { binding.server(&scope) };

	if !contain(|| vote.offered(server)).unwrap_or(false) {
		return;
	}

	// SAFETY: The game lists issues for the live player who asked, whose
	// `CBasePlayer` starts with its `CBaseEntity`.
	let Some(edict) = unsafe { Entity::from_live(server, player) }.edict() else {
		return;
	};
	let Some(line) = contain(|| usage_line(vote.name(), vote.usage())) else {
		return;
	};

	let message = TextMsg {
		destination: TextDestination::Console,
		message: &line,
		arguments: [c""; 4],
	};

	let _ = user_messages::send(server, &Recipients::player(edict).reliable(), &message);
}

/// How long, in whole seconds, `argument` stays locked out at `now`, by the
/// lockouts of an issue, if it is: by more than a second, as TF2 counts.
fn locked_out(lockouts: &[(CString, f32)], argument: &CStr, now: f32) -> Option<u16> {
	lockouts.iter().find_map(|(locked, until)| {
		let remaining = (until - now) as c_int;

		(names_match(locked, argument) && remaining > 1)
			.then(|| u16::try_from(remaining).unwrap_or(u16::MAX))
	})
}

/// The issue `this` is, its vote, and the server, if it is one of the
/// installed votes'.
fn lookup(this: *mut sys::CBaseIssue) -> Option<(ServerBinding, Rc<dyn CustomVote>)> {
	with_registry(|registry| {
		let issue = registry
			.issues
			.iter()
			.find(|issue| issue.raw.as_ptr() == this)?;

		Some((registry.binding, Rc::clone(&registry.votes[issue.vote])))
	})
	.flatten()
}

/// Makes the live and dead tables from the restart vote's `base`, after
/// checking that the methods the dead table borrows behave as `CBaseIssue`'s.
/// Both keep its RTTI, so that the game takes custom issues for restart votes,
/// which have no fields of their own.
///
/// # Safety
///
/// `restart` must point to TF2's live `CRestartGameIssue`, whose vtable is
/// `base`, in the game module.
unsafe fn make_tables(
	restart: NonNull<sys::CBaseIssue>,
	base: *const IssueVtable,
) -> Result<Tables, CustomVoteError> {
	let restart = restart.as_ptr();

	// SAFETY: As the caller promises, `base` is the vtable the compiler made
	// for `CRestartGameIssue`.
	let (rtti, base) = unsafe { (vtable_rtti(base), &*base) };

	// SAFETY: The restart vote's own methods, which read nothing but its
	// fields, called on it.
	let (options, restricted, label) = unsafe {
		(
			(base.CBaseIssue_GetNumberVoteOptions)(restart),
			(base.CBaseIssue_IsTeamRestrictedVote)(restart),
			util::cstr::borrow_cstr((base.CBaseIssue_GetTypeStringLocalized)(restart)),
		)
	};

	if options != 2 || restricted || label.is_none_or(|label| !label.is_empty()) {
		return Err(CustomVoteError::UnexpectedBase);
	}

	let methods = IssueMethods {
		destructors: DESTRUCTORS,
		get_type_string_localized,
		set_issue_details,
		on_vote_failed,
		is_enabled,
		request_call_vote,
		get_display_string,
		execute_command,
		list_issue_details,
		get_vote_passed_string,
		process_results,
	};

	let live = Box::leak(Box::new(TaggedVtable {
		rtti,
		methods: live_vtable(base, &methods),
		state: LIVE_VTABLE,
	}));

	// SAFETY: `CRestartGameIssue` overrides none of the methods the dead table
	// borrows, as checked above, its `RequestCallVote` refuses calls once
	// `IsEnabled` returns false, and it is in the game module, which outlives
	// every issue.
	let dead = Box::leak(Box::new(TaggedVtable {
		rtti,
		methods: unsafe { dead_vtable(base) },
		state: DEAD_VTABLE,
	}));

	Ok(Tables {
		live,
		dead,
		process_results: base.CBaseIssue_ProcessResults,
	})
}

/// Whether two names are the same, ignoring ASCII case, as TF2 matches them.
fn names_match(a: &CStr, b: &CStr) -> bool {
	a.to_bytes().eq_ignore_ascii_case(b.to_bytes())
}

/// The game time, if the engine's globals can be read.
fn now(server: Server<'_>) -> Option<f32> {
	Some(
		server
			.player_info_manager()
			.ok()?
			.global_vars()?
			.current_time(),
	)
}

/// `OnVoteFailed`: locks the vote's argument out, unless the server called
/// it.
unsafe extern "C" fn on_vote_failed(this: *mut sys::CBaseIssue, caller: c_int) {
	let Some((binding, _)) = lookup(this) else {
		return;
	};
	let scope = ();

	// SAFETY: As for `execute_command`.
	let server = unsafe { binding.server(&scope) };

	let lockout = server
		.cvar()
		.ok()
		.and_then(|cvar| cvar.find_var(c"sv_vote_failure_timer"))
		.map(|variable| variable.float());

	let now = now(server);

	with_issue(this, |issue| {
		let Some(text) = issue.current.take() else {
			return;
		};
		let (Some(now), Some(lockout)) = (now, lockout) else {
			return;
		};

		if caller == DEDICATED_SERVER {
			return;
		}

		issue
			.lockouts
			.retain(|(argument, _)| !names_match(argument, &text.argument));
		issue.lockouts.push((text.argument, now + lockout));
	});
}

/// Decides how [`attach_to`] handles each vote, from the issues of the
/// controller.
fn plan(
	registry: &Registry,
	issues: &[*mut sys::CBaseIssue],
	tables: Tables,
) -> Result<Vec<Step>, CustomVoteError> {
	let mut plan = Vec::with_capacity(registry.votes.len());

	for (index, vote) in registry.votes.iter().enumerate() {
		if let Some(issue) = registry.issues.iter().find(|issue| issue.vote == index) {
			plan.push(Step::Keep(issue.raw));
			continue;
		}

		let named = issues
			.iter()
			.copied()
			.filter_map(NonNull::new)
			.find(|&issue| {
				// SAFETY: The controller's issues are live.
				unsafe { type_string(issue) }.is_some_and(|name| names_match(&name, vote.name()))
			});

		let Some(issue) = named else {
			plan.push(Step::New(index));
			continue;
		};

		// SAFETY: As above. A tagged issue's vtable is the methods of a
		// `TaggedVtable`, which an SDK leaked or keeps live.
		let state = unsafe {
			is_tagged(issue.as_ptr()).then(|| {
				let vtable =
					TaggedVtable::from_issue_vtable((&raw const (*issue.as_ptr()).vtable_).read());

				(vtable, (&raw const (*vtable).state).read())
			})
		};

		match state {
			Some((vtable, _)) if vtable == tables.live => plan.push(Step::Keep(issue)),
			Some((_, DEAD_VTABLE)) => plan.push(Step::Revive(index, issue)),
			_ => return Err(CustomVoteError::NameTaken(vote.name().to_owned())),
		}
	}

	Ok(plan)
}

/// `ProcessResults`: the restart vote's, which writes the passed line to the
/// details string of a vote that passed.
unsafe extern "C" fn process_results(
	this: *mut sys::CBaseIssue,
	options: *const sys::CUtlVector<*const c_char, sys::CUtlMemory<*const c_char>>,
	counts: *const c_int,
	votes: *const sys::__BindgenOpaqueArray8<[u8; 40]>,
	highest: c_int,
	total: c_int,
	potential: c_int,
) -> sys::CBaseIssue_EVoteAction {
	let Some(Some(tables)) = with_registry(|registry| registry.tables) else {
		return VOTE_ACTION_FAIL;
	};

	// SAFETY: The game's own `ProcessResults`, with the arguments the game
	// passed.
	let action = unsafe {
		(tables.process_results)(this, options, counts, votes, highest, total, potential)
	};

	if action == VOTE_ACTION_PASS {
		let passed = with_issue(this, |issue| {
			issue.current.as_ref().map(|text| text.passed.clone())
		})
		.flatten();

		if let Some(passed) = passed {
			// SAFETY: The issue is live.
			unsafe { write_details(this, &passed) };
		}
	}

	action
}

/// `RequestCallVote`: whether a call may start the vote.
unsafe extern "C" fn request_call_vote(
	this: *mut sys::CBaseIssue,
	caller: c_int,
	details: *const c_char,
	failure: *mut sys::vote_create_failed_t,
	time: *mut c_int,
) -> bool {
	let refused = |refusal: VoteRefusal| {
		let (code, seconds) = refusal.to_raw();

		// SAFETY: The game passes its own output references, or null.
		unsafe {
			if !failure.is_null() {
				failure.write(code);
			}

			if !time.is_null() {
				time.write(seconds);
			}
		}

		false
	};

	let Some((binding, vote)) = lookup(this) else {
		return refused(VoteRefusal::Generic);
	};

	if caller == -1 {
		return refused(VoteRefusal::Generic);
	}

	let scope = ();

	// SAFETY: As for `execute_command`.
	let server = unsafe { binding.server(&scope) };

	// SAFETY: The game passes the call's argument, which lasts for the call.
	let argument = unsafe { util::cstr::borrow_cstr(details) }.unwrap_or_default();

	let call = VoteCall {
		caller_entity_index: caller,
		argument,
	};

	if !call.is_server_request() && is_waiting_for_players(server) {
		return refused(VoteRefusal::WaitingForPlayers);
	}

	if !contain(|| vote.offered(server)).unwrap_or(false) {
		return refused(VoteRefusal::Disabled);
	}

	let text = match contain(|| vote.call(server, call)) {
		Some(Ok(text)) => text,
		Some(Err(refusal)) => return refused(refusal),
		None => return refused(VoteRefusal::Generic),
	};

	let now = now(server);

	let accepted = with_issue(this, |issue| {
		if !call.is_server_request()
			&& let Some(remaining) =
				now.and_then(|now| locked_out(&issue.lockouts, &text.argument, now))
		{
			return Err(VoteRefusal::Cooldown(remaining));
		}

		issue.pending = Some(text);
		Ok(())
	});

	match accepted {
		Some(Ok(())) => true,
		Some(Err(refusal)) => refused(refusal),
		None => refused(VoteRefusal::Generic),
	}
}

/// The issue of `controller` whose vtable is `restart_vtable`, and whose
/// controller and type string are those `CRestartGameIssue`'s constructor
/// gave it, if there is one.
///
/// # Safety
///
/// `controller` must point to a live vote controller.
unsafe fn restart_issue(
	controller: NonNull<sys::CVoteController>,
	restart_vtable: *const IssueVtable,
) -> Option<NonNull<sys::CBaseIssue>> {
	let controller = controller.as_ptr();

	// SAFETY: As the caller promises; fields are read without forming
	// references.
	let (memory, size, capacity) = unsafe {
		let list = potential_issues(controller);

		(
			(&raw const (*list).m_Memory.m_pMemory).read(),
			(&raw const (*list).m_Size).read(),
			(&raw const (*list).m_Memory.m_nAllocationCount).read(),
		)
	};

	if memory.is_null() || size <= 0 || size > capacity {
		return None;
	}

	(0..size as usize).find_map(|index| {
		// SAFETY: The list's first `size` elements are live issues.
		let issue = NonNull::new(unsafe { memory.add(index).read() })?;

		// SAFETY: As above.
		let (vtable, owner) = unsafe {
			(
				(&raw const (*issue.as_ptr()).vtable_).read(),
				(&raw const (*issue.as_ptr()).m_pVoteController).read(),
			)
		};

		// SAFETY: As above.
		let named =
			unsafe { type_string(issue) }.is_some_and(|name| name.as_c_str() == c"RestartGame");

		(vtable == restart_vtable && owner == controller && named).then_some(issue)
	})
}

/// `SetIssueDetails`: starts the vote [`request_call_vote`] accepted, writing
/// its question to the details string, or copies `details` there as
/// `CBaseIssue`'s does for anything else.
unsafe extern "C" fn set_issue_details(this: *mut sys::CBaseIssue, details: *const c_char) {
	let question = with_issue(this, |issue| {
		issue.current = issue.pending.take();
		issue.current.as_ref().map(|text| text.question.clone())
	})
	.flatten();

	// SAFETY: The game passes a string that lasts for the call.
	let text = question
		.or_else(|| unsafe { util::cstr::borrow_cstr(details) }.map(CStr::to_owned))
		.unwrap_or_default();

	// SAFETY: The issue is live.
	unsafe { write_details(this, &text) };
}

/// The type string of `issue`, up to its NUL.
///
/// # Safety
///
/// `issue` must point to a live issue.
unsafe fn type_string(issue: NonNull<sys::CBaseIssue>) -> Option<CString> {
	// SAFETY: As the caller promises.
	let bytes = unsafe { (&raw const (*issue.as_ptr()).m_szTypeString).read() };
	let bytes = bytes.map(|byte| byte as u8);

	CStr::from_bytes_until_nul(&bytes).ok().map(CStr::to_owned)
}

/// The line `listissues` prints for a vote.
fn usage_line(name: &CStr, usage: &CStr) -> CString {
	let mut line = b"callvote ".to_vec();

	line.extend_from_slice(name.to_bytes());

	if !usage.is_empty() {
		line.push(b' ');
		line.extend_from_slice(usage.to_bytes());
	}

	line.push(b'\n');

	CString::new(line).expect("C strings hold no NUL")
}

/// Calls `f` with the registered issue `this` is, to change it, if it is one.
fn with_issue<R>(this: *mut sys::CBaseIssue, f: impl FnOnce(&mut Issue) -> R) -> Option<R> {
	with_registry_mut(|registry| {
		registry
			.issues
			.iter_mut()
			.find(|issue| issue.raw.as_ptr() == this)
			.map(f)
	})
	.flatten()
}

/// Calls `f` with the installed votes, or returns `None` if there are none, or
/// they are in use.
fn with_registry<R>(f: impl FnOnce(&Registry) -> R) -> Option<R> {
	// SAFETY: An installed registry lives until `CustomVotes` drops, which
	// clears `REGISTRY` first.
	let registry = unsafe { REGISTRY.get()?.as_ref() };

	Some(f(&*registry.try_borrow().ok()?))
}

/// Calls `f` with the installed votes, to change them, or returns `None` if
/// there are none, or they are in use.
fn with_registry_mut<R>(f: impl FnOnce(&mut Registry) -> R) -> Option<R> {
	// SAFETY: As for `with_registry`.
	let registry = unsafe { REGISTRY.get()?.as_ref() };

	Some(f(&mut *registry.try_borrow_mut().ok()?))
}

/// Writes `text` to the issue's details string, cut to [`MAX_TEXT_LEN`] bytes
/// at a character.
///
/// # Safety
///
/// `issue` must point to a live issue.
unsafe fn write_details(issue: *mut sys::CBaseIssue, text: &CStr) {
	let mut details = [0; MAX_VOTE_DETAILS_LENGTH];

	for (to, &from) in details.iter_mut().zip(cut_text(text.to_bytes())) {
		*to = from as c_char;
	}

	// SAFETY: As the caller promises.
	unsafe { (&raw mut (*issue).m_szDetailsString).write(details) };
}
