//! Control points (`team_control_point`), the master that wins rounds with
//! them (`team_control_point_master`), and their mini-rounds
//! (`team_control_point_round`).

#[cfg(test)]
#[path = "../../tests/tf2/objectives/control_points.rs"]
mod tests;

use super::{Objective, ObjectiveError};
use crate::Server;
use crate::entities::Entity;
use crate::inputs::InputValue;
use crate::tf2::scoreboard::ScoringTeam;
use sdk_raw::players::TEAM_UNASSIGNED;
use std::ffi::{CStr, CString, c_int};

/// The value of `cpm_restrict_team_cap_win` and `cpr_restrict_team_cap_win`
/// that keeps either team from winning by owning every point.
const NEITHER_WINS_BY_CAPTURE: c_int = 1;

/// Which teams win a round by owning every control point of it, as a
/// [master](ControlPointMaster) or a [mini-round](ControlPointRound) holds
/// it (`m_iInvalidCapWinner`).
///
/// A team that does not win by owning every point can still win the round
/// in other ways, such as when the round timer runs out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CaptureWins {
	/// Either team wins by owning every point.
	Either,

	/// Neither team wins by owning every point.
	Neither,

	/// Only the team other than this one wins by owning every point, as
	/// attacking teams do.
	Except(ScoringTeam),
}

impl CaptureWins {
	/// The rule of this `m_iInvalidCapWinner`, or `None` for another value.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			TEAM_UNASSIGNED => Some(Self::Either),
			NEITHER_WINS_BY_CAPTURE => Some(Self::Neither),

			_ => match ScoringTeam::from_raw(raw) {
				Some(team) => Some(Self::Except(team)),
				None => None,
			},
		}
	}

	/// The rule as a key value.
	fn to_key_value(self) -> CString {
		CString::new(self.to_raw().to_string()).expect("integers format without NUL bytes")
	}

	/// The rule's `m_iInvalidCapWinner`: 0 for either team, 1 for neither, or
	/// the number of the team that does not win.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Either => TEAM_UNASSIGNED,
			Self::Neither => NEITHER_WINS_BY_CAPTURE,
			Self::Except(team) => team.to_raw(),
		}
	}
}

/// A control point (`team_control_point`, `CTeamControlPoint`), which a team
/// owns, and which players capture by standing in its
/// [capture area](super::CaptureArea).
///
/// When a team comes to own every point of the round, or of the mini-round,
/// the [master](ControlPointMaster) ends it with a win for that team, unless
/// its [capture wins](ControlPointMaster::capture_wins) say otherwise.
#[doc(alias("team_control_point", "CTeamControlPoint"))]
#[derive(Debug, Clone, Copy)]
pub struct ControlPoint<'s>(Objective<'s>);

