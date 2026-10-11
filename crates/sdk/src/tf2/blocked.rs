//! Targets for native pusher-blocked notifications; holds no map entity.
use super::class_targets::{ClassTargetError, ClassVtable, SlotTargets};
use sdk_raw::tf2::blocked::BLOCKED_SLOT;

/// Primary class vtable. The caller must verify CBaseEntity primary lineage.
pub type BlockedTarget<'s> = ClassVtable<'s>;

/// Why a game-module class target could not be found.
pub type BlockedTargetError = ClassTargetError;

/// Snapshot TF2's module once, then find every required class in it.
pub type BlockedTargets<'s> = SlotTargets<'s, BLOCKED_SLOT>;
