//! Round timers (`team_round_timer`), King of the Hill's logic
//! (`tf_logic_koth`), and round wins (`game_round_win`).

#[cfg(test)]
#[path = "../../tests/tf2/objectives/round_timer.rs"]
mod tests;

use super::{Objective, ObjectiveError};
use crate::Server;
use crate::entities::Entity;
use crate::entities::outputs::OutputAction;
use crate::inputs::InputValue;
use crate::tf2::scoreboard::ScoringTeam;
use sdk_raw::players::TEAM_UNASSIGNED;
use sdk_raw::tf2::objectives as raw;
use std::ffi::{CStr, CString, c_int};

/// King of the Hill's logic (`tf_logic_koth`, `CKothLogic`), which gives each
/// team a timer counting down while it holds the point, and unlocks the point
/// some time after the round starts.
///
/// At each round's `RoundSpawn`, the logic creates a [`RoundTimer`] for each
/// team, [named](Self::timer) `zz_red_koth_timer` and `zz_blue_koth_timer`,
/// set to its [initial length](Self::initial_timer_length) and paused. The
/// game rules hold them too, as `m_hRedKothTimer` and `m_hBlueKothTimer`.
///
/// Its inputs only act while the game rules are in King of the Hill mode
/// (`IsInKothMode`).
#[doc(alias("tf_logic_koth", "CKothLogic"))]
#[derive(Debug, Clone, Copy)]
pub struct KothLogic<'s>(Objective<'s>);

impl<'s> KothLogic<'s> {
	/// Wraps King of the Hill's logic. Fails with
	/// [`ObjectiveError::WrongClass`] unless the server runs TF2 and
	/// `entity`'s data descriptions include `CKothLogic`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(server, entity, c"CKothLogic", "tf_logic_koth").map(Self)
	}

	/// Adds `seconds` to `team`'s timer, as [`RoundTimer::add_time`] does.
	#[doc(alias("AddRedTimer", "AddBlueTimer"))]
	pub fn add_timer(self, team: ScoringTeam, seconds: c_int) -> Result<(), ObjectiveError> {
		let input = match team {
			ScoringTeam::Red => c"AddRedTimer",
			ScoringTeam::Blue => c"AddBlueTimer",
		};

		self.0.input(input, InputValue::Int(seconds))
	}

	/// The logic's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// The seconds each team's timer starts each round with
	/// (`m_nTimerInitialLength`).
	#[doc(alias("m_nTimerInitialLength", "timer_length"))]
	pub fn initial_timer_length(self) -> Result<c_int, ObjectiveError> {
		self.0.int_field(c"CKothLogic", c"m_nTimerInitialLength")
	}

	/// Sets the time left on `team`'s timer, as [`RoundTimer::set_time`]
	/// does.
	#[doc(alias("SetRedTimer", "SetBlueTimer"))]
	pub fn set_timer(self, team: ScoringTeam, seconds: c_int) -> Result<(), ObjectiveError> {
		let input = match team {
			ScoringTeam::Red => c"SetRedTimer",
			ScoringTeam::Blue => c"SetBlueTimer",
		};

		self.0.input(input, InputValue::Int(seconds))
	}

	/// `team`'s timer: the first round timer named as the logic names it, or
	/// `None` if there is none, as before the first round spawns.
	pub fn timer(self, team: ScoringTeam) -> Result<Option<RoundTimer<'s>>, ObjectiveError> {
		let name = match team {
			ScoringTeam::Red => raw::KOTH_RED_TIMER_NAME,
			ScoringTeam::Blue => raw::KOTH_BLUE_TIMER_NAME,
		};

		super::find_by_name(self.0.server(), name, RoundTimer::new)
	}

	/// The seconds after the round starts that the point unlocks, or 0 if it
	/// starts unlocked (`m_nTimeToUnlockPoint`).
	#[doc(alias("m_nTimeToUnlockPoint", "unlock_point"))]
	pub fn unlock_delay(self) -> Result<c_int, ObjectiveError> {
		self.0.int_field(c"CKothLogic", c"m_nTimeToUnlockPoint")
	}
}

