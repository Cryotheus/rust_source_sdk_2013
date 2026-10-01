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
use crate::{Game, Server};
use std::ffi::{CStr, CString, c_int, c_void};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// `CBaseIssue::RequestCallVote`'s slot, generated from Valve's declaration.
pub const REQUEST_CALL_VOTE_SLOT: usize =
	std::mem::offset_of!(sys::CBaseIssue__bindgen_vtable, CBaseIssue_RequestCallVote)
		/ size_of::<usize>();

/// An accepted choice, indexed from zero (yes is 0, no is 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VoteChoice(u8);

impl VoteChoice {
	pub const NO: Self = Self(1);
	pub const YES: Self = Self(0);

	pub const fn new(index: u8) -> Option<Self> {
		if index < 5 { Some(Self(index)) } else { None }
	}

	pub const fn index(self) -> usize {
		self.0 as usize
	}
}

/// Whether TF2 may evaluate its own issue-specific eligibility checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoteDecision {
	Allow,
	/// Refuse the vote with TF2's generic vote-creation failure response.
	Block,
}

/// Owned event data, safe to retain after the engine callback ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoteEvent {
	/// A vote was created. Option labels are in `VoteChoice` order.
	Started {
		vote_id: c_int,
		options: Vec<CString>,
	},
	/// TF2 accepted this player's choice for the identified vote.
	Cast {
		vote_id: c_int,
		entity_index: u8,
		choice: VoteChoice,
		/// 0 for a global vote, or the allowed team's number.
		team: c_int,
	},
}

impl VoteEvent {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("TF2 vote event has a missing or invalid `{0}` field")]
pub struct VoteEventError(pub &'static str);

/// Why the built-in vote gates cannot all be hooked safely.
#[derive(Debug, thiserror::Error)]
pub enum VoteHookTargetError {
	#[error("built-in voting hooks require Team Fortress 2")]
	WrongGame,
	#[error("the game module could not be inspected: {0}")]
	Image(#[from] std::io::Error),
	#[error("the game module has an unsupported executable image")]
	InvalidImage,
	#[error("no unique primary vtable found for TF2 vote issue {0:?}")]
	UnsupportedIssue(VoteIssue),
}

/// TF2's concrete built-in vote issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VoteIssue {
	RestartGame,
	Kick,
	ChangeLevel,
	NextLevel,
	ExtendLevel,
	ScrambleTeams,
	ChangeMission,
	Eternaween,
	TeamAutoBalance,
	ClassLimits,
	PauseGame,
}

impl VoteIssue {
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
	pub issue: VoteIssue,
	vtable: NonNull<*mut c_void>,
	_scope: PhantomData<&'s Server<'s>>,
}

impl VoteIssueVtable<'_> {
	pub const fn as_ptr(self) -> NonNull<*mut c_void> {
		self.vtable
	}
}

/// A request that has reached TF2's issue-specific creation gate.
#[derive(Debug, Clone, Copy)]
pub struct VoteRequest<'a> {
	pub issue: VoteIssue,
	/// Entity index; TF2 uses the special value 99 for server requests.
	pub caller_entity_index: c_int,
	pub details: &'a CStr,
}

impl VoteRequest<'_> {
	/// TF2's `DEDICATED_SERVER` sentinel, also used for automatic votes.
	pub const fn is_server_request(self) -> bool {
		self.caller_entity_index == 99
	}
}

/// Receives vote requests on the server's main thread.
pub trait VoteStartHandler: 'static {
	/// Returning `Allow` does not force a vote to start; the game's own checks
	/// still run. A panic in a Metamod handler is contained and lets TF2 proceed.
	fn vote_start(&self, server: Server<'_>, request: VoteRequest<'_>) -> VoteDecision;
}

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
		let count = integer(c"count")
			.filter(|count| (2..=5).contains(count))
			.ok_or(VoteEventError("count"))?;
		let keys = [c"option1", c"option2", c"option3", c"option4", c"option5"];
		let options = keys[..count as usize]
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
	// while the generic image utility copies the module's readable sections.
	let image = unsafe { vtables::Image::load(server.game_server_factory().as_raw() as usize) }?;
	VoteIssue::ALL
		.into_iter()
		.map(|issue| {
			let pointer = image
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
