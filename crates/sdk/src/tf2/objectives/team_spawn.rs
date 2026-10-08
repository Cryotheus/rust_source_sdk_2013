//! Team spawn points (`info_player_teamspawn`).

#[cfg(test)]
#[path = "../../tests/tf2/objectives/team_spawn.rs"]
mod tests;

use super::{Objective, ObjectiveError};
use crate::Server;
use crate::entities::Entity;
use crate::inputs::InputValue;
use crate::tf2::scoreboard::ScoringTeam;
use sdk_raw::tf2::objectives as raw;
use std::ffi::c_int;

/// A team spawn point (`info_player_teamspawn`, `CTFTeamSpawn`), where
/// players of its [team](Self::team) spawn while it is enabled.
#[doc(alias("info_player_teamspawn", "CTFTeamSpawn", "spawn point"))]
#[derive(Debug, Clone, Copy)]
pub struct TeamSpawn<'s>(Objective<'s>);

impl<'s> TeamSpawn<'s> {
	/// Wraps a team spawn point. Fails with [`ObjectiveError::WrongClass`]
	/// unless the server runs TF2 and `entity`'s data descriptions include
	/// `CTFTeamSpawn`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(server, entity, c"CTFTeamSpawn", "info_player_teamspawn").map(Self)
	}

	/// Disables the spawn point, where no one spawns until it is enabled.
	#[doc(alias("Disable"))]
	pub fn disable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Disable", InputValue::Void)
	}

	/// Enables the spawn point.
	#[doc(alias("Enable"))]
	pub fn enable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Enable", InputValue::Void)
	}

	/// The spawn point's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Whether the spawn point is disabled (`m_bDisabled`).
	#[doc(alias("m_bDisabled", "StartDisabled", "IsDisabled"))]
	pub fn is_disabled(self) -> Result<bool, ObjectiveError> {
		self.0.bool_field(c"CTFTeamSpawn", c"m_bDisabled")
	}

	/// When players spawn at the spawn point (`m_nSpawnMode`).
	#[doc(alias("m_nSpawnMode", "SpawnMode", "GetTeamSpawnMode"))]
	pub fn mode(self) -> Result<TeamSpawnMode, ObjectiveError> {
		let raw = self.0.int_field(c"CTFTeamSpawn", c"m_nSpawnMode")?;

		TeamSpawnMode::from_raw(raw).ok_or(ObjectiveError::UnknownValue {
			name: c"m_nSpawnMode",
			value: raw,
		})
	}

	/// The team that spawns at the spawn point, or `None` for none
	/// (`m_iTeamNum`).
	#[doc(alias("m_iTeamNum", "TeamNum"))]
	pub fn team(self) -> Result<Option<ScoringTeam>, ObjectiveError> {
		Ok(ScoringTeam::from_raw(
			self.0.int_field(c"CBaseEntity", c"m_iTeamNum")?,
		))
	}
}

/// When players spawn at a [team spawn point](TeamSpawn)
/// (`PlayerTeamSpawnMode_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeamSpawnMode {
	/// Whenever players of its team spawn, including where a
	/// `trigger_player_respawn_override` names it.
	#[doc(alias("PlayerTeamSpawnMode_Normal"))]
	Normal,

	/// Only where a `trigger_player_respawn_override` names it.
	#[doc(alias("PlayerTeamSpawnMode_Triggered"))]
	Triggered,
}

impl TeamSpawnMode {
	/// The mode of this `PlayerTeamSpawnMode_t` value, or `None` for another
	/// value.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			raw::PLAYER_TEAM_SPAWN_MODE_NORMAL => Some(Self::Normal),
			raw::PLAYER_TEAM_SPAWN_MODE_TRIGGERED => Some(Self::Triggered),
			_ => None,
		}
	}

	/// The mode's `PlayerTeamSpawnMode_t` value.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Normal => raw::PLAYER_TEAM_SPAWN_MODE_NORMAL,
			Self::Triggered => raw::PLAYER_TEAM_SPAWN_MODE_TRIGGERED,
		}
	}
}