/// A round timer (`team_round_timer`, `CTeamRoundTimer`): the clock of a
/// round, or of its setup, which fires outputs as it runs down, and which
/// the HUD shows if it is the [timer shown](Self::shows_in_hud).
///
/// A timer counts down to its end time while it runs, and holds its
/// remaining time while it is paused. TF2 pauses timers at the end of a
/// round, and King of the Hill keeps the timer of the team not holding the
/// point paused.
///
/// The inputs this sends are those of `game/shared/teamplay_round_timer.cpp`.
/// While the timer is disabled, it ignores every input that changes its time.
#[doc(alias("team_round_timer", "CTeamRoundTimer"))]
#[derive(Debug, Clone, Copy)]
pub struct RoundTimer<'s>(Objective<'s>);

impl<'s> RoundTimer<'s> {
	/// Wraps a round timer. Fails with [`ObjectiveError::WrongClass`] unless
	/// the server runs TF2 and `entity`'s data descriptions include
	/// `CTeamRoundTimer`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(server, entity, c"CTeamRoundTimer", "team_round_timer").map(Self)
	}

	/// Adds time as [`Self::add_time`] does, crediting `team`, whose players
	/// hear that time was added for them while the other team hears it was
	/// added against them, if this is the timer the HUD shows.
	#[doc(alias("AddTeamTime"))]
	pub fn add_team_time(self, team: ScoringTeam, seconds: c_int) -> Result<(), ObjectiveError> {
		let value = CString::new(format!("{} {seconds}", team.to_raw()))
			.expect("integers format without NUL bytes");

		self.0.input(c"AddTeamTime", InputValue::String(&value))
	}

	/// Adds `seconds` to the timer, or takes them away if negative, up to its
	/// [maximum length](Self::max_length), as map logic does when a point is
	/// captured.
	///
	/// The game only adds time while the round runs or has just been won, and
	/// not in a stalemate (`CTeamRoundTimer::AddTimerSeconds`). In
	/// tournament stopwatch mode, it also keeps the timer below the stopwatch.
	#[doc(alias("AddTime"))]
	pub fn add_time(self, seconds: c_int) -> Result<(), ObjectiveError> {
		self.0.input(c"AddTime", InputValue::Int(seconds))
	}

	/// Whether the timer has the announcer count down its last seconds
	/// (`m_bAutoCountdown`).
	#[doc(alias("m_bAutoCountdown", "auto_countdown"))]
	pub fn announces_countdown(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bAutoCountdown")
	}

	/// Disables the timer: pauses it, and stops showing it in the HUD if it
	/// was the timer shown.
	#[doc(alias("Disable"))]
	pub fn disable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Disable", InputValue::Void)
	}

	/// Enables the timer: resumes it, and shows it in the HUD if it
	/// [shows in the HUD](Self::shows_in_hud).
	#[doc(alias("Enable"))]
	pub fn enable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Enable", InputValue::Void)
	}

	/// The game time at which the running timer reaches 0
	/// (`m_flTimerEndTime`). It is stale while the timer is paused.
	#[doc(alias("m_flTimerEndTime"))]
	pub fn end_time(self) -> Result<f32, ObjectiveError> {
		self.0.get(c"m_flTimerEndTime")
	}

	/// The timer's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// The length of the timer bar the HUD shows, in seconds, as
	/// `CTeamRoundTimer::GetTimerMaxLength` computes it: the
	/// [setup length](Self::setup_length) during setup, otherwise the
	/// [maximum length](Self::max_length), or the [length](Self::length)
	/// without one.
	#[doc(alias("GetTimerMaxLength"))]
	pub fn hud_length(self) -> Result<c_int, ObjectiveError> {
		if self.state()? == TimerState::Setup {
			return self.setup_length();
		}

		match self.max_length()? {
			Some(max) => Ok(max),
			None => self.length(),
		}
	}

	/// The length the timer starts each round with, and
	/// [restarts](Self::restart) to, in seconds (`m_nTimerInitialLength`).
	#[doc(alias("m_nTimerInitialLength", "timer_length"))]
	pub fn initial_length(self) -> Result<c_int, ObjectiveError> {
		self.0.get(c"m_nTimerInitialLength")
	}

	/// Whether the timer is in stopwatch mode's capture-watch state, in which
	/// it counts how long the attackers took instead
	/// (`m_bInCaptureWatchState`).
	#[doc(alias("m_bInCaptureWatchState"))]
	pub fn is_capture_watching(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bInCaptureWatchState")
	}

	/// Whether the timer is disabled (`m_bIsDisabled`).
	#[doc(alias("m_bIsDisabled", "StartDisabled"))]
	pub fn is_disabled(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bIsDisabled")
	}

	/// Whether the timer is paused (`m_bTimerPaused`).
	#[doc(alias("m_bTimerPaused"))]
	pub fn is_paused(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bTimerPaused")
	}

	/// Whether the timer is the one tournament stopwatch mode times rounds
	/// with (`m_bStopWatchTimer`).
	#[doc(alias("m_bStopWatchTimer"))]
	pub fn is_stopwatch(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bStopWatchTimer")
	}

	/// The timer's current length, in seconds, which setting and adding time
	/// change (`m_nTimerLength`).
	#[doc(alias("m_nTimerLength"))]
	pub fn length(self) -> Result<c_int, ObjectiveError> {
		self.0.get(c"m_nTimerLength")
	}

	/// The most seconds the timer can hold, or `None` without a maximum
	/// (`m_nTimerMaxLength`).
	#[doc(alias("m_nTimerMaxLength"))]
	pub fn max_length(self) -> Result<Option<c_int>, ObjectiveError> {
		let max: c_int = self.0.get(c"m_nTimerMaxLength")?;

		Ok((max > 0).then_some(max))
	}

	/// The actions of one of the timer's outputs, as
	/// [`Entity::output_actions`] reads them, or `None` if the timer lacks it.
	pub fn output_actions(self, output: RoundTimerOutput) -> Option<Vec<OutputAction>> {
		self.0.entity().output_actions(output.name())
	}

	/// Pauses the timer, keeping its remaining time.
	#[doc(alias("Pause"))]
	pub fn pause(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Pause", InputValue::Void)
	}

	/// Sets the timer back to its [initial length](Self::initial_length), as
	/// [`Self::set_time`] does.
	#[doc(alias("Restart"))]
	pub fn restart(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Restart", InputValue::Void)
	}

	/// Resumes the paused timer, from its remaining time.
	#[doc(alias("Resume"))]
	pub fn resume(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Resume", InputValue::Void)
	}

	/// Sets whether the announcer counts down the timer's last seconds.
	#[doc(alias("AutoCountdown"))]
	pub fn set_announces_countdown(self, announces: bool) -> Result<(), ObjectiveError> {
		self.0
			.input(c"AutoCountdown", InputValue::Int(c_int::from(announces)))
	}

	/// Sets the most seconds the timer can hold, shortening its remaining time
	/// to it, or removes the maximum with `None`.
	#[doc(alias("SetMaxTime"))]
	pub fn set_max_length(self, max: Option<c_int>) -> Result<(), ObjectiveError> {
		let max = max.filter(|max| *max > 0).unwrap_or(0);

		self.0.input(c"SetMaxTime", InputValue::Int(max))
	}

	/// Sets the length of the setup the timer counts down before the round,
	/// in seconds, for the next setup. Negative lengths are ignored.
	#[doc(alias("SetSetupTime"))]
	pub fn set_setup_length(self, seconds: c_int) -> Result<(), ObjectiveError> {
		self.0.input(c"SetSetupTime", InputValue::Int(seconds))
	}

	/// Sets whether the HUD shows the timer, replacing the timer it showed.
	#[doc(alias("ShowInHUD"))]
	pub fn set_shown_in_hud(self, shown: bool) -> Result<(), ObjectiveError> {
		self.0
			.input(c"ShowInHUD", InputValue::Int(c_int::from(shown)))
	}

	/// Sets the remaining time, in seconds, up to the timer's
	/// [maximum length](Self::max_length), and makes it the timer's
	/// [length](Self::length). A paused timer stays paused.
	///
	/// In tournament stopwatch mode, the game keeps the time below the
	/// stopwatch's, and the stopwatch's own timer takes a time stamp instead.
	#[doc(alias("SetTime"))]
	pub fn set_time(self, seconds: c_int) -> Result<(), ObjectiveError> {
		self.0.input(c"SetTime", InputValue::Int(seconds))
	}

	/// The length of the setup the timer counts down before the round, in
	/// seconds (`m_nSetupTimeLength`).
	#[doc(alias("m_nSetupTimeLength"))]
	pub fn setup_length(self) -> Result<c_int, ObjectiveError> {
		self.0.get(c"m_nSetupTimeLength")
	}

	/// Whether the timer [shows in the HUD](Self::set_shown_in_hud) while it
	/// is enabled (`m_bShowInHUD`).
	#[doc(alias("m_bShowInHUD", "show_in_hud"))]
	pub fn shows_in_hud(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bShowInHUD")
	}

	/// Whether the HUD shows the time remaining, rather than the time elapsed
	/// (`m_bShowTimeRemaining`).
	#[doc(alias("m_bShowTimeRemaining", "show_time_remaining"))]
	pub fn shows_time_remaining(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bShowTimeRemaining")
	}

	/// Whether the timer counts down the setup or the round (`m_nState`).
	#[doc(alias("m_nState"))]
	pub fn state(self) -> Result<TimerState, ObjectiveError> {
		let state: c_int = self.0.get(c"m_nState")?;

		TimerState::from_raw(state).ok_or(ObjectiveError::UnknownValue {
			name: c"m_nState",
			value: state,
		})
	}

	/// The seconds left on the timer, at least 0, as
	/// `CTeamRoundTimer::GetTimeRemaining` computes them from the game time:
	/// the time it holds while paused, and the time to its
	/// [end](Self::end_time) while running. A stopwatch in its capture-watch
	/// state gives the time it counted instead (`m_flTotalTime`).
	#[doc(alias("GetTimeRemaining"))]
	pub fn time_remaining(self) -> Result<f32, ObjectiveError> {
		let remaining = if self.is_stopwatch()? && self.is_capture_watching()? {
			self.0.get(c"m_flTotalTime")?
		} else if self.is_paused()? {
			self.0.get(c"m_flTimeRemaining")?
		} else {
			self.end_time()? - self.0.current_time()?
		};

		Ok(remaining.max(0.0))
	}
}