impl<'s> ControlPoint<'s> {
	/// Wraps a control point. Fails with [`ObjectiveError::WrongClass`]
	/// unless the server runs TF2 and `entity`'s data descriptions include
	/// `CTeamControlPoint`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(server, entity, c"CTeamControlPoint", "team_control_point").map(Self)
	}

	/// Gives the point to `owner` as if `capper` had captured it: as
	/// [`Self::set_owner`] does, and if `capper` is a player, also playing the
	/// capture sound and crediting the capture to `capper`, as captures in
	/// the point's area are credited (`CTeamControlPoint::InputSetOwner`).
	#[doc(alias("SetOwner"))]
	pub fn capture(self, owner: ScoringTeam, capper: Entity<'_>) -> Result<(), ObjectiveError> {
		self.0
			.input_from(c"SetOwner", InputValue::Int(owner.to_raw()), capper, capper)
	}

	/// The team that owns the point at the start of each round, or `None` if
	/// no team does (`m_iDefaultOwner`).
	#[doc(alias("m_iDefaultOwner", "point_default_owner"))]
	pub fn default_owner(self) -> Result<Option<ScoringTeam>, ObjectiveError> {
		Ok(ScoringTeam::from_raw(
			self.0.int_field(c"CTeamControlPoint", c"m_iDefaultOwner")?,
		))
	}

	/// The point's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// The group the map puts the point in, which the HUD lays out together
	/// (`m_iCPGroup`).
	#[doc(alias("m_iCPGroup", "point_group"))]
	pub fn group(self) -> Result<c_int, ObjectiveError> {
		self.0.int_field(c"CTeamControlPoint", c"m_iCPGroup")
	}

	/// Hides the point's model, though not its place in the HUD.
	#[doc(alias("HideModel"))]
	pub fn hide_model(self) -> Result<(), ObjectiveError> {
		self.0.input(c"HideModel", InputValue::Void)
	}

	/// The point's index (`m_iPointIndex`): its position among the round's
	/// points, which the [objective resource](super::ObjectiveResource)
	/// numbers its state by.
	///
	/// The master numbers the points from 0 in the order of their
	/// `point_index` when it sets up the round's points.
	#[doc(alias("m_iPointIndex", "point_index", "GetPointIndex"))]
	pub fn index(self) -> Result<c_int, ObjectiveError> {
		self.0.int_field(c"CTeamControlPoint", c"m_iPointIndex")
	}

	/// Whether the point is locked, so that no team can capture it
	/// (`m_bLocked`).
	#[doc(alias("m_bLocked", "point_start_locked"))]
	pub fn is_locked(self) -> Result<bool, ObjectiveError> {
		self.0.bool_field(c"CTeamControlPoint", c"m_bLocked")
	}

	/// The team that owns the point, or `None` if no team does
	/// (`m_iTeamNum`).
	#[doc(alias("m_iTeamNum", "GetOwner"))]
	pub fn owner(self) -> Result<Option<ScoringTeam>, ObjectiveError> {
		Ok(ScoringTeam::from_raw(
			self.0.int_field(c"CBaseEntity", c"m_iTeamNum")?,
		))
	}

	/// The point's name, which the HUD and the game's messages show
	/// (`m_iszPrintName`).
	#[doc(alias("m_iszPrintName", "point_printname"))]
	pub fn print_name(self) -> Result<CString, ObjectiveError> {
		self.0.string_key(c"CTeamControlPoint", c"point_printname")
	}

	/// Locks or unlocks the point. The point ignores it while the game waits
	/// for players.
	#[doc(alias("SetLocked"))]
	pub fn set_locked(self, locked: bool) -> Result<(), ObjectiveError> {
		self.0
			.input(c"SetLocked", InputValue::Int(c_int::from(locked)))
	}

	/// Gives the point to `owner`, or to no team with `None`, without
	/// crediting a player or playing a sound.
	///
	/// The point only changes hands while points may be captured
	/// (`PointsMayBeCaptured`): while the round runs or is in sudden death,
	/// and not in the setup or while the game waits for players. Giving a
	/// team the last point it lacks can win it the round.
	#[doc(alias("SetOwner"))]
	pub fn set_owner(self, owner: Option<ScoringTeam>) -> Result<(), ObjectiveError> {
		self.0
			.input(c"SetOwner", InputValue::Int(team_number(owner)))
	}

	/// Unlocks the point once `seconds` have passed and the round is
	/// running, or at once if `seconds` is 0 or less. The point ignores it
	/// while the game waits for players.
	#[doc(alias("SetUnlockTime"))]
	pub fn set_unlock_time(self, seconds: c_int) -> Result<(), ObjectiveError> {
		self.0.input(c"SetUnlockTime", InputValue::Int(seconds))
	}

	/// Shows the point's model again after [`Self::hide_model`].
	#[doc(alias("ShowModel"))]
	pub fn show_model(self) -> Result<(), ObjectiveError> {
		self.0.input(c"ShowModel", InputValue::Void)
	}
}

/// The control point master (`team_control_point_master`,
/// `CTeamControlPointMaster`), which sets up the round's control points and
/// ends the round when a team owns all of them.
///
/// A master networks none of its own state, so it is read from its data
/// description.
#[doc(alias("team_control_point_master", "CTeamControlPointMaster"))]
#[derive(Debug, Clone, Copy)]
pub struct ControlPointMaster<'s>(Objective<'s>);

