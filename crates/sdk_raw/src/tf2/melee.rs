//! ABI of TF2's melee hit notification, including hits on teammates whose
//! trace attack deals no damage. The virtual call follows native damage
//! dispatch in `CTFWeaponBaseMelee::DoMeleeDamage`.

use crate::vtable_slot;

pub use super::weapons::WeaponSoundKind;

/// The signature of `CTFWeaponBaseMelee::OnEntityHit`, with an entity as
/// receiver because TF2 melee weapons derive through their primary bases.
/// The damage info is native callback-scoped storage; hooks must not retain it.
#[doc(alias("OnEntityHit"))]
pub type MeleeHitFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	target: *mut sys::CBaseEntity,
	info: *mut sys::CTakeDamageInfo,
);

/// The signature of `CBaseCombatWeapon::WeaponSound`, with an entity as
/// receiver because TF2 melee weapons derive through their primary bases:
/// plays the sound the weapon's script gives for the kind of sound, to the
/// clients a prediction-aware filter around the weapon's owner names. The time
/// is when the sound starts, or 0.
#[doc(alias("WeaponSound"))]
pub type WeaponSoundFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, kind: WeaponSoundKind, time: f32);

// Windows 303 matches the `WeaponSound` call in Bravo's `server.dll` (a virtual
// call at offset 2424, in the melee hit routine that plays the hit sounds).
const _: () = assert!(
	WEAPON_SOUND_SLOT
		== cfg_select! {
			target_os = "windows" => 303,
			target_os = "linux" => 309,
		}
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

// Both generated platform bindings declare this signature. Only the
// receiver's pointee differs from WeaponSoundFn; its entity base is at
// offset 0.
const _: fn(
	&sys::CTFWeaponBaseMelee__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CTFWeaponBaseMelee, WeaponSoundKind, f32) =
	|vtable| vtable.CTFWeaponBaseMelee_WeaponSound;

/// The kind of sound a melee weapon plays as its swing hits a player (the
/// flesh hit).
#[doc(alias("MELEE_HIT"))]
pub const MELEE_HIT_SOUND: WeaponSoundKind = sys::WeaponSound_t_MELEE_HIT;

/// The kind of sound a melee weapon plays as its swing hits anything but a
/// player: the world, a building or a prop, the metallic clang or thud.
#[doc(alias("MELEE_HIT_WORLD"))]
pub const MELEE_HIT_WORLD_SOUND: WeaponSoundKind = sys::WeaponSound_t_MELEE_HIT_WORLD;

/// The kind of sound (`WeaponSound_t`) a melee weapon plays as its swing
/// finds nothing.
#[doc(alias("MELEE_MISS"))]
pub const MELEE_MISS_SOUND: WeaponSoundKind = sys::WeaponSound_t_MELEE_MISS;

/// The kind of sound (`WeaponSound_t`) a melee weapon plays as it swings with
/// a critical hit, in place of the miss sound.
#[doc(alias("BURST"))]
pub const BURST_SOUND: WeaponSoundKind = sys::WeaponSound_t_BURST;

/// The generated slot of `CTFWeaponBaseMelee::OnEntityHit` in a melee
/// weapon's primary vtable, shared by its derived weapon classes.
#[doc(alias("OnEntityHit"))]
pub const ON_ENTITY_HIT_SLOT: usize = vtable_slot!(
	sys::CTFWeaponBaseMelee__bindgen_vtable,
	CTFWeaponBaseMelee_OnEntityHit
);

/// The generated slot of `WeaponSound` in a melee weapon's primary vtable,
/// shared by its derived weapon classes.
#[doc(alias("WeaponSound"))]
pub const WEAPON_SOUND_SLOT: usize = vtable_slot!(
	sys::CTFWeaponBaseMelee__bindgen_vtable,
	CTFWeaponBaseMelee_WeaponSound
);