/// An output of a [`RoundTimer`], which a map connects to the inputs it
/// sends as the timer runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoundTimerOutput {
	/// The round started (`OnRoundStart`).
	#[doc(alias("OnRoundStart"))]
	RoundStart,

	/// The timer reached 0 (`OnFinished`).
	#[doc(alias("OnFinished"))]
	Finished,

	/// The timer started counting down the setup (`OnSetupStart`).
	#[doc(alias("OnSetupStart"))]
	SetupStart,

	/// The setup ended (`OnSetupFinished`).
	#[doc(alias("OnSetupFinished"))]
	SetupFinished,

	/// Five minutes are left (`On5MinRemain`).
	#[doc(alias("On5MinRemain"))]
	FiveMinutesLeft,

	/// Four minutes are left (`On4MinRemain`).
	#[doc(alias("On4MinRemain"))]
	FourMinutesLeft,

	/// Three minutes are left (`On3MinRemain`).
	#[doc(alias("On3MinRemain"))]
	ThreeMinutesLeft,

	/// Two minutes are left (`On2MinRemain`).
	#[doc(alias("On2MinRemain"))]
	TwoMinutesLeft,

	/// One minute is left (`On1MinRemain`).
	#[doc(alias("On1MinRemain"))]
	OneMinuteLeft,

	/// Thirty seconds are left (`On30SecRemain`).
	#[doc(alias("On30SecRemain"))]
	ThirtySecondsLeft,

	/// Ten seconds are left (`On10SecRemain`).
	#[doc(alias("On10SecRemain"))]
	TenSecondsLeft,

	/// Five seconds are left (`On5SecRemain`).
	#[doc(alias("On5SecRemain"))]
	FiveSecondsLeft,

	/// Four seconds are left (`On4SecRemain`).
	#[doc(alias("On4SecRemain"))]
	FourSecondsLeft,

	/// Three seconds are left (`On3SecRemain`).
	#[doc(alias("On3SecRemain"))]
	ThreeSecondsLeft,

	/// Two seconds are left (`On2SecRemain`).
	#[doc(alias("On2SecRemain"))]
	TwoSecondsLeft,

	/// One second is left (`On1SecRemain`).
	#[doc(alias("On1SecRemain"))]
	OneSecondLeft,
}

