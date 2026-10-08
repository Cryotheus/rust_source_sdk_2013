//! The VScripts TF2's entities run, and where to intercept them.
//!
//! The game runs the scripts an entity's `vscripts` key value names through
//! `CBaseEntity::RunVScripts`, which it calls through the entity's vtable as
//! it spawns the entity, such as `logic_script`'s `CLogicScript`, whose
//! entities exist to run scripts.
//!
//! [`sdk_raw::tf2::scripts`] holds the function's signature and its vtable
//! slot. [`ScriptTargets`] finds the vtable of an entity class by its C++
//! name, which `metamod_source`'s `script_hooks` hook, to see each of the
//! class's entities before it runs its scripts, even before any entity of the
//! class exists.

use super::class_targets::{ClassTargetError, ClassVtable, SlotTargets};
use sdk_raw::tf2::scripts::RUN_VSCRIPTS_SLOT;

/// The primary vtable of a C++ class in this server's game module, as
/// [`ScriptTargets`] finds it. For an entity class, the game calls
/// `CBaseEntity::RunVScripts` through it on the class's entities, but not on
/// those of the classes deriving from it.
///
/// The search finds any polymorphic class by name: hooking it as an entity
/// class is only sound for one deriving from `CBaseEntity`.
pub type ScriptTarget<'s> = ClassVtable<'s>;

/// Why the game module could not be searched for entity classes.
pub type ScriptTargetError = ClassTargetError;

/// A snapshot of TF2's game module, in which to find the vtables of its
/// entity classes, such as `CLogicScript` for `logic_script`: those holding
/// code at [`RUN_VSCRIPTS_SLOT`]. Snapshotting reads the whole module, so
/// find every class needed with one, or search a
/// [`ClassTargets`](super::class_targets::ClassTargets) snapshot taken for
/// other classes, through `From`.
pub type ScriptTargets<'s> = SlotTargets<'s, RUN_VSCRIPTS_SLOT>;
