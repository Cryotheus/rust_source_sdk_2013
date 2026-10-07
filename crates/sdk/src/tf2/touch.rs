//! Entities touching one another, and where to intercept it.
//!
//! The engine tells an entity of each entity touching it through
//! `CBaseEntity::Touch`, which it calls through the touched entity's vtable at
//! every tick they touch. The function runs the entity's touch function, such
//! as an item's `ItemTouch` or a dropped ammo pack's `PackTouch`, unless its
//! class overrides it, as `func_regenerate`'s `CRegenerateZone` does.
//!
//! [`sdk_raw::tf2::touch`] holds the function's signature and its vtable slot,
//! [`TOUCH_SLOT`](sdk_raw::tf2::touch::TOUCH_SLOT). [`TouchTargets`] finds the
//! vtable of an entity class by its C++ name, which `metamod_source`'s
//! `touch_hooks` hook, to see each touch of the class's entities before and
//! after the game, and to block it.

use super::class_targets::{ClassTargetError, ClassVtable, SlotTargets};
use sdk_raw::tf2::touch::TOUCH_SLOT;

/// The primary vtable of a C++ class in this server's game module, as
/// [`TouchTargets`] finds it. For an entity class, the game calls
/// `CBaseEntity::Touch` through it on the class's entities, but not on those
/// of the classes deriving from it.
///
/// The search finds any polymorphic class by name: hooking it as an entity
/// class is only sound for one deriving from `CBaseEntity`.
pub type TouchTarget<'s> = ClassVtable<'s>;

/// Why the game module could not be searched for entity classes.
pub type TouchTargetError = ClassTargetError;

/// A snapshot of TF2's game module, in which to find the vtables of its
/// entity classes, such as `CTFAmmoPack` for `tf_ammo_pack`: those holding
/// code at [`TOUCH_SLOT`]. Snapshotting reads the whole module, so find every
/// class needed with one, or search a
/// [`ClassTargets`](super::class_targets::ClassTargets) snapshot taken for
/// other classes, through `From`.
pub type TouchTargets<'s> = SlotTargets<'s, TOUCH_SLOT>;