impl RoundTimerOutput {
	/// The output's name, as the timer's data description declares it.
	pub const fn name(self) -> &'static CStr {
		match self {
			Self::RoundStart => c"OnRoundStart",
			Self::Finished => c"OnFinished",
			Self::SetupStart => c"OnSetupStart",
			Self::SetupFinished => c"OnSetupFinished",
			Self::FiveMinutesLeft => c"On5MinRemain",
			Self::FourMinutesLeft => c"On4MinRemain",
			Self::ThreeMinutesLeft => c"On3MinRemain",
			Self::TwoMinutesLeft => c"On2MinRemain",
			Self::OneMinuteLeft => c"On1MinRemain",
			Self::ThirtySecondsLeft => c"On30SecRemain",
			Self::TenSecondsLeft => c"On10SecRemain",
			Self::FiveSecondsLeft => c"On5SecRemain",
			Self::FourSecondsLeft => c"On4SecRemain",
			Self::ThreeSecondsLeft => c"On3SecRemain",
			Self::TwoSecondsLeft => c"On2SecRemain",
			Self::OneSecondLeft => c"On1SecRemain",
		}
	}
}

/// A round win (`game_round_win`, `CTeamplayRoundWin`), which ends the round
/// with a win for its team, or a stalemate without one, when map logic or
/// [`Self::win`] sends it `RoundWin`.
#[doc(alias("game_round_win", "CTeamplayRoundWin"))]
#[derive(Debug, Clone, Copy)]
pub struct RoundWin<'s>(Objective<'s>);

