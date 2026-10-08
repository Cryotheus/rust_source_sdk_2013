//! Flags (`item_teamflag`), and the zones players capture them in
//! (`func_capturezone`).

#[cfg(test)]
#[path = "../../tests/tf2/objectives/flags.rs"]
mod tests;

use super::{Objective, ObjectiveError};
use crate::Server;
use crate::entities::Entity;
use crate::inputs::InputValue;
use crate::tf2::scoreboard::ScoringTeam;
use sdk_raw::tf2::objectives as raw;
use std::ffi::c_int;

/// A flag (`item_teamflag`, `CCaptureFlag`): Capture the Flag's intelligence,
/// and the flags, cores and bombs of the modes built on it, which players
/// carry to a [capture zone](CaptureZone).
///
/// Who may pick a flag up depends on its [type](Self::flag_type) and its
/// [team](Self::team).
#[doc(alias("item_teamflag", "CCaptureFlag", "intelligence"))]
#[derive(Debug, Clone, Copy)]
pub struct CaptureFlag<'s>(Objective<'s>);

impl<'s> CaptureFlag<'s> {
	/// Wraps a flag. Fails with [`ObjectiveError::WrongClass`] unless the
	/// server runs TF2 and `entity`'s data descriptions include
	/// `CCaptureFlag`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(server, entity, c"CCaptureFlag", "item_teamflag").map(Self)
	}

	/// The player carrying the flag, or `None` while it is not
	/// [stolen](FlagStatus::Stolen).
	pub fn carrier(self) -> Result<Option<Entity<'s>>, ObjectiveError> {
		if self.status()? == FlagStatus::Stolen {
			self.last_carrier()
		} else {
			Ok(None)
		}
	}

	/// Disables the flag: players no longer pick it up, and a dropped flag no
	/// longer returns home. A disabled flag is hidden unless it is
	/// [visible when disabled](Self::is_visible_when_disabled), and its
	/// carrier, if any, keeps it.
	#[doc(alias("Disable"))]
	pub fn disable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Disable", InputValue::Void)
	}

	/// Enables the flag.
	#[doc(alias("Enable"))]
	pub fn enable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Enable", InputValue::Void)
	}

	/// The flag's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// The game mode the flag plays (`m_nType`).
	#[doc(alias("m_nType", "GameType", "GetType"))]
	pub fn flag_type(self) -> Result<FlagType, ObjectiveError> {
		let raw: c_int = self.0.get(c"m_nType")?;

		FlagType::from_raw(raw).ok_or(ObjectiveError::UnknownValue {
			name: c"m_nType",
			value: raw,
		})
	}

	/// Makes the flag's carrier, if any, drop it.
	#[doc(alias("ForceDrop"))]
	pub fn force_drop(self) -> Result<(), ObjectiveError> {
		self.0.input(c"ForceDrop", InputValue::Void)
	}

	/// Whether the flag is disabled (`m_bDisabled`).
	#[doc(alias("m_bDisabled", "StartDisabled", "IsDisabled"))]
	pub fn is_disabled(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bDisabled")
	}

	/// Whether the flag's glow, which players see through walls, is enabled
	/// (`m_bGlowEnabled`). Clients hide it in other cases too, such as from
	/// the team whose intelligence an enemy carries.
	#[doc(alias("m_bGlowEnabled", "IsGlowEnabled"))]
	pub fn is_glow_enabled(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bGlowEnabled")
	}

	/// Whether the flag stays visible, faded, while it is disabled
	/// (`m_bVisibleWhenDisabled`).
	#[doc(alias("m_bVisibleWhenDisabled", "VisibleWhenDisabled"))]
	pub fn is_visible_when_disabled(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bVisibleWhenDisabled")
	}

	/// The player carrying the flag, or who carried it last since it last
	/// returned home, or `None` (`m_hPrevOwner`).
	#[doc(alias("m_hPrevOwner", "GetPrevOwner"))]
	pub fn last_carrier(self) -> Result<Option<Entity<'s>>, ObjectiveError> {
		self.0.handle_entity(c"m_hPrevOwner")
	}

	/// The game time at which a dropped Invade flag goes back to the team it
	/// started with, or `None` if it is not counting down to it
	/// (`m_flNeutralTime`).
	#[doc(alias("m_flNeutralTime"))]
	pub fn neutral_time(self) -> Result<Option<f32>, ObjectiveError> {
		let time: f32 = self.0.get(c"m_flNeutralTime")?;

		Ok((time > 0.0).then_some(time))
	}

	/// The points the flag holds, as Robot and Player Destruction's flags do,
	/// which score them when captured (`m_nPointValue`).
	#[doc(alias("m_nPointValue", "PointValue", "GetPointValue"))]
	pub fn point_value(self) -> Result<c_int, ObjectiveError> {
		self.0.get(c"m_nPointValue")
	}

	/// Returns the flag home unless it is home already, making its carrier
	/// drop it first, with the announcements and the `OnReturn` output a
	/// return makes.
	///
	/// As any return does, this removes a Player Destruction flag instead,
	/// and gives the points a Robot Destruction core holds back to its team.
	#[doc(alias("ForceReset"))]
	pub fn reset(self) -> Result<(), ObjectiveError> {
		self.0.input(c"ForceReset", InputValue::Void)
	}

	/// Returns the flag home as [`Self::reset`] does, without the
	/// announcements or the `OnReturn` output.
	#[doc(alias("ForceResetSilent"))]
	pub fn reset_silently(self) -> Result<(), ObjectiveError> {
		self.0.input(c"ForceResetSilent", InputValue::Void)
	}

	/// The seconds a dropped flag waits before it returns home
	/// (`m_nReturnTime`). Robot and Player Destruction's flags wait as their
	/// modes decide instead, and the shot clock mode can shorten the wait.
	#[doc(alias("m_nReturnTime", "ReturnTime"))]
	pub fn return_delay(self) -> Result<c_int, ObjectiveError> {
		self.0.int_field(c"CCaptureFlag", c"m_nReturnTime")
	}

	/// The game time at which the dropped flag returns home, or `None` if it
	/// is not counting down to it (`m_flResetTime`). A
	/// [timer shown](Self::show_timer) on a flag that is not dropped counts
	/// down to it too.
	#[doc(alias("m_flResetTime"))]
	pub fn return_time(self) -> Result<Option<f32>, ObjectiveError> {
		let time: f32 = self.0.get(c"m_flResetTime")?;

		Ok((time > 0.0).then_some(time))
	}

	/// Enables or disables the flag's [glow](Self::is_glow_enabled).
	#[doc(alias("ForceGlowDisabled"))]
	pub fn set_glow_enabled(self, enabled: bool) -> Result<(), ObjectiveError> {
		self.0
			.input(c"ForceGlowDisabled", InputValue::Int(c_int::from(!enabled)))
	}

	/// Sets the seconds a dropped flag waits before it returns home, with
	/// negative seconds as 0, and restarts the countdown if the flag is
	/// dropped.
	#[doc(alias("SetReturnTime"))]
	pub fn set_return_delay(self, seconds: c_int) -> Result<(), ObjectiveError> {
		self.0.input(c"SetReturnTime", InputValue::Int(seconds))
	}

	/// Sets the [return delay](Self::return_delay) as
	/// [`Self::set_return_delay`] does, restarts the countdown wherever the
	/// flag is, and shows it above the flag. The flag returns home when the
	/// countdown ends only if it is dropped then; otherwise the countdown
	/// only disappears.
	#[doc(alias("ShowTimer"))]
	pub fn show_timer(self, seconds: c_int) -> Result<(), ObjectiveError> {
		self.0.input(c"ShowTimer", InputValue::Int(seconds))
	}

	/// Where the flag is (`m_nFlagStatus`).
	#[doc(alias("m_nFlagStatus"))]
	pub fn status(self) -> Result<FlagStatus, ObjectiveError> {
		let raw: c_int = self.0.get(c"m_nFlagStatus")?;

		FlagStatus::from_raw(raw).ok_or(ObjectiveError::UnknownValue {
			name: c"m_nFlagStatus",
			value: raw,
		})
	}

	/// The flag's team, or `None` for none (`m_iTeamNum`).
	///
	/// Players of other teams pick up Capture the Flag's intelligence and
	/// Robot Destruction's cores, and only the flag's own team picks up attack
	/// and defend and territory control flags. Invade and resource control
	/// flags are picked up by their own team, or by anyone while they have
	/// none, and take the team of the player who picks them up until they
	/// return home, or until a dropped Invade flag goes back to the team it
	/// started with.
	#[doc(alias("m_iTeamNum", "TeamNum"))]
	pub fn team(self) -> Result<Option<ScoringTeam>, ObjectiveError> {
		Ok(ScoringTeam::from_raw(
			self.0.int_field(c"CBaseEntity", c"m_iTeamNum")?,
		))
	}
}

