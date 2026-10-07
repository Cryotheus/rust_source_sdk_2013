//! Zones of a team: respawn rooms (`func_respawnroom`) and the walls that
//! keep enemies out of them (`func_respawnroomvisualizer`), resupply zones
//! (`func_regenerate`) and zones without buildings (`func_nobuild`).

#[cfg(test)]
#[path = "../../tests/tf2/objectives/zones.rs"]
mod tests;

use super::{Objective, ObjectiveError};
use crate::Server;
use crate::entities::Entity;
use crate::inputs::InputValue;
use crate::tf2::objects::ObjectKind;
use crate::tf2::scoreboard::ScoringTeam;
use std::ffi::{CStr, CString};

/// A zone without buildings (`func_nobuild`, `CFuncNoBuild`): Engineers of
/// the [team](Self::team) it denies cannot place there the buildings it
/// does not [allow](Self::allows).
#[doc(alias("func_nobuild", "CFuncNoBuild"))]
#[derive(Debug, Clone, Copy)]
pub struct NoBuildZone<'s>(Objective<'s>);

impl<'s> NoBuildZone<'s> {
	/// Wraps a zone without buildings. Fails with
	/// [`ObjectiveError::WrongClass`] unless the server runs TF2 and
	/// `entity`'s data descriptions include `CFuncNoBuild`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(server, entity, c"CFuncNoBuild", "func_nobuild").map(Self)
	}

	/// Whether the zone allows buildings of `kind` (`m_bAllowSentry`,
	/// `m_bAllowDispenser` and `m_bAllowTeleporters`).
	#[doc(alias("m_bAllowSentry", "m_bAllowDispenser", "m_bAllowTeleporters"))]
	pub fn allows(self, kind: ObjectKind) -> Result<bool, ObjectiveError> {
		let field = match kind {
			ObjectKind::Dispenser => c"m_bAllowDispenser",
			ObjectKind::Sentry => c"m_bAllowSentry",
			ObjectKind::Teleporter => c"m_bAllowTeleporters",
		};

		self.0.bool_field(c"CFuncNoBuild", field)
	}

	/// Whether [activating](Self::set_active) the zone destroys the buildings
	/// in it of the team it denies, whichever it
	/// [allows](Self::allows), except those the map placed
	/// (`m_bDestroyBuildingsOnActive`).
	#[doc(alias("m_bDestroyBuildingsOnActive", "DestroyBuildings"))]
	pub fn destroys_buildings(self) -> Result<bool, ObjectiveError> {
		self.0
			.bool_field(c"CFuncNoBuild", c"m_bDestroyBuildingsOnActive")
	}

	/// The zone's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Whether the zone is active, which it is unless `m_bDisabled`.
	#[doc(alias("m_bDisabled", "StartDisabled", "GetActive"))]
	pub fn is_active(self) -> Result<bool, ObjectiveError> {
		Ok(!self.0.bool_field(c"CBaseTrigger", c"m_bDisabled")?)
	}

	/// Activates the zone, which then [destroys](Self::destroys_buildings)
	/// the buildings in it if set to, or deactivates it.
	#[doc(alias("SetActive", "SetInactive"))]
	pub fn set_active(self, active: bool) -> Result<(), ObjectiveError> {
		let input = if active { c"SetActive" } else { c"SetInactive" };

		self.0.input(input, InputValue::Void)
	}

	/// Sets whether the zone allows buildings of `kind`.
	#[doc(alias("AllowSentry", "AllowDispenser", "AllowTeleporters"))]
	pub fn set_allows(self, kind: ObjectKind, allows: bool) -> Result<(), ObjectiveError> {
		let key = match kind {
			ObjectKind::Dispenser => c"AllowDispenser",
			ObjectKind::Sentry => c"AllowSentry",
			ObjectKind::Teleporter => c"AllowTeleporters",
		};

		self.0.set_key_value(key, flag_value(allows))
	}

	/// The team whose buildings the zone denies, or `None` for every team's
	/// (`m_iTeamNum`).
	#[doc(alias("m_iTeamNum", "TeamNum"))]
	pub fn team(self) -> Result<Option<ScoringTeam>, ObjectiveError> {
		team(self.0)
	}

	/// Deactivates the zone if it is active, or activates it otherwise.
	#[doc(alias("ToggleActive"))]
	pub fn toggle_active(self) -> Result<(), ObjectiveError> {
		self.0.input(c"ToggleActive", InputValue::Void)
	}
}

/// A resupply zone (`func_regenerate`, `CRegenerateZone`): the trigger of a
/// resupply locker, which refills the health and ammunition of players of
/// its [team](Self::team) and applies their loadout changes, at most every 3
/// seconds for each player.
///
/// The zone keeps whether it is disabled in a variable of its own, which its
/// data description does not declare, so it cannot be read.
#[doc(alias("func_regenerate", "CRegenerateZone", "resupply"))]
#[derive(Debug, Clone, Copy)]
pub struct RegenerateZone<'s>(Objective<'s>);

impl<'s> RegenerateZone<'s> {
	/// Wraps a resupply zone. Fails with [`ObjectiveError::WrongClass`]
	/// unless the server runs TF2 and `entity`'s data descriptions include
	/// `CRegenerateZone`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(server, entity, c"CRegenerateZone", "func_regenerate").map(Self)
	}

	/// Disables the zone, which no longer resupplies anyone.
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

	/// The team whose players the zone resupplies, or `None` for every
	/// team's (`m_iTeamNum`). Once a round is won, every zone resupplies the
	/// winners alone.
	#[doc(alias("m_iTeamNum", "TeamNum"))]
	pub fn team(self) -> Result<Option<ScoringTeam>, ObjectiveError> {
		team(self.0)
	}

	/// Enables the zone if it is disabled, or disables it otherwise.
	#[doc(alias("Toggle"))]
	pub fn toggle(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Toggle", InputValue::Void)
	}
}

