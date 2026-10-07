//! The teams of TF2's entities, and where to intercept their changes.
//!
//! The game changes an entity's team through `CBaseEntity::ChangeTeam`, which
//! it calls through the entity's vtable. Some classes act on their team, such
//! as a respawn room (`CFuncRespawnRoom`), which only counts for its team's
//! players, or a resupply cabinet (`CRegenerateZone`), which only resupplies
//! them.
//!
//! [`sdk_raw::tf2::teams`] holds the function's signature and its vtable slot.
//! [`TeamTargets`] finds the vtable of an entity class by its C++ name, which
//! `metamod_source`'s `team_hooks` hook, to decide each change of the class's
//! entities' teams, before any entity of the class exists.

use super::class_targets::{ClassTargetError, ClassVtable, SlotTargets};
use sdk_raw::tf2::teams::CHANGE_TEAM_SLOT;

/// The primary vtable of a C++ class in this server's game module, as
/// [`TeamTargets`] finds it. For an entity class, the game calls
/// `CBaseEntity::ChangeTeam` through it on the class's entities, but not on
/// those of the classes deriving from it.
///
/// The search finds any polymorphic class by name: hooking it as an entity
/// class is only sound for one deriving from `CBaseEntity`.
pub type TeamTarget<'s> = ClassVtable<'s>;

/// Why the game module could not be searched for entity classes.
pub type TeamTargetError = ClassTargetError;

/// A snapshot of TF2's game module, in which to find the vtables of its
/// entity classes, such as `CFuncRespawnRoom` for `func_respawnroom`: those
/// holding code at [`CHANGE_TEAM_SLOT`]. Snapshotting reads the whole module,
/// so find every class needed with one, or search a
/// [`ClassTargets`](super::class_targets::ClassTargets) snapshot taken for
/// other classes, through `From`.
pub type TeamTargets<'s> = SlotTargets<'s, CHANGE_TEAM_SLOT>;
