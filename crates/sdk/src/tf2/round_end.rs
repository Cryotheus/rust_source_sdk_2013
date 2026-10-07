//! Ending TF2's rounds through its game rules (`CTFGameRules`): a team's win,
//! or a round without a winner, and sudden death, as the level's objectives
//! and timers end them.
//!
//! [`GameRules::set_winning_team`] and [`GameRules::set_stalemate`] call the
//! game rules' own methods, which a level's `game_round_win` calls when it is
//! sent `RoundWin`, but need no such entity. The round ends while the method
//! runs: the game fires `teamplay_round_win` or `teamplay_round_stalemate`,
//! and the callbacks of the game, of other plugins, and of this plugin run
//! before it returns, and must keep to the contract of [`Server::new`].
//!
//! [`Server::new`]: crate::Server::new

#[cfg(test)]
#[path = "../tests/tf2/round_end.rs"]
mod tests;

use crate::entities::EntityHandle;
use crate::interfaces::ServerTools;
use crate::tf2::game_rules::{GameRules, GameRulesError, RoundState};
use crate::tf2::objectives::WinReason;
use crate::tf2::scoreboard::ScoringTeam;
use sdk_raw::players::TEAM_UNASSIGNED;
use sdk_raw::tf2::game_rules as raw;
use std::ffi::c_int;

/// What follows a team's win, beyond who won and why: the options of
/// `SetWinningTeam`, whose [defaults](Self::default) are the game's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WinOptions {
	/// Whether the whole map resets after the bonus round (`bForceMapReset`),
	/// as at the end of a full round, which counts the round as played,
	/// rather than only the next mini-round of a level that plays several, as
	/// multi-stage Attack/Defend levels do.
	pub reset_map: bool,

	/// Whether the teams switch sides as the map resets (`bSwitchTeams`).
	pub switch_teams: bool,

	/// Whether the winners score the round (the opposite of `bDontAddScore`),
	/// which they only do if the map resets, or the time limit ran out, and if
	/// the game scores rounds.
	pub add_score: bool,

	/// Whether the win ends the game (`bFinal`), which TF2 only uses to time
	/// the winners' crit boost to the bonus round that ends a game, which
	/// competitive matches shorten.
	pub final_round: bool,
}

impl Default for WinOptions {
	/// A full round's end, which resets the map and scores the round, without
	/// switching teams or ending the game.
	fn default() -> Self {
		Self {
			reset_map: true,
			switch_teams: false,
			add_score: true,
			final_round: false,
		}
	}
}

/// What follows sudden death, or a round's end without a winner while
/// `mp_stalemate_enable` is off: the options of `SetStalemate`, whose
/// [defaults](Self::default) are the game's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StalemateOptions {
	/// Whether the whole map resets once the round ends (`bForceMapReset`), as
	/// [`WinOptions::reset_map`] does.
	pub reset_map: bool,

	/// Whether the teams switch sides as the map resets (`bSwitchTeams`), which
	/// TF2 only passes on to a round's end without a winner, while
	/// `mp_stalemate_enable` is off.
	pub switch_teams: bool,
}

impl Default for StalemateOptions {
	/// A stalemate that resets the map, without switching teams.
	fn default() -> Self {
		Self {
			reset_map: true,
			switch_teams: false,
		}
	}
}

/// Why a round went to sudden death, which `teamplay_round_stalemate` tells
/// its listeners (`STALEMATE_`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum StalemateReason {
	/// Players joined mid-round.
	#[doc(alias("STALEMATE_JOIN_MID"))]
	JoinedMidRound = raw::STALEMATE_JOIN_MID,

	/// The round's timer ran out.
	#[doc(alias("STALEMATE_TIMER"))]
	Timer = raw::STALEMATE_TIMER,

	/// The level's time limit ran out.
	#[doc(alias("STALEMATE_SERVER_TIMELIMIT"))]
	TimeLimit = raw::STALEMATE_SERVER_TIMELIMIT,
}

impl StalemateReason {
	/// Every reason, in the game's order.
	pub const ALL: [Self; 3] = [Self::JoinedMidRound, Self::Timer, Self::TimeLimit];

	/// The reason the game numbers `raw`, or `None` for any other value.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			raw::STALEMATE_JOIN_MID => Some(Self::JoinedMidRound),
			raw::STALEMATE_TIMER => Some(Self::Timer),
			raw::STALEMATE_SERVER_TIMELIMIT => Some(Self::TimeLimit),
			_ => None,
		}
	}

	/// The value the game numbers the reason with.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