impl<'s> RoundWin<'s> {
	/// Wraps a round win. Fails with [`ObjectiveError::WrongClass`] unless the
	/// server runs TF2 and `entity`'s data descriptions include
	/// `CTeamplayRoundWin`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(server, entity, c"CTeamplayRoundWin", "game_round_win").map(Self)
	}

	/// The round win's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Whether winning restarts the whole map, with every mini-round, rather
	/// than only the next round (`m_bForceMapReset`).
	#[doc(alias("m_bForceMapReset", "force_map_reset"))]
	pub fn resets_map(self) -> Result<bool, ObjectiveError> {
		self.0.bool_field(c"CTeamplayRoundWin", c"m_bForceMapReset")
	}

	/// Sets whether winning restarts the whole map.
	pub fn set_resets_map(self, resets: bool) -> Result<(), ObjectiveError> {
		self.0
			.set_key_value(c"force_map_reset", if resets { c"1" } else { c"0" })
	}

	/// Sets whether the teams switch sides after a win that restarts the map.
	pub fn set_switches_teams(self, switches: bool) -> Result<(), ObjectiveError> {
		self.0
			.set_key_value(c"switch_teams", if switches { c"1" } else { c"0" })
	}

	/// Sets the team that wins, or a stalemate with `None`.
	#[doc(alias("SetTeam"))]
	pub fn set_team(self, team: Option<ScoringTeam>) -> Result<(), ObjectiveError> {
		let team = team.map_or(TEAM_UNASSIGNED, ScoringTeam::to_raw);

		self.0.input(c"SetTeam", InputValue::Int(team))
	}

	/// Sets the reason the win panel gives for the win.
	pub fn set_win_reason(self, reason: WinReason) -> Result<(), ObjectiveError> {
		let value =
			CString::new(reason.to_raw().to_string()).expect("integers format without NUL bytes");

		self.0.set_key_value(c"win_reason", &value)
	}

	/// Whether the teams switch sides after a win that restarts the map
	/// (`m_bSwitchTeamsOnWin`).
	#[doc(alias("m_bSwitchTeamsOnWin", "switch_teams"))]
	pub fn switches_teams(self) -> Result<bool, ObjectiveError> {
		self.0
			.bool_field(c"CTeamplayRoundWin", c"m_bSwitchTeamsOnWin")
	}

	/// The team that wins, or `None` for a stalemate (`m_iTeamNum`). A team
	/// number other than RED's or BLU's also ends in a stalemate.
	#[doc(alias("m_iTeamNum", "TeamNum"))]
	pub fn team(self) -> Result<Option<ScoringTeam>, ObjectiveError> {
		Ok(ScoringTeam::from_raw(
			self.0.int_field(c"CBaseEntity", c"m_iTeamNum")?,
		))
	}

	/// Ends the round, with a win for [the team](Self::team) for
	/// [the reason](Self::win_reason), or a stalemate without one, and fires
	/// the `OnRoundWin` output (`CTeamplayRoundWin::RoundWin`).
	///
	/// The game ignores a win in commentary mode. Fails with
	/// [`ObjectiveError::RoundEnd`], without sending the input, while a team
	/// has already won the round, which TF2 would crit boost again before
	/// ignoring the win, and on a King of the Hill level whose round has not
	/// spawned the teams' timers, which the game would read.
	/// [`GameRules::set_winning_team`] ends rounds without an entity.
	///
	/// [`GameRules::set_winning_team`]: crate::tf2::game_rules::GameRules::set_winning_team
	#[doc(alias("RoundWin"))]
	pub fn win(self) -> Result<(), ObjectiveError> {
		self.0.check_round_end()?;
		self.0.input(c"RoundWin", InputValue::Void)
	}

	/// The reason the win panel gives for the win (`m_iWinReason`), which is
	/// [`WinReason::DefendedUntilTimeLimit`] unless the map changes it.
	#[doc(alias("m_iWinReason"))]
	pub fn win_reason(self) -> Result<WinReason, ObjectiveError> {
		let reason = self.0.int_field(c"CTeamplayRoundWin", c"m_iWinReason")?;

		WinReason::from_raw(reason).ok_or(ObjectiveError::UnknownValue {
			name: c"m_iWinReason",
			value: reason,
		})
	}
}

