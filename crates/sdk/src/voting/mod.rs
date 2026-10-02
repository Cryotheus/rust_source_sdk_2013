//! TF2's built-in votes, including server-initiated and team-specific votes.
//!
//! Listen for [`VoteEvent::EVENTS`] with a pinned `GameEventListener`, then
//! decode events with [`VoteEvent::from_event`]. `vote_options` announces an
//! actual newly created vote; `vote_cast` reports an accepted ballot, including
//! the caller's automatic yes vote. Failed requests and rejected ballots do
//! not produce these notifications. The vote ID correlates simultaneous RED,
//! BLU, and global votes. Player identifiers here are **entity indices**, not
//! the user IDs used by most other game events.
//!
//! To veto creation, use `metamod_source::MetamodApi::hook_vote_starts` (its
//! `sdk` feature). It intercepts `CBaseIssue::RequestCallVote` before TF2
//! changes vote state, covering player, server, and coordinator requests.
//! Blocking `vote_options` or `VoteStart` messages would only hide a vote.
//!
//! Behavior follows Valve's `game/server/vote_controller.cpp`, specifically
//! `CVoteController::CreateVote` and `TryCastVote`.

mod vtables;

use crate::interfaces::game_event::GameEvent;
use crate::voting::vtables::VotingOvft;
use crate::{Game, Server};
use std::ffi::{CStr, CString, c_int, c_void};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The caller index TF2 uses for server-initiated and automatic votes.
const DEDICATED_SERVER: c_int = 99;

/// The most options a TF2 vote can offer.
const MAX_VOTE_OPTIONS: u8 = 5;

/// `CBaseIssue::RequestCallVote`'s slot, generated from Valve's declaration.
#[doc(alias = "RequestCallVote")]
pub const REQUEST_CALL_VOTE_SLOT: usize =
	std::mem::offset_of!(sys::CBaseIssue__bindgen_vtable, CBaseIssue_RequestCallVote)
		/ size_of::<usize>();

/// An accepted choice, indexed from zero (yes is 0, no is 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VoteChoice(u8);

impl VoteChoice {
	/// The second option, "No" in a yes/no vote.
	pub const NO: Self = Self(1);

	/// The first option, "Yes" in a yes/no vote.
	pub const YES: Self = Self(0);

	/// An option index, or `None` past TF2's five options.
	pub const fn new(index: u8) -> Option<Self> {
		if index < MAX_VOTE_OPTIONS {
			Some(Self(index))
		} else {
			None
		}
	}

	/// The option's zero-based index into [`VoteEvent::Started`]'s options.
	pub const fn index(self) -> usize {
		self.0 as usize
	}
}

/// Whether TF2 may evaluate its own issue-specific eligibility checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoteDecision {
	/// Let TF2 continue with its own checks, which can still refuse the vote.
	Allow,

	/// Refuse the vote with TF2's generic vote-creation failure response.
	Block,
}

/// Owned event data, safe to retain after the engine callback ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoteEvent {
	/// A vote was created. Option labels are in `VoteChoice` order.
	#[doc(alias = "vote_options")]
	Started {
		/// The vote's ID (`voteidx`).
		vote_id: c_int,

		/// The two to five option labels (`option1` onward).
		options: Vec<CString>,
	},

	/// TF2 accepted this player's choice for the identified vote.
	#[doc(alias = "vote_cast")]
	Cast {
		/// The vote's ID (`voteidx`).
		vote_id: c_int,

		/// The voter's entity index (`entityid`), not a user ID.
		entity_index: u8,

		/// The accepted option (`vote_option`).
		choice: VoteChoice,

		/// 0 for a global vote, or the allowed team's number.
		team: c_int,
	},
}

