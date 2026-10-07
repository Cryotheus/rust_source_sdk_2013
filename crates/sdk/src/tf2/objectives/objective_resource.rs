//! TF2's objective resource (`tf_objective_resource`), which networks the
//! state of the map's control points.

#[cfg(test)]
#[path = "../../tests/tf2/objectives/objective_resource.rs"]
mod tests;

use super::{Objective, ObjectiveError, RoundTimer};
use crate::Server;
use crate::entities::Entity;
use crate::math::Vector;
use crate::tf2::scoreboard::ScoringTeam;
use sdk_raw::inputs::MAX_CONTROL_POINTS;
use std::ffi::{CStr, c_int};

/// The objective resource (`tf_objective_resource`, `CTFObjectiveResource`):
/// the entity that networks the state of the map's control points to
/// clients, whose HUD shows the points as it describes them.
///
/// The game keeps one. The control point master fills it in for the points
/// of each round, numbering them from 0 by their `point_index`, as
/// [`ControlPoint::index`](super::ControlPoint::index) reads, and the points
/// and their capture areas update it as their state changes. What it holds
/// of a point is a copy, which the HUD shows: the point and its capture area
/// decide what happens.
///
/// Methods taking a point fail with [`ObjectiveError::PointOutOfRange`] for
/// an index of [`MAX_CONTROL_POINTS`] or more. Points from the
/// [point count](Self::point_count) on hold what an earlier round left
/// there, or zeroes.
#[doc(alias(
	"tf_objective_resource",
	"CTFObjectiveResource",
	"CBaseTeamObjectiveResource"
))]
#[derive(Debug, Clone, Copy)]
pub struct ObjectiveResource<'s>(Objective<'s>);