impl<'s> ControlPointMaster<'s> {
	/// Wraps a control point master. Fails with
	/// [`ObjectiveError::WrongClass`] unless the server runs TF2 and
	/// `entity`'s data descriptions include `CTeamControlPointMaster`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(
			server,
			entity,
			c"CTeamControlPointMaster",
			"team_control_point_master",
		)
		.map(Self)
	}

	/// Finds the first control point master, or `None` if the map has none.
	pub fn find(server: Server<'s>) -> Result<Option<Self>, ObjectiveError> {
		super::find_by_class(server, c"team_control_point_master", Self::new)
	}

	/// The layout of the points in the HUD (`m_iszCapLayoutInHUD`): their
	/// indexes, with commas between rows, such as `2, 0 1` for point 2 above
	/// points 0 and 1, or empty for one row of every point.
	#[doc(alias("m_iszCapLayoutInHUD", "caplayout"))]
	pub fn cap_layout(self) -> Result<CString, ObjectiveError> {
		self.0.string_key(c"CTeamControlPointMaster", c"caplayout")
	}

	/// Which teams win the round by owning every point
	/// (`m_iInvalidCapWinner`).
	#[doc(alias("m_iInvalidCapWinner", "cpm_restrict_team_cap_win"))]
	pub fn capture_wins(self) -> Result<CaptureWins, ObjectiveError> {
		let raw = self
			.0
			.int_field(c"CTeamControlPointMaster", c"m_iInvalidCapWinner")?;

		CaptureWins::from_raw(raw).ok_or(ObjectiveError::UnknownValue {
			name: c"m_iInvalidCapWinner",
			value: raw,
		})
	}

	/// Disables the master, which then neither checks for nor awards wins.
	#[doc(alias("Disable"))]
	pub fn disable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Disable", InputValue::Void)
	}

	/// Enables the master.
	#[doc(alias("Enable"))]
	pub fn enable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Enable", InputValue::Void)
	}

	/// The master's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Whether the master is disabled (`m_bDisabled`).
	#[doc(alias("m_bDisabled", "StartDisabled"))]
	pub fn is_disabled(self) -> Result<bool, ObjectiveError> {
		self.0
			.bool_field(c"CTeamControlPointMaster", c"m_bDisabled")
	}

	/// Sets the [layout of the points](Self::cap_layout) in the HUD, which
	/// the HUD takes the first 31 bytes of.
	#[doc(alias("SetCapLayout"))]
	pub fn set_cap_layout(self, layout: &CStr) -> Result<(), ObjectiveError> {
		self.0.input(c"SetCapLayout", InputValue::String(layout))
	}

	/// Moves the HUD's points to `x` and `y`, as fractions of the screen, or
	/// back to their usual place along an axis with -1.
	#[doc(alias("SetCapLayoutCustomPositionX", "SetCapLayoutCustomPositionY"))]
	pub fn set_cap_layout_position(self, x: f32, y: f32) -> Result<(), ObjectiveError> {
		self.0
			.input(c"SetCapLayoutCustomPositionX", InputValue::Float(x))?;
		self.0
			.input(c"SetCapLayoutCustomPositionY", InputValue::Float(y))
	}

	/// Sets which teams win the round by owning every point.
	#[doc(alias("cpm_restrict_team_cap_win"))]
	pub fn set_capture_wins(self, wins: CaptureWins) -> Result<(), ObjectiveError> {
		self.0
			.set_key_value(c"cpm_restrict_team_cap_win", &wins.to_key_value())
	}

	/// Sets whether the teams switch sides when one wins by capturing.
	#[doc(alias("switch_teams"))]
	pub fn set_switches_teams(self, switches: bool) -> Result<(), ObjectiveError> {
		self.0
			.set_key_value(c"switch_teams", if switches { c"1" } else { c"0" })
	}

	/// Ends the round with a win for `winner`, for capturing every point, or
	/// a stalemate with `None`, and fires the master's `OnWonByTeam1` or
	/// `OnWonByTeam2` output for a win.
	///
	/// Fails with [`ObjectiveError::RoundEnd`], without sending the input, as
	/// [`RoundWin::win`](super::RoundWin::win) does.
	#[doc(alias("SetWinner"))]
	pub fn set_winner(self, winner: Option<ScoringTeam>) -> Result<(), ObjectiveError> {
		self.0.check_round_end()?;
		self.0
			.input(c"SetWinner", InputValue::Int(team_number(winner)))
	}

	/// Gives every point of the round, or of the mini-round, to `winner`, or
	/// to no team with `None`, without crediting anyone, then ends the round
	/// as [`Self::set_winner`] does, and fails as it does, before giving any
	/// point away.
	#[doc(alias("SetWinnerAndForceCaps"))]
	pub fn set_winner_and_force_caps(
		self,
		winner: Option<ScoringTeam>,
	) -> Result<(), ObjectiveError> {
		self.0.check_round_end()?;
		self.0.input(
			c"SetWinnerAndForceCaps",
			InputValue::Int(team_number(winner)),
		)
	}

	/// Whether the teams switch sides when one wins by capturing
	/// (`m_bSwitchTeamsOnWin`).
	#[doc(alias("m_bSwitchTeamsOnWin", "switch_teams"))]
	pub fn switches_teams(self) -> Result<bool, ObjectiveError> {
		self.0
			.bool_field(c"CTeamControlPointMaster", c"m_bSwitchTeamsOnWin")
	}
}

