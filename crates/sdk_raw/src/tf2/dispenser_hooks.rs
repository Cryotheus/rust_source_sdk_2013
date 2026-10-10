//! ABI of `CObjectDispenser::DispenseAmmo`, which gives a player primary
//! and secondary ammunition, then calls `DispenseMetal`. Hooking only the
//! latter cannot replace or charge for the ammunition supplied first.

use crate::vtable_slot;
use std::mem::offset_of;

/// `bool CObjectDispenser::DispenseAmmo(CTFPlayer *)`, with the dispenser
/// as its receiver. The return value tells the dispenser's think whether
/// ammunition was accepted, determining its next resupply interval.
///
/// Both classes start with `CBaseEntity` through their primary bases; the
/// generated layouts assert the dispenser base here, and the player's in
/// [`super`]. These pointers therefore need no receiver adjustment.
#[doc(alias("DispenseAmmo"))]
pub type DispenseAmmoFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, player: *mut sys::CBaseEntity) -> bool;

const _: () = assert!(offset_of!(sys::CObjectDispenser, _base) == 0);

// Keep the typed hook contract tied to the generated declaration.
const _: fn(
	&sys::CObjectDispenser__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CObjectDispenser, *mut sys::CTFPlayer) -> bool =
	|vtable| vtable.CObjectDispenser_DispenseAmmo;

/// The slot of `CObjectDispenser::DispenseAmmo` in a dispenser's primary
/// vtable, derived from the generated ABI. This slot is not a building-wide
/// slot: sentries, teleporters and sappers do not have this method.
#[doc(alias("DispenseAmmo"))]
pub const DISPENSE_AMMO_SLOT: usize = vtable_slot!(
	sys::CObjectDispenser__bindgen_vtable,
	CObjectDispenser_DispenseAmmo
);