/// What a [`RoundTimer`] counts down (`m_nState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimerState {
	/// The setup before the round, while the attackers' gates stay closed.
	#[doc(alias("RT_STATE_SETUP"))]
	Setup,

	/// The round.
	#[doc(alias("RT_STATE_NORMAL"))]
	Normal,
}

impl TimerState {
	/// The state of this `RT_STATE_` value, or `None` for another value.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			raw::RT_STATE_SETUP => Some(Self::Setup),
			raw::RT_STATE_NORMAL => Some(Self::Normal),
			_ => None,
		}
	}

	/// The state's `RT_STATE_` value.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Setup => raw::RT_STATE_SETUP,
			Self::Normal => raw::RT_STATE_NORMAL,
		}
	}
}

/// Why a team won a round, which the win panel shows (`WINREASON_`), as TF2
/// numbers the reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WinReason {
	/// No reason.
	#[doc(alias("WINREASON_NONE"))]
	None,

	/// The team captured every control point.
	#[doc(alias("WINREASON_ALL_POINTS_CAPTURED"))]
	AllPointsCaptured,

	/// The other team was eliminated, as in Arena.
	#[doc(alias("WINREASON_OPPONENTS_DEAD"))]
	OpponentsDead,

	/// The team reached the flag capture limit.
	#[doc(alias("WINREASON_FLAG_CAPTURE_LIMIT"))]
	FlagCaptureLimit,

	/// The team defended until the round timer ran out.
	#[doc(alias("WINREASON_DEFEND_UNTIL_TIME_LIMIT"))]
	DefendedUntilTimeLimit,

	/// A stalemate.
	#[doc(alias("WINREASON_STALEMATE"))]
	Stalemate,

	/// The map's time limit ran out.
	#[doc(alias("WINREASON_TIMELIMIT"))]
	TimeLimit,

	/// The team reached the win limit.
	#[doc(alias("WINREASON_WINLIMIT"))]
	WinLimit,

	/// The team reached the win difference limit.
	#[doc(alias("WINREASON_WINDIFFLIMIT"))]
	WinDifferenceLimit,

	/// The team captured Robot Destruction's reactor core.
	#[doc(alias("WINREASON_RD_REACTOR_CAPTURED"))]
	ReactorCaptured,

	/// The team collected Robot Destruction's cores.
	#[doc(alias("WINREASON_RD_CORES_COLLECTED"))]
	CoresCollected,

	/// The team returned Robot Destruction's reactor core.
	#[doc(alias("WINREASON_RD_REACTOR_RETURNED"))]
	ReactorReturned,

	/// The team collected Player Destruction's points.
	#[doc(alias("WINREASON_PD_POINTS"))]
	PlayerDestructionPoints,

	/// The team scored, as in PASS Time.
	#[doc(alias("WINREASON_SCORED"))]
	Scored,

	/// The team watching a stopwatch round won it.
	#[doc(alias("WINREASON_STOPWATCH_WATCHING_ROUNDS"))]
	StopwatchWatchingRounds,

	/// The team watching the final stopwatch round won it.
	#[doc(alias("WINREASON_STOPWATCH_WATCHING_FINAL_ROUND"))]
	StopwatchWatchingFinalRound,

	/// The team playing a stopwatch round won it.
	#[doc(alias("WINREASON_STOPWATCH_PLAYING_ROUNDS"))]
	StopwatchPlayingRounds,
}

