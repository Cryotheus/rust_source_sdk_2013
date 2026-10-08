//! Pickups placed in maps: health kits, ammo packs and the other items built
//! on `CTFPowerup`, which players pick up by touching them.
//!
//! [`Pickup`] reads a pickup's state and changes it through the inputs and
//! keys a map's logic uses, as the [objective](super::objectives) wrappers
//! do, and fails with the same [`ObjectiveError`]s. A round reset that resets
//! the map removes the map's pickups and spawns them again from the map, so
//! what a wrapper changes lasts until then.

#[cfg(test)]
#[path = "../tests/tf2/pickups.rs"]
mod tests;

use crate::Server;
use crate::entities::Entity;
use crate::inputs::InputValue;
use crate::tf2::objectives::{Objective, ObjectiveError};
use crate::tf2::scoreboard::ScoringTeam;

/// A pickup (`CTFPowerup`): a health kit (`item_healthkit_small`, `_medium`
/// and `_full`), an ammo pack (`item_ammopack_small`, `_medium` and
/// `_full`), or another item built on `CTFPowerup`, such as a Halloween spell
/// or a Mannpower rune.
///
/// A picked up pickup hides until its respawn delay passes, and then
/// [materializes](Self::auto_materializes) again.
#[doc(alias("CTFPowerup", "item_healthkit_full", "item_ammopack_full"))]
#[derive(Debug, Clone, Copy)]
pub struct Pickup<'s>(Objective<'s>);

impl<'s> Pickup<'s> {
	/// Wraps a pickup. Fails with [`ObjectiveError::WrongClass`] unless the
	/// server runs TF2 and `entity`'s data descriptions include
	/// `CTFPowerup`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ObjectiveError> {
		Objective::new(
			server,
			entity,
			c"CTFPowerup",
			"item_healthkit_* or item_ammopack_*",
		)
		.map(Self)
	}

	/// Whether the pickup comes back by itself once it respawns
	/// (`m_bAutoMaterialize`). One that does not stays hidden until it is
	/// [enabled](Self::enable).
	#[doc(alias("m_bAutoMaterialize", "AutoMaterialize"))]
	pub fn auto_materializes(self) -> Result<bool, ObjectiveError> {
		self.0.bool_field(c"CTFPowerup", c"m_bAutoMaterialize")
	}

	/// Disables the pickup, which hides it, and players no longer pick it
	/// up.
	#[doc(alias("Disable"))]
	pub fn disable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Disable", InputValue::Void)
	}

	/// Enables the pickup, which shows it again unless it is respawning. A
	/// respawning pickup that does not
	/// [materialize by itself](Self::auto_materializes) comes back at once.
	#[doc(alias("Enable"))]
	pub fn enable(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Enable", InputValue::Void)
	}

	/// The pickup's entity.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Whether the pickup is disabled (`m_bDisabled`).
	#[doc(alias("m_bDisabled", "StartDisabled", "IsDisabled"))]
	pub fn is_disabled(self) -> Result<bool, ObjectiveError> {
		self.0.bool_field(c"CTFPowerup", c"m_bDisabled")
	}

	/// Sets whether the pickup comes back by itself once it respawns,
	/// including a respawn under way.
	#[doc(alias("AutoMaterialize"))]
	pub fn set_auto_materialize(self, auto: bool) -> Result<(), ObjectiveError> {
		self.0
			.set_key_value(c"AutoMaterialize", if auto { c"1" } else { c"0" })
	}

	/// The team whose players alone pick the pickup up, or `None` for every
	/// team (`m_iTeamNum`).
	#[doc(alias("m_iTeamNum", "TeamNum"))]
	pub fn team(self) -> Result<Option<ScoringTeam>, ObjectiveError> {
		Ok(ScoringTeam::from_raw(
			self.0.int_field(c"CBaseEntity", c"m_iTeamNum")?,
		))
	}

	/// Enables the pickup if it is disabled, or disables it otherwise.
	#[doc(alias("Toggle"))]
	pub fn toggle(self) -> Result<(), ObjectiveError> {
		self.0.input(c"Toggle", InputValue::Void)
	}
}