impl VoteEvent {
	/// The game events that [`Self::from_event`] decodes.
	pub const EVENTS: [&'static CStr; 2] = [c"vote_options", c"vote_cast"];

	/// Returns `Ok(None)` for unrelated events. Missing or out-of-range data
	/// is rejected instead of interpreting a missing key as a zero value.
	pub fn from_event(event: GameEvent<'_>) -> Result<Option<Self>, VoteEventError> {
		decode(
			event.name(),
			|key| event.get_int(key),
			|key| event.get_string(key),
		)
	}
}

/// A vote event's missing or invalid field, by key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("TF2 vote event has a missing or invalid `{0}` field")]
pub struct VoteEventError(pub &'static str);

/// Why the built-in vote gates cannot all be hooked safely.
#[derive(Debug, thiserror::Error)]
pub enum VoteHookTargetError {
	/// The server does not run Team Fortress 2.
	#[error("built-in voting hooks require Team Fortress 2")]
	WrongGame,

	/// The game module could not be read.
	#[error("the game module could not be inspected")]
	Image(#[from] std::io::Error),

	/// The game module is not an executable image the vtable search supports.
	#[error("the game module has an unsupported executable image")]
	InvalidImage,

	/// The issue's class has no unique primary vtable in the game module.
	#[error("no unique primary vtable found for TF2 vote issue {0:?}")]
	UnsupportedIssue(VoteIssue),
}

/// TF2's concrete built-in vote issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoteIssue {
	/// Restart the current map.
	#[doc(alias = "CRestartGameIssue")]
	RestartGame,

	/// Kick a player.
	#[doc(alias = "CKickIssue")]
	Kick,

	/// Change to another map now.
	#[doc(alias = "CChangeLevelIssue")]
	ChangeLevel,

	/// Choose the next map.
	#[doc(alias = "CNextLevelIssue")]
	NextLevel,

	/// Extend the current map.
	#[doc(alias = "CExtendLevelIssue")]
	ExtendLevel,

	/// Scramble the teams.
	#[doc(alias = "CScrambleTeams")]
	ScrambleTeams,

	/// Change the Mann vs. Machine mission.
	#[doc(alias = "CMannVsMachineChangeChallengeIssue")]
	ChangeMission,

	/// Enable Halloween mode temporarily.
	#[doc(alias = "CEnableTemporaryHalloweenIssue")]
	Eternaween,

	/// Toggle automatic team balancing.
	#[doc(alias = "CTeamAutoBalanceIssue")]
	TeamAutoBalance,

	/// Toggle class limits.
	#[doc(alias = "CClassLimitsIssue")]
	ClassLimits,

	/// Pause the game.
	#[doc(alias = "CPauseGameIssue")]
	PauseGame,
}

impl VoteIssue {
	/// Every built-in issue.
	pub const ALL: [Self; 11] = [
		Self::RestartGame,
		Self::Kick,
		Self::ChangeLevel,
		Self::NextLevel,
		Self::ExtendLevel,
		Self::ScrambleTeams,
		Self::ChangeMission,
		Self::Eternaween,
		Self::TeamAutoBalance,
		Self::ClassLimits,
		Self::PauseGame,
	];

	/// The issue's C++ class name, as its RTTI records it.
	fn class(self) -> &'static str {
		match self {
			Self::RestartGame => "CRestartGameIssue",
			Self::Kick => "CKickIssue",
			Self::ChangeLevel => "CChangeLevelIssue",
			Self::NextLevel => "CNextLevelIssue",
			Self::ExtendLevel => "CExtendLevelIssue",
			Self::ScrambleTeams => "CScrambleTeams",
			Self::ChangeMission => "CMannVsMachineChangeChallengeIssue",
			Self::Eternaween => "CEnableTemporaryHalloweenIssue",
			Self::TeamAutoBalance => "CTeamAutoBalanceIssue",
			Self::ClassLimits => "CClassLimitsIssue",
			Self::PauseGame => "CPauseGameIssue",
		}
	}

	/// The variant's name, such as `"RestartGame"`.
	pub const fn name(self) -> &'static str {
		match self {
			Self::RestartGame => "RestartGame",
			Self::Kick => "Kick",
			Self::ChangeLevel => "ChangeLevel",
			Self::NextLevel => "NextLevel",
			Self::ExtendLevel => "ExtendLevel",
			Self::ScrambleTeams => "ScrambleTeams",
			Self::ChangeMission => "ChangeMission",
			Self::Eternaween => "Eternaween",
			Self::TeamAutoBalance => "TeamAutoBalance",
			Self::ClassLimits => "ClassLimits",
			Self::PauseGame => "PauseGame",
		}
	}
}

/// A verified built-in issue's primary vtable in this server's game module.
/// Intended for the Metamod adapter; no per-map issue pointer is retained.
#[derive(Debug, Clone, Copy)]
pub struct VoteIssueVtable<'s> {
	/// The issue whose class owns this vtable.
	pub issue: VoteIssue,
	vtable: NonNull<*mut c_void>,
	_scope: PhantomData<&'s Server<'s>>,
}

impl VoteIssueVtable<'_> {
	/// The vtable's address in the game module.
	pub const fn as_ptr(self) -> NonNull<*mut c_void> {
		self.vtable
	}
}

/// A request that has reached TF2's issue-specific creation gate.
#[derive(Debug, Clone, Copy)]
pub struct VoteRequest<'a> {
	/// The requested issue.
	pub issue: VoteIssue,

	/// Entity index; TF2 uses the special value 99 for server requests.
	pub caller_entity_index: c_int,

	/// The issue's details string, such as a map name for map votes.
	pub details: &'a CStr,
}

impl VoteRequest<'_> {
	/// TF2's `DEDICATED_SERVER` sentinel, also used for automatic votes.
	pub const fn is_server_request(self) -> bool {
		self.caller_entity_index == DEDICATED_SERVER
	}
}

/// Receives vote requests on the server's main thread.
pub trait VoteStartHandler: 'static {
	/// Returning `Allow` does not force a vote to start; the game's own checks
	/// still run. A panic in a Metamod handler is contained and lets TF2 proceed.
	#[doc(alias = "RequestCallVote")]
	fn vote_start(&self, server: Server<'_>, request: VoteRequest<'_>) -> VoteDecision;
}

