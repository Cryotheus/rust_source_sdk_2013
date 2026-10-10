//! ABI of TF2's melee hit notification, including hits on teammates whose
//! trace attack deals no damage. The virtual call follows native damage
//! dispatch in `CTFWeaponBaseMelee::DoMeleeDamage`.

use crate::vtable_slot;

/// The signature of `CTFWeaponBaseMelee::OnEntityHit`, with an entity as
/// receiver because TF2 melee weapons derive through their primary bases.
/// The damage info is native callback-scoped storage; hooks must not retain it.
#[doc(alias("OnEntityHit"))]
pub type MeleeHitFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	target: *mut sys::CBaseEntity,
	info: *mut sys::CTakeDamageInfo,
);

// Both generated platform bindings declare this signature. Only the
// receiver's pointee differs from MeleeHitFn; its entity base is at offset 0.
const _: fn(
	&sys::CTFWeaponBaseMelee__bindgen_vtable,
) -> unsafe extern "C" fn(
	*mut sys::CTFWeaponBaseMelee,
	*mut sys::CBaseEntity,
	*mut sys::CTakeDamageInfo,
) = |vtable| vtable.CTFWeaponBaseMelee_OnEntityHit;

/// The generated slot of `CTFWeaponBaseMelee::OnEntityHit` in a melee
/// weapon's primary vtable, shared by its derived weapon classes.
#[doc(alias("OnEntityHit"))]
pub const ON_ENTITY_HIT_SLOT: usize = vtable_slot!(
	sys::CTFWeaponBaseMelee__bindgen_vtable,
	CTFWeaponBaseMelee_OnEntityHit
);