/// A capture zone (`func_capturezone`, `CCaptureZone`): the trigger players
/// carrying a [flag](CaptureFlag) capture it in. In Player Destruction, the
/// zone scores the points of the flags carried in it one at a time instead.
#[doc(alias("func_capturezone", "CCaptureZone"))]
#[derive(Debug, Clone, Copy)]
pub struct CaptureZone<'s>(Objective<'s>);

impl<'s> CaptureZone<'s> {
	/// Wraps a capture zone. Fails with [`ObjectiveError::WrongClass`] unless
	/// the server runs TF2 and `entity`'s data descriptions include
	/// `CCaptureZone`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(server, entity, c"CCaptureZone", "func_capturezone").map(Self)
	}

	/// Disables the zone, in which flags are then not captured.
	#[doc(alias("Disable"))]
	pub fn disable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Disable", InputValue::Void)
	}

	/// Enables the zone.
	#[doc(alias("Enable"))]
	pub fn enable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Enable", InputValue::Void)
	}

	/// The zone's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Whether the zone is disabled (`m_bDisabled`).
	#[doc(alias("m_bDisabled", "StartDisabled", "IsDisabled"))]
	pub fn is_disabled(self) -> Result<bool, ObjectiveError> {
		self.0.flag(c"m_bDisabled")
	}

	/// The team whose players capture flags in the zone, or `None` for any
	/// team's (`m_iTeamNum`).
	#[doc(alias("m_iTeamNum", "TeamNum"))]
	pub fn team(self) -> Result<Option<ScoringTeam>, ObjectiveError> {
		Ok(ScoringTeam::from_raw(
			self.0.int_field(c"CBaseEntity", c"m_iTeamNum")?,
		))
	}
}

