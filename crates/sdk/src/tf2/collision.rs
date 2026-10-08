//! TF2's collision groups: those every game shares, and TF2's own, which its
//! game rules (`CTFGameRules::ShouldCollide`) add for its projectiles,
//! buildings and respawn rooms.
//!
//! [`Entity::collision_group`] reads an entity's group as a number, which
//! [`TfCollisionGroup::of`] converts. [`sdk_raw::tf2::collision`] holds TF2's
//! numbers.

#[cfg(test)]
#[path = "../tests/tf2/collision.rs"]
mod tests;

use crate::entities::{CollisionGroup, Entity};
use sdk_raw::tf2::collision as raw;
use std::ffi::c_int;

/// A collision group as TF2 numbers them: one every game shares, or one TF2
/// adds past them (`TFCOLLISION_GROUP_*`), which decides what an entity
/// collides with, and which traces hit it.
#[doc(alias("Collision_Group_t", "TFCOLLISION_GROUP"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TfCollisionGroup {
	/// Players and their movement pass through it, but bullets and
	/// projectiles hit it, as the Medic's shield and sappers are.
	#[doc(alias("TFCOLLISION_GROUP_COMBATOBJECT"))]
	CombatObject,

	/// Grenades and stickybombs, which collide with neither players nor each
	/// other, nor rockets, and handle hitting players themselves while they
	/// fly.
	#[doc(alias("TF_COLLISIONGROUP_GRENADES"))]
	Grenades,

	/// A building that blocks the movement of the other team's players only,
	/// and pushes its own team's players away. A building is in this group
	/// unless it is solid to players, which its `SolidToPlayer` key, or else
	/// its kind's defaults in `scripts/objects.txt`, decide.
	#[doc(alias("TFCOLLISION_GROUP_OBJECT"))]
	Object,

	/// A building that blocks the movement of every player.
	#[doc(alias("TFCOLLISION_GROUP_OBJECT_SOLIDTOPLAYERMOVEMENT"))]
	ObjectSolidToPlayerMovement,

	/// Respawn room brushes, which collide with players and their movement
	/// only.
	#[doc(alias("TFCOLLISION_GROUP_RESPAWNROOMS"))]
	RespawnRooms,

	/// Rockets and the like, which hit players but not their movement, and
	/// pass through weapons, grenades and projectiles of the shared group.
	#[doc(alias("TFCOLLISION_GROUP_ROCKETS"))]
	Rockets,

	/// As [`Self::Rockets`], and also passes through rockets of either group,
	/// as arrows do.
	#[doc(alias("TFCOLLISION_GROUP_ROCKET_BUT_NOT_WITH_OTHER_ROCKETS"))]
	RocketsNotWithRockets,

	/// One of the groups every game shares.
	Shared(CollisionGroup),

	/// Players and their movement pass through it, as they do through a team's
	/// pumpkin bombs.
	#[doc(alias("TFCOLLISION_GROUP_TANK"))]
	Tank,
}

impl TfCollisionGroup {
	/// TF2's own groups, in the order it numbers them.
	pub const OWN: [Self; 8] = [
		Self::Grenades,
		Self::Object,
		Self::ObjectSolidToPlayerMovement,
		Self::CombatObject,
		Self::Rockets,
		Self::RespawnRooms,
		Self::Tank,
		Self::RocketsNotWithRockets,
	];

	/// Converts a group TF2 numbers `value`, shared or its own, or returns
	/// `None` for a number it gives no group.
	pub const fn from_raw(value: c_int) -> Option<Self> {
		if let Some(shared) = CollisionGroup::from_raw(value) {
			return Some(Self::Shared(shared));
		}

		Some(match value {
			raw::TF_COLLISIONGROUP_GRENADES => Self::Grenades,
			raw::TFCOLLISION_GROUP_OBJECT => Self::Object,

			raw::TFCOLLISION_GROUP_OBJECT_SOLIDTOPLAYERMOVEMENT => {
				Self::ObjectSolidToPlayerMovement
			}

			raw::TFCOLLISION_GROUP_COMBATOBJECT => Self::CombatObject,
			raw::TFCOLLISION_GROUP_ROCKETS => Self::Rockets,
			raw::TFCOLLISION_GROUP_RESPAWNROOMS => Self::RespawnRooms,
			raw::TFCOLLISION_GROUP_TANK => Self::Tank,
			raw::TFCOLLISION_GROUP_ROCKET_BUT_NOT_WITH_OTHER_ROCKETS => Self::RocketsNotWithRockets,
			_ => return None,
		})
	}

	/// The group `entity` is in, as [`Entity::collision_group`] reads it, or
	/// `None` if the entity has no collideable, or is in a group TF2 does not
	/// number.
	#[doc(alias("GetCollisionGroup", "m_CollisionGroup"))]
	pub fn of(entity: Entity<'_>) -> Option<Self> {
		Self::from_raw(entity.collision_group()?)
	}

	/// The shared group this is, or `None` for one of TF2's own.
	pub const fn shared(self) -> Option<CollisionGroup> {
		match self {
			Self::Shared(shared) => Some(shared),
			_ => None,
		}
	}

	/// The number TF2 gives the group.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Shared(shared) => shared.to_raw(),
			Self::Grenades => raw::TF_COLLISIONGROUP_GRENADES,
			Self::Object => raw::TFCOLLISION_GROUP_OBJECT,

			Self::ObjectSolidToPlayerMovement => {
				raw::TFCOLLISION_GROUP_OBJECT_SOLIDTOPLAYERMOVEMENT
			}

			Self::CombatObject => raw::TFCOLLISION_GROUP_COMBATOBJECT,
			Self::Rockets => raw::TFCOLLISION_GROUP_ROCKETS,
			Self::RespawnRooms => raw::TFCOLLISION_GROUP_RESPAWNROOMS,
			Self::Tank => raw::TFCOLLISION_GROUP_TANK,
			Self::RocketsNotWithRockets => raw::TFCOLLISION_GROUP_ROCKET_BUT_NOT_WITH_OTHER_ROCKETS,
		}
	}
}

impl From<CollisionGroup> for TfCollisionGroup {
	fn from(shared: CollisionGroup) -> Self {
		Self::Shared(shared)
	}
}
