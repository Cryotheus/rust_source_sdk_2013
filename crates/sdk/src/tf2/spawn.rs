//! Entities spawning, and where to follow it.
//!
//! The game spawns each entity once it created it and gave it its keys,
//! through `CBaseEntity::Spawn`, which `DispatchSpawn` calls through the
//! entity's vtable. A class's `Spawn` sets what its entities start with, such
//! as their solid type and flags and their collision group, overwriting what
//! was set on the entity as it was created: a change to those must wait for
//! the spawn.
//!
//! [`sdk_raw::entities`] holds the function's signature and its vtable slot,
//! [`SPAWN_SLOT`](sdk_raw::entities::SPAWN_SLOT). [`SpawnTargets`] finds the
//! vtable of an entity class by its C++ name, which `metamod_source`'s
//! `spawn_hooks` hook, to see each of the class's entities once spawned,
//! before any entity of the class exists.

use super::class_targets::{ClassTargetError, ClassVtable, SlotTargets};
use sdk_raw::entities::SPAWN_SLOT;

/// The primary vtable of a C++ class in this server's game module, as
/// [`SpawnTargets`] finds it. For an entity class, the game calls
/// `CBaseEntity::Spawn` through it on the class's entities, but not on those
/// of the classes deriving from it.
///
/// The search finds any polymorphic class by name: hooking it as an entity
/// class is only sound for one deriving from `CBaseEntity`.
pub type SpawnTarget<'s> = ClassVtable<'s>;

/// Why the game module could not be searched for entity classes.
pub type SpawnTargetError = ClassTargetError;

/// A snapshot of TF2's game module, in which to find the vtables of its
/// entity classes, such as `CTFFlameManager` for `tf_flame_manager`: those
/// holding code at [`SPAWN_SLOT`]. Snapshotting reads the whole module, so
/// find every class needed with one, or search a
/// [`ClassTargets`](super::class_targets::ClassTargets) snapshot taken for
/// other classes, through `From`.
pub type SpawnTargets<'s> = SlotTargets<'s, SPAWN_SLOT>;