/// Decodes a vote event from its name and field getters.
fn decode(
	name: &CStr,
	mut integer: impl FnMut(&CStr) -> Option<c_int>,
	mut string: impl FnMut(&CStr) -> Option<CString>,
) -> Result<Option<VoteEvent>, VoteEventError> {
	if !VoteEvent::EVENTS.contains(&name) {
		return Ok(None);
	}

	let vote_id = integer(c"voteidx").ok_or(VoteEventError("voteidx"))?;

	if name == c"vote_options" {
		let keys = [c"option1", c"option2", c"option3", c"option4", c"option5"];

		let count = integer(c"count")
			.and_then(|count| usize::try_from(count).ok())
			.filter(|count| (2..=keys.len()).contains(count))
			.ok_or(VoteEventError("count"))?;

		let options = keys[..count]
			.iter()
			.map(|key| string(key).ok_or(VoteEventError("option")))
			.collect::<Result<Vec<_>, _>>()?;

		Ok(Some(VoteEvent::Started { vote_id, options }))
	} else {
		let entity_index = integer(c"entityid")
			.and_then(|index| u8::try_from(index).ok())
			.filter(|index| *index != 0)
			.ok_or(VoteEventError("entityid"))?;

		let choice = integer(c"vote_option")
			.and_then(|index| u8::try_from(index).ok())
			.and_then(VoteChoice::new)
			.ok_or(VoteEventError("vote_option"))?;

		let team = integer(c"team").ok_or(VoteEventError("team"))?;

		Ok(Some(VoteEvent::Cast {
			vote_id,
			entity_index,
			choice,
			team,
		}))
	}
}

/// Finds all built-in issue classes before any hooks are installed.
/// Missing, ambiguous, or incompatible RTTI is an error; partial protection
/// is never silently returned. Call once during plugin load.
pub fn vote_issue_vtables(
	server: Server<'_>,
) -> Result<Vec<VoteIssueVtable<'_>>, VoteHookTargetError> {
	if server.game() != Game::TeamFortress2 {
		return Err(VoteHookTargetError::WrongGame);
	}

	// SAFETY: Server's callback scope keeps its game factory and module loaded
	// while the module's readable sections are inspected.
	let virtuals = unsafe { VotingOvft::load(server.game_server_factory().as_raw() as usize) }?;

	VoteIssue::ALL
		.into_iter()
		.map(|issue| {
			let pointer = virtuals
				.find(issue.class(), REQUEST_CALL_VOTE_SLOT)
				.ok_or(VoteHookTargetError::UnsupportedIssue(issue))?;

			Ok(VoteIssueVtable {
				issue,
				vtable: pointer,
				_scope: PhantomData,
			})
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn accepted_ballot_keeps_entity_index_and_vote_id() {
		assert_eq!(
			parse(
				c"vote_cast",
				&[
					(c"voteidx", 42),
					(c"entityid", 7),
					(c"vote_option", 1),
					(c"team", 3)
				]
			)
			.unwrap(),
			Some(VoteEvent::Cast {
				vote_id: 42,
				entity_index: 7,
				choice: VoteChoice::NO,
				team: 3
			})
		);
	}

	fn parse(name: &CStr, fields: &[(&CStr, c_int)]) -> Result<Option<VoteEvent>, VoteEventError> {
		decode(
			name,
			|key| {
				fields
					.iter()
					.find(|(name, _)| *name == key)
					.map(|(_, n)| *n)
			},
			|key| match key.to_bytes() {
				b"option1" => Some(c"Yes".to_owned()),
				b"option2" => Some(c"No".to_owned()),
				_ => None,
			},
		)
	}

	#[test]
	fn rejects_missing_and_invalid_fields_without_defaulting_to_yes() {
		assert_eq!(parse(c"other_event", &[]).unwrap(), None);
		assert_eq!(parse(c"vote_cast", &[]), Err(VoteEventError("voteidx")));
		for count in [-1, 0, 1, 6] {
			assert_eq!(
				parse(c"vote_options", &[(c"voteidx", 1), (c"count", count)]),
				Err(VoteEventError("count"))
			);
		}
		for choice in [-1, 5, 256] {
			assert_eq!(
				parse(
					c"vote_cast",
					&[
						(c"voteidx", 0),
						(c"entityid", 1),
						(c"vote_option", choice),
						(c"team", 0)
					]
				),
				Err(VoteEventError("vote_option"))
			);
		}
	}

	#[test]
	fn starts_preserve_option_order_and_distinct_vote_ids() {
		for vote_id in [0, 17, 18] {
			assert_eq!(
				parse(c"vote_options", &[(c"voteidx", vote_id), (c"count", 2)]).unwrap(),
				Some(VoteEvent::Started {
					vote_id,
					options: vec![c"Yes".to_owned(), c"No".to_owned()]
				})
			);
		}
	}
}