/// A mini-round (`team_control_point_round`, `CTeamControlPointRound`): a
/// set of control points the master plays as a round of its own, as Hydro
/// plays its territories.
///
/// The master plays mini-rounds of the highest [priority](Self::priority)
/// first, picking at random among playable ones of equal priority. A
/// mini-round networks none of its own state, so it is read from its data
/// description.
#[doc(alias("team_control_point_round", "CTeamControlPointRound"))]
#[derive(Debug, Clone, Copy)]
pub struct ControlPointRound<'s>(Objective<'s>);

impl<'s> ControlPointRound<'s> {
	/// Wraps a mini-round. Fails with [`ObjectiveError::WrongClass`] unless
	/// the server runs TF2 and `entity`'s data descriptions include
	/// `CTeamControlPointRound`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(
			server,
			entity,
			c"CTeamControlPointRound",
			"team_control_point_round",
		)
		.map(Self)
	}

	/// Which teams win the mini-round by owning all its points
	/// (`m_iInvalidCapWinner`).
	#[doc(alias("m_iInvalidCapWinner", "cpr_restrict_team_cap_win"))]
	pub fn capture_wins(self) -> Result<CaptureWins, ObjectiveError> {
		let raw = self
			.0
			.int_field(c"CTeamControlPointRound", c"m_iInvalidCapWinner")?;

		CaptureWins::from_raw(raw).ok_or(ObjectiveError::UnknownValue {
			name: c"m_iInvalidCapWinner",
			value: raw,
		})
	}

	/// Disables the mini-round, so that the master does not pick it.
	#[doc(alias("Disable"))]
	pub fn disable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Disable", InputValue::Void)
	}

	/// Enables the mini-round.
	#[doc(alias("Enable"))]
	pub fn enable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Enable", InputValue::Void)
	}

	/// The mini-round's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Whether the mini-round is disabled (`m_bDisabled`).
	#[doc(alias("m_bDisabled", "StartDisabled"))]
	pub fn is_disabled(self) -> Result<bool, ObjectiveError> {
		self.0.bool_field(c"CTeamControlPointRound", c"m_bDisabled")
	}

	/// The names of the mini-round's points, separated by spaces
	/// (`m_iszCPNames`), as the round reads them when it spawns.
	#[doc(alias("m_iszCPNames", "cpr_cp_names"))]
	pub fn point_names(self) -> Result<CString, ObjectiveError> {
		self.0
			.string_key(c"CTeamControlPointRound", c"cpr_cp_names")
	}

	/// The mini-round's name, which the HUD shows (`m_iszPrintName`).
	#[doc(alias("m_iszPrintName", "cpr_printname"))]
	pub fn print_name(self) -> Result<CString, ObjectiveError> {
		self.0
			.string_key(c"CTeamControlPointRound", c"cpr_printname")
	}

	/// The mini-round's priority (`m_nPriority`): the master plays
	/// mini-rounds of higher priority first.
	#[doc(alias("m_nPriority", "cpr_priority"))]
	pub fn priority(self) -> Result<c_int, ObjectiveError> {
		self.0.int_field(c"CTeamControlPointRound", c"m_nPriority")
	}

	/// Sets which teams win the mini-round by owning all its points.
	#[doc(alias("cpr_restrict_team_cap_win"))]
	pub fn set_capture_wins(self, wins: CaptureWins) -> Result<(), ObjectiveError> {
		self.0
			.set_key_value(c"cpr_restrict_team_cap_win", &wins.to_key_value())
	}
}

/// The team number of `team`, or 0 for no team.
fn team_number(team: Option<ScoringTeam>) -> c_int {
	team.map_or(TEAM_UNASSIGNED, ScoringTeam::to_raw)
}
