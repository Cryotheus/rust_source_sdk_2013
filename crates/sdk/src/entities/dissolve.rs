//! Dissolving entities with the game's own `env_entity_dissolver`, which
//! fades them out with sparks before removing them.

#[cfg(test)]
#[path = "../tests/entities/dissolve.rs"]
mod tests;

use crate::entities::Entity;
use crate::entities::fields::FieldError;
use crate::entities::movement::EntityFlags;
use crate::entities::spawn::{EntitySpawn, SpawnError};
use crate::inputs::{InputError, InputValue};
use crate::interfaces::ServerTools;
use std::ffi::CString;

/// Why [`ServerTools::dissolve`] did not dissolve an entity.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DissolveError {
	/// The entity is dissolving already.
	#[error("the entity is dissolving already")]
	AlreadyDissolving,

	/// A member the dissolve reads is missing from the entity's datamaps.
	#[error(transparent)]
	Field(#[from] FieldError),

	/// The game refused the dissolver's `Dissolve` input.
	#[error(transparent)]
	Input(#[from] InputError),

	/// The entity has no model to dissolve: it is not a `CBaseAnimating`.
	#[error("the entity is not a CBaseAnimating, so has no model to dissolve")]
	NotAnimating,

	/// The dissolver could not be spawned.
	#[error(transparent)]
	Spawn(#[from] SpawnError),
}

/// How an entity dissolves (`ENTITY_DISSOLVE_*`), the dissolver's
/// `dissolvetype` key value.
#[doc(alias("dissolvetype"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DissolveType {
	/// `ENTITY_DISSOLVE_NORMAL`: fades out with sparks.
	#[doc(alias("ENTITY_DISSOLVE_NORMAL"))]
	#[default]
	Normal,

	/// `ENTITY_DISSOLVE_ELECTRICAL`: arcs with electricity, then dissolves.
	#[doc(alias("ENTITY_DISSOLVE_ELECTRICAL"))]
	Electrical,

	/// `ENTITY_DISSOLVE_ELECTRICAL_LIGHT`: arcs with less electricity, then
	/// dissolves.
	#[doc(alias("ENTITY_DISSOLVE_ELECTRICAL_LIGHT"))]
	ElectricalLight,

	/// `ENTITY_DISSOLVE_CORE`: dissolves into the dissolver, as the Citadel's
	/// core does.
	#[doc(alias("ENTITY_DISSOLVE_CORE"))]
	Core,
}

impl DissolveType {
	/// The type's `ENTITY_DISSOLVE_*` value.
	pub const fn to_raw(self) -> u8 {
		match self {
			Self::Normal => 0,
			Self::Electrical => 1,
			Self::ElectricalLight => 2,
			Self::Core => 3,
		}
	}
}

impl<'s> ServerTools<'s> {
	/// Dissolves `target`, which the game then removes, as an
	/// `env_entity_dissolver`'s `Dissolve` input does: a dissolver is spawned
	/// with `kind` and `magnitude` (250 by default), which clients' effects
	/// read, and is sent the input naming `target` as `!activator`, then
	/// removed, as the input makes another dissolver that follows `target`.
	///
	/// A player is not dissolved, but killed at once instead, as damage of
	/// its health (`DMG_GENERIC | DMG_REMOVENORAGDOLL`) would.
	///
	/// Fails if `target` is not a `CBaseAnimating`, which nothing dissolves, or
	/// is dissolving already.
	#[doc(alias("env_entity_dissolver", "Dissolve"))]
	pub fn dissolve(
		self,
		target: Entity<'_>,
		kind: DissolveType,
		magnitude: u8,
	) -> Result<(), DissolveError> {
		if !target.is_a(c"CBaseAnimating") {
			return Err(DissolveError::NotAnimating);
		}

		if target.flags()?.contains(EntityFlags::DISSOLVING) {
			return Err(DissolveError::AlreadyDissolving);
		}

		let spawn = EntitySpawn::new(c"env_entity_dissolver")
			.key(c"dissolvetype", &cstring(kind.to_raw()))
			.key(c"magnitude", &cstring(magnitude))
			.activate(false);

		// SAFETY: `CEntityDissolve`'s constructor and `Spawn`, which precaches
		// its sprite, free no entity, and it is not activated.
		let dissolver = unsafe { spawn.spawn(self) }?;

		let dissolved = self.accept_input(
			dissolver,
			c"Dissolve",
			InputValue::String(c"!activator"),
			target,
			target,
		);

		// The input made the dissolver that follows `target`, so this one is
		// done; a dissolver is no entity `remove` protects.
		let _ = self.remove(dissolver);

		dissolved.map_err(DissolveError::from)
	}
}

/// A number as a key value.
fn cstring(value: u8) -> CString {
	CString::new(value.to_string()).expect("numbers have no NUL")
}