/// Why a round could not be ended.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RoundEndError {
	/// The game rules could not be read.
	#[error(transparent)]
	GameRules(#[from] GameRulesError),

	/// A team already won the round. TF2 would still crit boost the team and
	/// count King of the Hill's and Payload Race's progress again, before the
	/// game rules ignore the win.
	#[error("a team already won the round")]
	AlreadyWon,

	/// The level plays King of the Hill, but the team's King of the Hill timer
	/// does not exist, which the game would read without checking. Its
	/// `tf_logic_koth` creates the timers as each round spawns.
	#[error("the {0:?} team's King of the Hill timer does not exist")]
	NoKothTimer(ScoringTeam),
}

/// Ending the round, which [the module](self) describes.
impl GameRules<'_> {
	/// Checks that ending the round is safe, and has TF2 do no more than the
	/// game rules accept.
	fn check_round_end(self, tools: ServerTools<'_>) -> Result<(), RoundEndError> {
		if self.round_state()? == RoundState::TeamWin {
			return Err(RoundEndError::AlreadyWon);
		}

		if self.is_king_of_the_hill()? {
			for team in [ScoringTeam::Red, ScoringTeam::Blue] {
				let timer = self.koth_timer(team)?;

				if timer
					.and_then(|timer| tools.entity_by_handle(timer))
					.is_none()
				{
					return Err(RoundEndError::NoKothTimer(team));
				}
			}
		}

		Ok(())
	}

	/// `team`'s King of the Hill timer, as the game rules hold it
	/// (`m_hRedKothTimer`), which counts down while the team holds the point,
	/// or `None` if it has none, as on levels that do not play King of the
	/// Hill, and before a round spawns the timers.
	///
	/// [`KothLogic::timer`](crate::tf2::objectives::KothLogic::timer) finds
	/// the same timers by name.
	#[doc(alias("m_hRedKothTimer", "m_hBlueKothTimer", "GetRedKothRoundTimer"))]
	pub fn koth_timer(self, team: ScoringTeam) -> Result<Option<EntityHandle>, GameRulesError> {
		self.read_handle(match team {
			ScoringTeam::Red => c"m_hRedKothTimer",
			ScoringTeam::Blue => c"m_hBlueKothTimer",
		})
	}

	/// Starts sudden death for `reason` (`SetStalemate`), which lasts until a
	/// team wins it, or its timer (`mp_stalemate_timelimit`) ends the round
	/// without a winner. Every building and projectile is removed, and outside
	/// Arena, players respawn, and health packs are disabled until it ends.
	/// While `mp_stalemate_enable` is off, the round ends without a winner
	/// straight away, as [`Self::set_winning_team`] ends it with `None`.
	///
	/// The game ignores the stalemate during sudden death, and in a
	/// tournament's pre-match. Fails, without calling the game, as
	/// [`Self::set_winning_team`] does.
	#[doc(alias("SetStalemate"))]
	pub fn set_stalemate(
		self,
		tools: ServerTools<'_>,
		reason: StalemateReason,
		options: StalemateOptions,
	) -> Result<(), RoundEndError> {
		self.check_round_end(tools)?;

		// SAFETY: The game rules are TF2's live `CTFGameRules`, as
		// `GameRules::get` checks the game, and they are only used on the main
		// thread. A round's end without a winner reads King of the Hill's
		// timers, which exist, as checked. What the method runs frees entities
		// only through deferred deletion (`Server::new`'s contract).
		unsafe {
			raw::set_stalemate(
				self.as_non_null(),
				reason.to_raw(),
				options.reset_map,
				options.switch_teams,
			);
		}

		Ok(())
	}

	/// Ends the round with `winner`'s win, or without a winner for `None`,
	/// for `reason` (`SetWinningTeam`), and starts the bonus round, in which
	/// the winners' living players are crit boosted and the losers humiliated.
	///
	/// The game ignores the win in commentary mode, after TF2 has crit boosted
	/// the winners. Fails with [`RoundEndError::AlreadyWon`], without calling
	/// the game, while a team has already won the round, and with
	/// [`RoundEndError::NoKothTimer`] on a King of the Hill level whose round
	/// has not spawned the teams' timers, which the game would read.
	#[doc(alias("SetWinningTeam"))]
	pub fn set_winning_team(
		self,
		tools: ServerTools<'_>,
		winner: Option<ScoringTeam>,
		reason: WinReason,
		options: WinOptions,
	) -> Result<(), RoundEndError> {
		self.check_round_end(tools)?;

		let team = winner.map_or(TEAM_UNASSIGNED, ScoringTeam::to_raw);

		// SAFETY: As for `set_stalemate`. The team is a playing team or
		// `TEAM_UNASSIGNED`.
		unsafe {
			raw::set_winning_team(
				self.as_non_null(),
				team,
				reason.to_raw(),
				options.reset_map,
				options.switch_teams,
				!options.add_score,
				options.final_round,
			);
		}

		Ok(())
	}
}