impl<'s> ObjectiveResource<'s> {
	/// Wraps the objective resource. Fails with
	/// [`ObjectiveError::WrongClass`] unless the server runs TF2 and
	/// `entity`'s data descriptions include `CBaseTeamObjectiveResource`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(
			server,
			entity,
			c"CBaseTeamObjectiveResource",
			"tf_objective_resource",
		)
		.map(Self)
	}

	/// Finds the objective resource, or `None` if there is none, as before
	/// the map's entities spawn or on a server not running TF2.
	pub fn find(server: Server<'s>) -> Result<Option<Self>, ObjectiveError> {
		super::find_by_class(server, c"tf_objective_resource", Self::new)
	}

	/// How many players of `team` stand in `point`'s capture area
	/// (`m_iNumTeamMembers`), as the capture area counts them.
	#[doc(alias("m_iNumTeamMembers", "GetNumPlayersInArea"))]
	pub fn cappers_in_zone(self, point: usize, team: ScoringTeam) -> Result<c_int, ObjectiveError> {
		self.0
			.element(c"m_iNumTeamMembers", team_index(point, team)?)
	}

	/// The team capturing `point`, or `None` if no team is
	/// (`m_iCappingTeam`).
	#[doc(alias("m_iCappingTeam", "GetCappingTeam"))]
	pub fn capping_team(self, point: usize) -> Result<Option<ScoringTeam>, ObjectiveError> {
		let team = self.0.element(c"m_iCappingTeam", point_index(point)?)?;

		Ok(ScoringTeam::from_raw(team))
	}

	/// The part of `point`'s capture still to go, from 1 when it starts to 0
	/// when it completes, or 0 while no team is capturing it
	/// (`m_flCapPercentages`, not networked).
	///
	/// Clients receive it only as the
	/// [lazy progress](Self::lazy_capture_left).
	#[doc(alias("m_flCapPercentages", "GetCPCapPercentage"))]
	pub fn capture_left(self, point: usize) -> Result<f32, ObjectiveError> {
		self.0.float_element(
			c"CBaseTeamObjectiveResource",
			c"m_flCapPercentages",
			point_index(point)?,
		)
	}

	/// The capture time `point`'s capture area gives the HUD for `team`, in
	/// seconds (`m_flTeamCapTime`): the area's own
	/// [capture time](super::CaptureArea::capture_time), or, with
	/// `mp_capstyle` set, twice that for each
	/// [required capper](Self::required_cappers).
	#[doc(alias("m_flTeamCapTime", "GetCPCapTime"))]
	pub fn capture_time(self, point: usize, team: ScoringTeam) -> Result<f32, ObjectiveError> {
		self.0.element(c"m_flTeamCapTime", team_index(point, team)?)
	}

	/// The objective resource's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// The group the map puts `point` in, which the HUD lays out together
	/// (`m_iCPGroup`).
	#[doc(alias("m_iCPGroup", "GetCPGroup", "point_group"))]
	pub fn group(self, point: usize) -> Result<c_int, ObjectiveError> {
		self.0.element(c"m_iCPGroup", point_index(point)?)
	}

	/// The round timer the HUD shows, or `None` if it shows none
	/// (`m_iTimerToShowInHUD`).
	#[doc(alias("m_iTimerToShowInHUD", "GetTimerToShowInHUD"))]
	pub fn hud_timer(self) -> Result<Option<RoundTimer<'s>>, ObjectiveError> {
		self.timer(c"m_iTimerToShowInHUD")
	}

	/// Whether `point`'s capture is blocked, because players of both teams
	/// stand on it while one captures it (`m_bBlocked`).
	#[doc(alias("m_bBlocked", "CapIsBlocked"))]
	pub fn is_blocked(self, point: usize) -> Result<bool, ObjectiveError> {
		self.0.flag_element(c"m_bBlocked", point_index(point)?)
	}

	/// Whether `point` is part of the mini-round being played, or of every
	/// round on a map without mini-rounds (`m_bInMiniRound`).
	#[doc(alias("m_bInMiniRound", "IsInMiniRound"))]
	pub fn is_in_mini_round(self, point: usize) -> Result<bool, ObjectiveError> {
		self.0.flag_element(c"m_bInMiniRound", point_index(point)?)
	}

	/// Whether `point` is locked, so that no team can capture it
	/// (`m_bCPLocked`).
	#[doc(alias("m_bCPLocked", "GetCPLocked"))]
	pub fn is_locked(self, point: usize) -> Result<bool, ObjectiveError> {
		self.0.flag_element(c"m_bCPLocked", point_index(point)?)
	}

	/// Whether the HUD shows `point` (`m_bCPIsVisible`), unless the point has
	/// the spawn flag [`SF_CAP_POINT_HIDE_FLAG`].
	///
	/// [`SF_CAP_POINT_HIDE_FLAG`]: sdk_raw::tf2::objectives::SF_CAP_POINT_HIDE_FLAG
	#[doc(alias("m_bCPIsVisible", "IsCPVisible"))]
	pub fn is_visible(self, point: usize) -> Result<bool, ObjectiveError> {
		self.0.flag_element(c"m_bCPIsVisible", point_index(point)?)
	}

	/// The part of `point`'s capture still to go, as
	/// [`capture_left`](Self::capture_left) gives it, updated every 3 seconds
	/// for clients (`m_flLazyCapPerc`).
	#[doc(alias("m_flLazyCapPerc"))]
	pub fn lazy_capture_left(self, point: usize) -> Result<f32, ObjectiveError> {
		self.0.element(c"m_flLazyCapPerc", point_index(point)?)
	}

	/// The team owning `point`, or `None` if no team does (`m_iOwner`).
	#[doc(alias("m_iOwner", "GetOwningTeam"))]
	pub fn owner(self, point: usize) -> Result<Option<ScoringTeam>, ObjectiveError> {
		let team = self.0.element(c"m_iOwner", point_index(point)?)?;

		Ok(ScoringTeam::from_raw(team))
	}

	/// How far along the payload's track `point` lies, from 0 at its start to
	/// 1 at its end, as a train watcher sets it for the HUD's track
	/// (`m_flPathDistance`).
	#[doc(alias("m_flPathDistance", "GetPathDistance"))]
	pub fn path_distance(self, point: usize) -> Result<f32, ObjectiveError> {
		self.0.element(c"m_flPathDistance", point_index(point)?)
	}

	/// Whether the map plays its points in mini-rounds, as with
	/// `team_control_point_round` (`m_bPlayingMiniRounds`).
	#[doc(alias("m_bPlayingMiniRounds", "PlayingMiniRounds"))]
	pub fn plays_mini_rounds(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bPlayingMiniRounds")
	}

	/// How many control points the round has (`m_iNumControlPoints`), which
	/// are numbered from 0. It is never more than [`MAX_CONTROL_POINTS`].
	#[doc(alias("m_iNumControlPoints", "GetNumControlPoints"))]
	pub fn point_count(self) -> Result<usize, ObjectiveError> {
		let count: c_int = self.0.get(c"m_iNumControlPoints")?;

		Ok(usize::try_from(count.clamp(0, MAX_CONTROL_POINTS)).unwrap_or(0))
	}

	/// Where `point` stands (`m_vCPPositions`).
	#[doc(alias("m_vCPPositions", "GetCPPosition"))]
	pub fn position(self, point: usize) -> Result<Vector, ObjectiveError> {
		self.0.element(c"m_vCPPositions", point_index(point)?)
	}

	/// How many players of `team` it takes to capture `point`
	/// (`m_iTeamReqCappers`), as its capture area's `team_numcap_` key sets
	/// it.
	#[doc(alias("m_iTeamReqCappers", "GetRequiredCappers"))]
	pub fn required_cappers(
		self,
		point: usize,
		team: ScoringTeam,
	) -> Result<c_int, ObjectiveError> {
		self.0
			.element(c"m_iTeamReqCappers", team_index(point, team)?)
	}

	/// The round timer tournament stopwatch mode times rounds with, or `None`
	/// outside stopwatch mode (`m_iStopWatchTimer`).
	#[doc(alias("m_iStopWatchTimer", "GetStopWatchTimer"))]
	pub fn stopwatch_timer(self) -> Result<Option<RoundTimer<'s>>, ObjectiveError> {
		self.timer(c"m_iStopWatchTimer")
	}

	/// Whether `team` may capture `point` (`m_bTeamCanCap`), as its capture
	/// area's `team_cancap_` key and
	/// [`CaptureArea::set_team_can_capture`](super::CaptureArea::set_team_can_capture)
	/// set it.
	#[doc(alias("m_bTeamCanCap", "TeamCanCapPoint"))]
	pub fn team_can_capture(self, point: usize, team: ScoringTeam) -> Result<bool, ObjectiveError> {
		self.0
			.flag_element(c"m_bTeamCanCap", team_index(point, team)?)
	}

	/// The team whose players stand on `point`, or `None` if none do or both
	/// teams' do (`m_iTeamInZone`).
	#[doc(alias("m_iTeamInZone", "GetTeamInZone"))]
	pub fn team_in_zone(self, point: usize) -> Result<Option<ScoringTeam>, ObjectiveError> {
		let team = self.0.element(c"m_iTeamInZone", point_index(point)?)?;

		Ok(ScoringTeam::from_raw(team))
	}

	/// The round timer at the edict index in the networked variable `name`,
	/// or `None` if it holds 0 or an index without a round timer.
	fn timer(self, name: &CStr) -> Result<Option<RoundTimer<'s>>, ObjectiveError> {
		let index: c_int = self.0.get(name)?;

		if index <= 0 {
			return Ok(None);
		}

		let server = self.0.server();

		Ok(server
			.server_tools()?
			.entity_by_index(index)
			.and_then(|entity| RoundTimer::new(server, entity).ok()))
	}

	/// The game time at which `point`'s own timer runs out, as
	/// `tf_logic_cp_timer` sets it, or -1 or 0 without one
	/// (`m_flCPTimerTimes`).
	#[doc(alias("m_flCPTimerTimes", "GetCPTimerTime"))]
	pub fn timer_end(self, point: usize) -> Result<f32, ObjectiveError> {
		self.0.element(c"m_flCPTimerTimes", point_index(point)?)
	}

	/// The game time at which `point` unlocks, or 0 if it is not waiting to
	/// (`m_flUnlockTimes`), as
	/// [`ControlPoint::set_unlock_time`](super::ControlPoint::set_unlock_time)
	/// sets it.
	#[doc(alias("m_flUnlockTimes", "GetCPUnlockTime"))]
	pub fn unlock_time(self, point: usize) -> Result<f32, ObjectiveError> {
		self.0.element(c"m_flUnlockTimes", point_index(point)?)
	}
}

/// `point`, if it is below [`MAX_CONTROL_POINTS`].
fn point_index(point: usize) -> Result<usize, ObjectiveError> {
	if point < MAX_CONTROL_POINTS as usize {
		Ok(point)
	} else {
		Err(ObjectiveError::PointOutOfRange { index: point })
	}
}

/// The index of `team`'s entry for `point` in the per-team arrays, which
/// hold [`MAX_CONTROL_POINTS`] entries per team number (`TEAM_ARRAY`).
fn team_index(point: usize, team: ScoringTeam) -> Result<usize, ObjectiveError> {
	let team = usize::try_from(team.to_raw()).expect("TF2's teams are positive");

	Ok(point_index(point)? + team * MAX_CONTROL_POINTS as usize)
}