impl WinReason {
	/// Every reason, in TF2's order.
	pub const ALL: [Self; 17] = [
		Self::None,
		Self::AllPointsCaptured,
		Self::OpponentsDead,
		Self::FlagCaptureLimit,
		Self::DefendedUntilTimeLimit,
		Self::Stalemate,
		Self::TimeLimit,
		Self::WinLimit,
		Self::WinDifferenceLimit,
		Self::ReactorCaptured,
		Self::CoresCollected,
		Self::ReactorReturned,
		Self::PlayerDestructionPoints,
		Self::Scored,
		Self::StopwatchWatchingRounds,
		Self::StopwatchWatchingFinalRound,
		Self::StopwatchPlayingRounds,
	];

	/// The reason of this `WINREASON_` value, or `None` for another value.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		Some(match raw {
			raw::WINREASON_NONE => Self::None,
			raw::WINREASON_ALL_POINTS_CAPTURED => Self::AllPointsCaptured,
			raw::WINREASON_OPPONENTS_DEAD => Self::OpponentsDead,
			raw::WINREASON_FLAG_CAPTURE_LIMIT => Self::FlagCaptureLimit,
			raw::WINREASON_DEFEND_UNTIL_TIME_LIMIT => Self::DefendedUntilTimeLimit,
			raw::WINREASON_STALEMATE => Self::Stalemate,
			raw::WINREASON_TIMELIMIT => Self::TimeLimit,
			raw::WINREASON_WINLIMIT => Self::WinLimit,
			raw::WINREASON_WINDIFFLIMIT => Self::WinDifferenceLimit,
			raw::WINREASON_RD_REACTOR_CAPTURED => Self::ReactorCaptured,
			raw::WINREASON_RD_CORES_COLLECTED => Self::CoresCollected,
			raw::WINREASON_RD_REACTOR_RETURNED => Self::ReactorReturned,
			raw::WINREASON_PD_POINTS => Self::PlayerDestructionPoints,
			raw::WINREASON_SCORED => Self::Scored,
			raw::WINREASON_STOPWATCH_WATCHING_ROUNDS => Self::StopwatchWatchingRounds,
			raw::WINREASON_STOPWATCH_WATCHING_FINAL_ROUND => Self::StopwatchWatchingFinalRound,
			raw::WINREASON_STOPWATCH_PLAYING_ROUNDS => Self::StopwatchPlayingRounds,
			_ => return None,
		})
	}

	/// The reason's `WINREASON_` value.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::None => raw::WINREASON_NONE,
			Self::AllPointsCaptured => raw::WINREASON_ALL_POINTS_CAPTURED,
			Self::OpponentsDead => raw::WINREASON_OPPONENTS_DEAD,
			Self::FlagCaptureLimit => raw::WINREASON_FLAG_CAPTURE_LIMIT,
			Self::DefendedUntilTimeLimit => raw::WINREASON_DEFEND_UNTIL_TIME_LIMIT,
			Self::Stalemate => raw::WINREASON_STALEMATE,
			Self::TimeLimit => raw::WINREASON_TIMELIMIT,
			Self::WinLimit => raw::WINREASON_WINLIMIT,
			Self::WinDifferenceLimit => raw::WINREASON_WINDIFFLIMIT,
			Self::ReactorCaptured => raw::WINREASON_RD_REACTOR_CAPTURED,
			Self::CoresCollected => raw::WINREASON_RD_CORES_COLLECTED,
			Self::ReactorReturned => raw::WINREASON_RD_REACTOR_RETURNED,
			Self::PlayerDestructionPoints => raw::WINREASON_PD_POINTS,
			Self::Scored => raw::WINREASON_SCORED,
			Self::StopwatchWatchingRounds => raw::WINREASON_STOPWATCH_WATCHING_ROUNDS,
			Self::StopwatchWatchingFinalRound => raw::WINREASON_STOPWATCH_WATCHING_FINAL_ROUND,
			Self::StopwatchPlayingRounds => raw::WINREASON_STOPWATCH_PLAYING_ROUNDS,
		}
	}
}