/// A respawn room (`func_respawnroom`, `CFuncRespawnRoom`): the volume
/// where players of its [team](Self::team) change class and loadout on the
/// spot, and drop the flag they carry on entering, and where no Engineer
/// builds. Its [visualizers](RespawnRoomVisualizer) keep enemies out.
///
/// The room keeps whether it is active in a variable of its own, which its
/// data description does not declare, so it cannot be read.
#[doc(alias("func_respawnroom", "CFuncRespawnRoom", "spawn room"))]
#[derive(Debug, Clone, Copy)]
pub struct RespawnRoom<'s>(Objective<'s>);

impl<'s> RespawnRoom<'s> {
	/// Wraps a respawn room. Fails with [`ObjectiveError::WrongClass`]
	/// unless the server runs TF2 and `entity`'s data descriptions include
	/// `CFuncRespawnRoom`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(server, entity, c"CFuncRespawnRoom", "func_respawnroom").map(Self)
	}

	/// The room's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Activates the room, or deactivates it so that it counts as no respawn
	/// room. Its visualizers follow, and let anyone through while it is
	/// inactive.
	#[doc(alias("SetActive", "SetInactive"))]
	pub fn set_active(self, active: bool) -> Result<(), ObjectiveError> {
		let input = if active { c"SetActive" } else { c"SetInactive" };

		self.0.input(input, InputValue::Void)
	}

	/// The team the room belongs to, or `None` (`m_iTeamNum`). A room the map
	/// gives no team takes the team of the first enabled spawn point inside
	/// it as each round starts.
	#[doc(alias("m_iTeamNum", "TeamNum"))]
	pub fn team(self) -> Result<Option<ScoringTeam>, ObjectiveError> {
		team(self.0)
	}

	/// Deactivates the room if it is active, or activates it otherwise, as
	/// [`Self::set_active`] does.
	#[doc(alias("ToggleActive"))]
	pub fn toggle_active(self) -> Result<(), ObjectiveError> {
		self.0.input(c"ToggleActive", InputValue::Void)
	}
}

/// A respawn room visualizer (`func_respawnroomvisualizer`,
/// `CFuncRespawnRoomVisualizer`): the wall at a
/// [respawn room](RespawnRoom)'s door that its enemies see and cannot pass.
///
/// The visualizer finds its room by name, and takes its team, as each round
/// starts.
#[doc(alias("func_respawnroomvisualizer", "CFuncRespawnRoomVisualizer"))]
#[derive(Debug, Clone, Copy)]
pub struct RespawnRoomVisualizer<'s>(Objective<'s>);

impl<'s> RespawnRoomVisualizer<'s> {
	/// Wraps a respawn room visualizer. Fails with
	/// [`ObjectiveError::WrongClass`] unless the server runs TF2 and
	/// `entity`'s data descriptions include `CFuncRespawnRoomVisualizer`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(
			server,
			entity,
			c"CFuncRespawnRoomVisualizer",
			"func_respawnroomvisualizer",
		)
		.map(Self)
	}

	/// The visualizer's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Whether the visualizer was last set to stop enemies (`m_bSolid`). Its
	/// room's [activation](RespawnRoom::set_active) makes it solid or not too,
	/// without changing this.
	#[doc(alias("m_bSolid", "solid_to_enemies"))]
	pub fn is_solid(self) -> Result<bool, ObjectiveError> {
		self.0
			.bool_field(c"CFuncRespawnRoomVisualizer", c"m_bSolid")
	}

	/// The first respawn room named as the visualizer's
	/// [room name](Self::room_name), or `None` if there is none.
	pub fn room(self) -> Result<Option<RespawnRoom<'s>>, ObjectiveError> {
		let name = self.room_name()?;

		super::find_by_name(self.0.server(), &name, RespawnRoom::new)
	}

	/// The name of the visualizer's respawn room (`m_iszRespawnRoomName`).
	#[doc(alias("m_iszRespawnRoomName", "respawnroomname"))]
	pub fn room_name(self) -> Result<CString, ObjectiveError> {
		self.0
			.string_key(c"CFuncRespawnRoomVisualizer", c"respawnroomname")
	}

	/// Sets whether the visualizer stops enemies.
	#[doc(alias("SetSolid"))]
	pub fn set_solid(self, solid: bool) -> Result<(), ObjectiveError> {
		self.0.input(c"SetSolid", InputValue::Bool(solid))
	}

	/// The team whose enemies the visualizer stops, its room's, or `None`
	/// for none, which stops no one (`m_iTeamNum`).
	#[doc(alias("m_iTeamNum", "TeamNum"))]
	pub fn team(self) -> Result<Option<ScoringTeam>, ObjectiveError> {
		team(self.0)
	}
}

/// The key value of a boolean.
fn flag_value(value: bool) -> &'static CStr {
	if value { c"1" } else { c"0" }
}

/// The team of `objective`, or `None` for none (`m_iTeamNum`).
fn team(objective: Objective<'_>) -> Result<Option<ScoringTeam>, ObjectiveError> {
	Ok(ScoringTeam::from_raw(
		objective.int_field(c"CBaseEntity", c"m_iTeamNum")?,
	))
}
