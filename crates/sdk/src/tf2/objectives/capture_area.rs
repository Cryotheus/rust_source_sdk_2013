//! Capture areas (`trigger_capture_area`), where players capture control
//! points.

#[cfg(test)]
#[path = "../../tests/tf2/objectives/capture_area.rs"]
mod tests;

use super::{ControlPoint, Objective, ObjectiveError};
use crate::Server;
use crate::entities::Entity;
use crate::inputs::InputValue;
use crate::tf2::scoreboard::ScoringTeam;
use std::ffi::{CStr, CString};

/// A capture area (`trigger_capture_area`, `CTriggerAreaCapture`): the
/// trigger players stand in to capture its [control point](ControlPoint).
///
/// The area finds its point by name at each round's `RoundSpawn`, and copies
/// what it holds per team, such as whether the team may capture, to the
/// [objective resource](super::ObjectiveResource), which networks it.
#[doc(alias("trigger_capture_area", "CTriggerAreaCapture"))]
#[derive(Debug, Clone, Copy)]
pub struct CaptureArea<'s>(Objective<'s>);

impl<'s> CaptureArea<'s> {
	/// Wraps a capture area. Fails with [`ObjectiveError::WrongClass`] unless
	/// the server runs TF2 and `entity`'s data descriptions include
	/// `CTriggerAreaCapture`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(
			server,
			entity,
			c"CTriggerAreaCapture",
			"trigger_capture_area",
		)
		.map(Self)
	}

	/// Ends the capture in progress, if any, as completed by the capturing
	/// team.
	#[doc(alias("CaptureCurrentCP"))]
	pub fn capture_current_point(self) -> Result<(), ObjectiveError> {
		self.0.input(c"CaptureCurrentCP", InputValue::Void)
	}

	/// The seconds a capture takes (`m_flCapTime`), which the
	/// [objective resource](super::ObjectiveResource::capture_time) scales
	/// for each team.
	#[doc(alias("m_flCapTime", "area_time_to_cap"))]
	pub fn capture_time(self) -> Result<f32, ObjectiveError> {
		self.0.float_field(c"CTriggerAreaCapture", c"m_flCapTime")
	}

	/// The first control point named as the area's
	/// [point name](Self::control_point_name), or `None` if there is none.
	pub fn control_point(self) -> Result<Option<ControlPoint<'s>>, ObjectiveError> {
		let name = self.control_point_name()?;

		super::find_by_name(self.0.server(), &name, ControlPoint::new)
	}

	/// The name of the control point the area captures
	/// (`m_iszCapPointName`).
	#[doc(alias("m_iszCapPointName", "area_cap_point"))]
	pub fn control_point_name(self) -> Result<CString, ObjectiveError> {
		self.0.string_key(c"CTriggerAreaCapture", c"area_cap_point")
	}

	/// Disables the area: it stops being a trigger, and no player captures in
	/// it until it is enabled. The players in it leave it once the game next
	/// checks what entities touch, after they think in this frame or the
	/// next; [`Self::disable_and_end_touch`] has them leave at once.
	#[doc(alias("Disable"))]
	pub fn disable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Disable", InputValue::Void)
	}

	/// Has the players in the area leave it at once, as when they step out,
	/// and then [disables](Self::disable) it.
	#[doc(alias("DisableAndEndTouch", "EndTouch"))]
	pub fn disable_and_end_touch(self) -> Result<(), ObjectiveError> {
		self.0.input(c"DisableAndEndTouch", InputValue::Void)
	}

	/// Enables the area.
	#[doc(alias("Enable"))]
	pub fn enable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Enable", InputValue::Void)
	}

	/// The area's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Whether the area is disabled (`m_bDisabled`).
	#[doc(alias("m_bDisabled", "StartDisabled"))]
	pub fn is_disabled(self) -> Result<bool, ObjectiveError> {
		self.0.bool_field(c"CBaseTrigger", c"m_bDisabled")
	}

	/// Makes the area capture the control point named `name` instead,
	/// breaking the capture in progress.
	///
	/// The game's `SetControlPoint` keeps the name in a buffer that is gone
	/// once the input returns, so this sets it again as the area's
	/// `area_cap_point` key, which keeps its own copy.
	#[doc(alias("SetControlPoint", "area_cap_point"))]
	pub fn set_control_point(self, name: &CStr) -> Result<(), ObjectiveError> {
		self.0.input(c"SetControlPoint", InputValue::String(name))?;
		self.0.set_key_value(c"area_cap_point", name)
	}

	/// Sets whether `team` may capture the area's point, as the map's
	/// `team_cancap_` keys do.
	#[doc(alias("SetTeamCanCap", "team_cancap_2", "team_cancap_3"))]
	pub fn set_team_can_capture(self, team: ScoringTeam, can: bool) -> Result<(), ObjectiveError> {
		let value = CString::new(format!("{} {}", team.to_raw(), u8::from(can)))
			.expect("integers format without NUL bytes");

		self.0.input(c"SetTeamCanCap", InputValue::String(&value))
	}

	/// Enables the area if it is disabled, or disables it otherwise.
	#[doc(alias("Toggle"))]
	pub fn toggle(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Toggle", InputValue::Void)
	}
}