/// Where a [flag](CaptureFlag) is (`m_nFlagStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlagStatus {
	/// At its spawn point.
	#[doc(alias("TF_FLAGINFO_HOME"))]
	Home,

	/// Carried by a player.
	#[doc(alias("TF_FLAGINFO_STOLEN"))]
	Stolen,

	/// Dropped by its carrier, away from home.
	#[doc(alias("TF_FLAGINFO_DROPPED"))]
	Dropped,
}

impl FlagStatus {
	/// The status of this `TF_FLAGINFO_` value, or `None` for another value.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			raw::TF_FLAGINFO_HOME => Some(Self::Home),
			raw::TF_FLAGINFO_STOLEN => Some(Self::Stolen),
			raw::TF_FLAGINFO_DROPPED => Some(Self::Dropped),
			_ => None,
		}
	}

	/// The status's `TF_FLAGINFO_` value.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Home => raw::TF_FLAGINFO_HOME,
			Self::Stolen => raw::TF_FLAGINFO_STOLEN,
			Self::Dropped => raw::TF_FLAGINFO_DROPPED,
		}
	}
}

/// The game mode a [flag](CaptureFlag) plays (`ETFFlagType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlagType {
	/// Capture the Flag's intelligence.
	#[doc(alias("TF_FLAGTYPE_CTF"))]
	CaptureTheFlag,

	/// An attack and defend flag, which only the attackers carry.
	#[doc(alias("TF_FLAGTYPE_ATTACK_DEFEND"))]
	AttackDefend,

	/// A territory control flag.
	#[doc(alias("TF_FLAGTYPE_TERRITORY_CONTROL"))]
	TerritoryControl,

	/// An Invade flag, which turns neutral some time after it is dropped.
	#[doc(alias("TF_FLAGTYPE_INVADE"))]
	Invade,

	/// A resource control flag.
	#[doc(alias("TF_FLAGTYPE_RESOURCE_CONTROL"))]
	ResourceControl,

	/// Robot Destruction's reactor core.
	#[doc(alias("TF_FLAGTYPE_ROBOT_DESTRUCTION"))]
	RobotDestruction,

	/// Player Destruction's flag.
	#[doc(alias("TF_FLAGTYPE_PLAYER_DESTRUCTION"))]
	PlayerDestruction,
}

impl FlagType {
	/// The type of this `TF_FLAGTYPE_` value, or `None` for another value.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		Some(match raw {
			raw::TF_FLAGTYPE_CTF => Self::CaptureTheFlag,
			raw::TF_FLAGTYPE_ATTACK_DEFEND => Self::AttackDefend,
			raw::TF_FLAGTYPE_TERRITORY_CONTROL => Self::TerritoryControl,
			raw::TF_FLAGTYPE_INVADE => Self::Invade,
			raw::TF_FLAGTYPE_RESOURCE_CONTROL => Self::ResourceControl,
			raw::TF_FLAGTYPE_ROBOT_DESTRUCTION => Self::RobotDestruction,
			raw::TF_FLAGTYPE_PLAYER_DESTRUCTION => Self::PlayerDestruction,
			_ => return None,
		})
	}

	/// The type's `TF_FLAGTYPE_` value.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::CaptureTheFlag => raw::TF_FLAGTYPE_CTF,
			Self::AttackDefend => raw::TF_FLAGTYPE_ATTACK_DEFEND,
			Self::TerritoryControl => raw::TF_FLAGTYPE_TERRITORY_CONTROL,
			Self::Invade => raw::TF_FLAGTYPE_INVADE,
			Self::ResourceControl => raw::TF_FLAGTYPE_RESOURCE_CONTROL,
			Self::RobotDestruction => raw::TF_FLAGTYPE_ROBOT_DESTRUCTION,
			Self::PlayerDestruction => raw::TF_FLAGTYPE_PLAYER_DESTRUCTION,
		}
	}
}
