//! Typed native pusher-blocked callback from generated CBaseEntity bindings.
use crate::tf2::class_targets::SlotVtables;
use crate::vtable_slot;

/// The pusher and the entity that prevented its speculative movement.
pub type BlockedFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, other: *mut sys::CBaseEntity);

/// A game-module snapshot whose selected class has executable code at the slot.
pub type BlockedVtables = SlotVtables<BLOCKED_SLOT>;

const _: () = assert!(BLOCKED_SLOT == crate::tf2::touch::TOUCH_SLOT + 3);
const _: fn(&sys::CBaseEntity__bindgen_vtable) -> BlockedFn = |vtable| vtable.CBaseEntity_Blocked;

/// CBaseEntity::Blocked in the generated, target-specific primary vtable.
pub const BLOCKED_SLOT: usize = vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_Blocked);
