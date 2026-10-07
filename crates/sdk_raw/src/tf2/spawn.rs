//! Hand-written ABI of entities spawning: the search for an entity class's
//! vtable, through which the game calls `CBaseEntity::Spawn` on the class's
//! entities.
//!
//! The game spawns an entity once it created it and gave it its keys, through
//! `DispatchSpawn`, which calls `Spawn` through the entity's vtable. Every
//! game has the method at the same slot, [`SPAWN_SLOT`], with the signature
//! [`SpawnFn`](crate::entities::SpawnFn).

use crate::entities::SPAWN_SLOT;
use crate::tf2::class_targets::SlotVtables;

/// An owned snapshot of TF2's game server module, in which to find the
/// primary vtables of its entity classes, through which the game calls
/// `Spawn` on their entities: those whose [`SPAWN_SLOT`] entries are
/// executable.
pub type SpawnVtables = SlotVtables<SPAWN_SLOT>;
