//! Train watchers (`team_train_watcher`), which track payload carts.

#[cfg(test)]
#[path = "../../tests/tf2/objectives/train_watcher.rs"]
mod tests;

use super::{CaptureArea, Objective, ObjectiveError};
use crate::Server;
use crate::entities::Entity;
use crate::inputs::InputValue;
use std::ffi::{CString, c_int};

/// A train watcher (`team_train_watcher`, `CTeamTrainWatcher`): the logic
/// entity that tracks a payload cart, a `func_tracktrain`, along its path,
/// networks its progress and speed to the HUD, and counts down to rolling it
/// back when no one pushes it.
///
/// A watcher that [moves its train](Self::handles_train_movement) also sets
/// the train's speed from the players pushing it, which its capture area
/// [counts](Self::cappers). Maps that do not let it move the train do so
/// with their own logic, from the watcher's and the capture area's outputs.
#[doc(alias("team_train_watcher", "CTeamTrainWatcher", "payload", "cart"))]
#[derive(Debug, Clone, Copy)]
pub struct TrainWatcher<'s>(Objective<'s>);

impl<'s> TrainWatcher<'s> {
	/// Wraps a train watcher. Fails with [`ObjectiveError::WrongClass`] unless
	/// the server runs TF2 and `entity`'s data descriptions include
	/// `CTeamTrainWatcher`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(server, entity, c"CTeamTrainWatcher", "team_train_watcher").map(Self)
	}

	/// Whether the train may roll back when no one pushes it
	/// (`m_bTrainCanRecede`).
	#[doc(alias("m_bTrainCanRecede", "train_can_recede"))]
	pub fn can_recede(self) -> Result<bool, ObjectiveError> {
		self.0
			.bool_field(c"CTeamTrainWatcher", c"m_bTrainCanRecede")
	}

	/// The players pushing the train, as its capture area last counted them,
	/// or -1 while enemies block it, which holds the train still
	/// (`m_nNumCappers`).
	#[doc(alias("m_nNumCappers", "GetCapperCount"))]
	pub fn cappers(self) -> Result<c_int, ObjectiveError> {
		self.0.get(c"m_nNumCappers")
	}

	/// Disables the watcher: it stops tracking the train and networking to
	/// clients, cancels a pending roll back, and stops the train if it
	/// [moves it](Self::handles_train_movement).
	#[doc(alias("Disable"))]
	pub fn disable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Disable", InputValue::Void)
	}

	/// Enables the watcher, which finds its train and path again.
	#[doc(alias("Enable"))]
	pub fn enable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Enable", InputValue::Void)
	}

	/// The watcher's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Whether the watcher sets the train's speed itself
	/// (`m_bHandleTrainMovement`), rather than leaving it to the map's logic.
	#[doc(alias("m_bHandleTrainMovement", "handle_train_movement"))]
	pub fn handles_train_movement(self) -> Result<bool, ObjectiveError> {
		self.0
			.bool_field(c"CTeamTrainWatcher", c"m_bHandleTrainMovement")
	}

	/// Whether the watcher is disabled (`m_bDisabled`).
	#[doc(alias("m_bDisabled", "StartDisabled", "IsDisabled"))]
	pub fn is_disabled(self) -> Result<bool, ObjectiveError> {
		self.0.bool_field(c"CTeamTrainWatcher", c"m_bDisabled")
	}

	/// How far the train has come along its path, from 0 at its start to 1
	/// at its goal (`m_flTotalProgress`).
	#[doc(alias("m_flTotalProgress", "GetTrainProgress"))]
	pub fn progress(self) -> Result<f32, ObjectiveError> {
		self.0.get(c"m_flTotalProgress")
	}

	/// The seconds the train waits, once no one pushes it, before it rolls
	/// back (`m_nTrainRecedeTime`), or `None` for `tf_escort_recede_time`'s.
	/// In overtime it waits `tf_escort_recede_time_overtime`'s instead.
	#[doc(alias("m_nTrainRecedeTime", "train_recede_time"))]
	pub fn recede_delay(self) -> Result<Option<c_int>, ObjectiveError> {
		let seconds = self
			.0
			.int_field(c"CTeamTrainWatcher", c"m_nTrainRecedeTime")?;

		Ok((seconds > 0).then_some(seconds))
	}

	/// The game time at which the train starts rolling back, or `None` if it
	/// is not counting down to it (`m_flRecedeTime`).
	#[doc(alias("m_flRecedeTime"))]
	pub fn recede_time(self) -> Result<Option<f32>, ObjectiveError> {
		let time: f32 = self.0.get(c"m_flRecedeTime")?;

		Ok((time > 0.0).then_some(time))
	}

	/// Sets whether the train may roll back when no one pushes it. A
	/// countdown already started still ends in a roll back.
	#[doc(alias("SetTrainCanRecede"))]
	pub fn set_can_recede(self, can: bool) -> Result<(), ObjectiveError> {
		self.0.input(c"SetTrainCanRecede", InputValue::Bool(can))
	}

	/// Sets the [players pushing the train](Self::cappers), as `area`, the
	/// train's capture area, does as players come and go, or as the map's
	/// logic does with the area's outputs. Pass the area so the watcher reads
	/// whether enemies block it, which keeps the train from rolling back.
	///
	/// Ignored while the watcher is disabled. The area sets the count again
	/// whenever its own count changes.
	#[doc(alias("SetNumTrainCappers"))]
	pub fn set_cappers(
		self,
		cappers: c_int,
		area: Option<CaptureArea<'_>>,
	) -> Result<(), ObjectiveError> {
		let caller = area.map_or(self.0.entity(), CaptureArea::entity);

		self.0.input_from(
			c"SetNumTrainCappers",
			InputValue::Int(cappers),
			caller,
			caller,
		)
	}

	/// Sets the [recede delay](Self::recede_delay), with 0 or less for
	/// `tf_escort_recede_time`'s, for the next countdown.
	#[doc(alias("SetTrainRecedeTime"))]
	pub fn set_recede_delay(self, seconds: c_int) -> Result<(), ObjectiveError> {
		self.0
			.input(c"SetTrainRecedeTime", InputValue::Int(seconds))
	}

	/// Sets the [recede delay](Self::recede_delay) as
	/// [`Self::set_recede_delay`] does, and restarts the countdown with it if
	/// one is running.
	#[doc(alias("SetTrainRecedeTimeAndUpdate"))]
	pub fn set_recede_delay_and_restart(self, seconds: c_int) -> Result<(), ObjectiveError> {
		self.0
			.input(c"SetTrainRecedeTimeAndUpdate", InputValue::Int(seconds))
	}

	/// Sets the [forward speed modifier](Self::speed_forward_modifier), as
	/// its absolute value clamped to 0 to 1. Ignored while the watcher is
	/// disabled or does not [move its train](Self::handles_train_movement).
	#[doc(alias("SetSpeedForwardModifier"))]
	pub fn set_speed_forward_modifier(self, modifier: f32) -> Result<(), ObjectiveError> {
		self.0
			.input(c"SetSpeedForwardModifier", InputValue::Float(modifier))
	}

	/// The share of its speed the train keeps when it moves forward, from 0
	/// to 1 (`m_flSpeedForwardModifier`).
	#[doc(alias("m_flSpeedForwardModifier"))]
	pub fn speed_forward_modifier(self) -> Result<f32, ObjectiveError> {
		self.0
			.float_field(c"CTeamTrainWatcher", c"m_flSpeedForwardModifier")
	}

	/// The train's speed as the HUD shows it (`m_iTrainSpeedLevel`): -1 while
	/// it rolls back, 0 while it stands, or 1 to 3 as it moves forward faster
	/// than the map's `hud_min_speed_level_` keys.
	#[doc(alias("m_iTrainSpeedLevel"))]
	pub fn speed_level(self) -> Result<c_int, ObjectiveError> {
		self.0.get(c"m_iTrainSpeedLevel")
	}

	/// The first entity named as the watcher's [train](Self::train_name), or
	/// `None` if there is none.
	pub fn train(self) -> Result<Option<Entity<'s>>, ObjectiveError> {
		let name = self.train_name()?;

		super::find_by_name(self.0.server(), &name, |_, entity| Ok(entity))
	}

	/// The name of the train the watcher tracks (`m_iszTrain`), which it
	/// finds when it is enabled or the round starts.
	#[doc(alias("m_iszTrain", "train"))]
	pub fn train_name(self) -> Result<CString, ObjectiveError> {
		self.0.string_key(c"CTeamTrainWatcher", c"train")
	}
}
