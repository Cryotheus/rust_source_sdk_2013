//! TF2's own collision groups, which `game/shared/tf/tf_shareddefs.h` numbers
//! from [`LAST_SHARED_COLLISION_GROUP`] on, past those every game shares.
//!
//! `CTFGameRules::ShouldCollide` decides what each group collides with, and
//! passes every pair it does not decide on to the shared rules.

use crate::entities::LAST_SHARED_COLLISION_GROUP;
use std::ffi::c_int;

/// `TF_COLLISIONGROUP_GRENADES`: grenades and stickybombs, which collide with
/// neither players nor each other, nor rockets, and handle hitting players
/// themselves while they fly.
pub const TF_COLLISIONGROUP_GRENADES: c_int = LAST_SHARED_COLLISION_GROUP;

/// `TFCOLLISION_GROUP_COMBATOBJECT`: players and their movement pass through
/// it, but bullets and projectiles hit it, as the Medic's shield and sappers
/// are.
pub const TFCOLLISION_GROUP_COMBATOBJECT: c_int = LAST_SHARED_COLLISION_GROUP + 3;

/// `TFCOLLISION_GROUP_OBJECT`: buildings that block the movement of the other
/// team's players only (`CBaseObject::ShouldCollide`), and push their own
/// team's players away. A building is in it unless it is solid to players,
/// which its `SolidToPlayer` key, or else its kind's defaults in
/// `scripts/objects.txt`, decide.
pub const TFCOLLISION_GROUP_OBJECT: c_int = LAST_SHARED_COLLISION_GROUP + 1;

/// `TFCOLLISION_GROUP_OBJECT_SOLIDTOPLAYERMOVEMENT`: buildings that block the
/// movement of every player.
pub const TFCOLLISION_GROUP_OBJECT_SOLIDTOPLAYERMOVEMENT: c_int = LAST_SHARED_COLLISION_GROUP + 2;

/// `TFCOLLISION_GROUP_RESPAWNROOMS`: respawn room brushes, which collide with
/// players and their movement only.
pub const TFCOLLISION_GROUP_RESPAWNROOMS: c_int = LAST_SHARED_COLLISION_GROUP + 5;

/// `TFCOLLISION_GROUP_ROCKET_BUT_NOT_WITH_OTHER_ROCKETS`: as
/// [`TFCOLLISION_GROUP_ROCKETS`], and also passes through rockets of either
/// group, as arrows do.
pub const TFCOLLISION_GROUP_ROCKET_BUT_NOT_WITH_OTHER_ROCKETS: c_int =
	LAST_SHARED_COLLISION_GROUP + 7;

/// `TFCOLLISION_GROUP_ROCKETS`: rockets and the like, which hit players but
/// not their movement, and pass through weapons, grenades and projectiles of
/// the shared group.
pub const TFCOLLISION_GROUP_ROCKETS: c_int = LAST_SHARED_COLLISION_GROUP + 4;

/// `TFCOLLISION_GROUP_TANK`: players and their movement pass through it, as
/// they do through a team's pumpkin bombs.
pub const TFCOLLISION_GROUP_TANK: c_int = LAST_SHARED_COLLISION_GROUP + 6;
