//! Class targets for intercepting the map inputs received by TF2 entities.
//!
//! `AcceptInput` receives a name and a `variant_t` before running the input
//! handler. Its primary-vtable slot is checked from the generated SDK.
//! Targets hold no entity and remain usable across map changes. A class
//! target covers that class, not the distinct vtables of derived classes.

use super::class_targets::{ClassTargetError, ClassVtable, SlotTargets};
use sdk_raw::entities::ACCEPT_INPUT_SLOT;

/// The primary vtable of a class whose `AcceptInput` may be intercepted.
/// The caller must establish that this class derives from `CBaseEntity`.
pub type InputTarget<'s> = ClassVtable<'s>;

/// Why the game module could not be searched.
pub type InputTargetError = ClassTargetError;

/// One game-module snapshot locating classes with code at `AcceptInput`'s
/// slot. Reuse one snapshot to find all classes needed by a plugin.
pub type InputTargets<'s> = SlotTargets<'s, ACCEPT_INPUT_SLOT>;
