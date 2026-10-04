//! Hand-written ABI of entity health in `game/server/baseentity.h`: the
//! `DAMAGE_*` values of `m_takedamage` from `game/shared/shareddefs.h`, the
//! vtable slots of `TakeHealth`, `IsAlive`, `Event_Killed` and
//! `GetMaxHealth`, calls through them, and layout facts of the members
//! holding health.
//!
//! `m_iHealth`, `m_iMaxHealth`, `m_lifeState` and `m_takedamage` are
//! `CBaseEntity` members its datamap declares, which are found there rather
//! than at the generated offsets, since every game's `CBaseEntity` lays them
//! out differently. TF2's buildings (`CBaseObject`) also keep their health as
//! a float, `m_flHealth`, which only the generated TF2 layout gives.

use crate::abi::CppDestructors;
use crate::util::pointee_size;
use crate::{vcall, vtable_slot};
use std::ffi::c_int;
use std::mem::{MaybeUninit, offset_of};

/// The signature of `CBaseEntity::Event_Killed`, `void (const
/// CTakeDamageInfo &)`, with the entity as its receiver.
#[doc(alias("Event_Killed"))]
pub type EventKilledFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, info: *const sys::CTakeDamageInfo);

#[cfg(target_os = "windows")]
const _: () = assert!(offset_of!(sys::CBaseAnimating, _base) == 0);

// `Event_Killed` is slot 68 of TF2's 64-bit Windows `server.dll`, whose
// `CTFPlayer` vtable, found through its run-time type information, holds
// there the function referencing `CTFPlayer::Event_Killed`'s strings, such as
// `"electrocuted_gibbed_red"`. Linux's Itanium vtables start with two
// destructor slots instead of MSVC's one. TF2's players keep the slot, and the
// generated method has the signature of `EventKilledFn`.
const _: () = {
	assert!(EVENT_KILLED_SLOT == 67 + CppDestructors::VTABLE_SLOTS);
	assert!(
		EVENT_KILLED_SLOT == vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_Event_Killed)
	);
};

// `CBaseEntity` keeps its health and maximum in `int`s, and its damage mode in
// one `char`, which the `DAMAGE_*` values are read from as a byte.
const _: () = {
	let entity = MaybeUninit::<sys::CBaseEntity>::uninit();

	// SAFETY: The places are only projected to, never read.
	let (health, max_health, take_damage) = unsafe {
		let entity = entity.as_ptr();

		(
			&raw const (*entity).m_iHealth,
			&raw const (*entity).m_iMaxHealth,
			&raw const (*entity).m_takedamage,
		)
	};

	assert!(pointee_size(health) == size_of::<c_int>());
	assert!(pointee_size(max_health) == size_of::<c_int>());
	assert!(pointee_size(take_damage) == size_of::<u8>());
};

// A building's float health is a plain `float` member, which TF2 does not
// network: it networks the `m_iHealth` it rounds up from it.
const _: () = {
	let object = MaybeUninit::<sys::CBaseObject>::uninit();

	// SAFETY: The place is only projected to, never read.
	let health = unsafe { &raw const (*object.as_ptr()).m_flHealth };

	assert!(pointee_size(health) == size_of::<f32>());
};

// TF2's slots of these methods match the generated vtable on each ABI.
const _: () = {
	use sys::CBaseEntity__bindgen_vtable as Vtable;

	assert!(TAKE_HEALTH_SLOT == vtable_slot!(Vtable, CBaseEntity_TakeHealth));
	assert!(IS_ALIVE_SLOT == vtable_slot!(Vtable, CBaseEntity_IsAlive));
	assert!(EVENT_KILLED_SLOT == vtable_slot!(Vtable, CBaseEntity_Event_Killed));
};

// A `CBaseEntity *` to a TF2 building is also a `CBaseObject *`, whose chain
// of primary bases down to `CBaseEntity` each start their class, as
// `sdk_raw::tf2` asserts for the bases a building shares with a player.
const _: () = assert!(
	offset_of!(sys::CBaseObject, _base) == 0
		&& offset_of!(sys::CBaseCombatCharacter, _base) == 0
		&& offset_of!(sys::CBaseFlex, _base) == 0
		&& offset_of!(sys::CBaseAnimatingOverlay, _base) == 0
);

const _: fn(&sys::CBaseEntity__bindgen_vtable) -> EventKilledFn =
	|vtable| vtable.CBaseEntity_Event_Killed;

/// The `m_takedamage` of an entity that takes damage, and which aim
/// assistance may target.
///
/// This is `DAMAGE_AIM` from `game/shared/shareddefs.h`.
pub const DAMAGE_AIM: u8 = 3;

/// The `m_takedamage` of an entity that runs its damage functions, such as
/// its outputs and effects, without losing health, in the classes whose
/// `OnTakeDamage` checks it. TF2's buildings (`CBaseObject::OnTakeDamage`) do
/// not, and still lose health.
///
/// This is `DAMAGE_EVENTS_ONLY` from `game/shared/shareddefs.h`.
pub const DAMAGE_EVENTS_ONLY: u8 = 1;

/// The `m_takedamage` of an entity whose `OnTakeDamage` ignores all damage.
///
/// This is `DAMAGE_NO` from `game/shared/shareddefs.h`.
pub const DAMAGE_NO: u8 = 0;

/// The `m_takedamage` of an entity that takes damage.
///
/// This is `DAMAGE_YES` from `game/shared/shareddefs.h`.
pub const DAMAGE_YES: u8 = 2;

/// The `DMG_*` mask of no particular kind of damage, from
/// `game/shared/shareddefs.h`, which plain healing passes to `TakeHealth`.
pub const DMG_GENERIC: c_int = 0;

/// `CBaseEntity::Event_Killed` in the primary vtable, which directly follows
/// [`IS_ALIVE_SLOT`], so it too is the same for every game.
///
/// The game calls it once for each death, after the entity's health reached
/// zero, with the damage that killed it.
#[doc(alias("Event_Killed"))]
pub const EVENT_KILLED_SLOT: usize = IS_ALIVE_SLOT + 1;

/// `CBaseEntity::IsAlive` in the primary vtable.
///
/// Every virtual method TF2 declares in `CBaseEntity` under `TF_DLL`, and
/// `IsNextBot`, which TF2 and HL2:DM declare under `NEXT_BOT`, follows it, so
/// the slot is the same for every game. Derived from
/// `game/server/baseentity.h` with the MSVC ABI model on Windows and the
/// Itanium ABI model on Linux.
#[doc(alias("IsAlive"))]
pub const IS_ALIVE_SLOT: usize = cfg_select! {
	target_os = "windows" => 67,
	target_os = "linux" => 68,
};

/// `CBaseEntity::TakeHealth` in the primary vtable, which directly precedes
/// [`IS_ALIVE_SLOT`], so it too is the same for every game.
#[doc(alias("TakeHealth"))]
pub const TAKE_HEALTH_SLOT: usize = IS_ALIVE_SLOT - 1;

/// `CBaseEntity::GetMaxHealth` in TF2's game DLL, from the generated vtable.
///
/// Virtual methods that only some games declare, under `TF_DLL` and
/// `NEXT_BOT`, precede it, so other games' slots differ by game, and none is
/// given for them.
#[doc(alias("GetMaxHealth"))]
pub const TF2_GET_MAX_HEALTH_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_GetMaxHealth);

/// Calls TF2's `CBaseEntity::GetMaxHealth`, which returns `m_iMaxHealth`
/// unless the entity's class computes it, as TF2's players do from their
/// class and attributes (`CTFPlayer::GetMaxHealth`).
///
/// The method is called through the generated vtable, at
/// [`TF2_GET_MAX_HEALTH_SLOT`].
///
/// # Safety
///
/// `entity` must point to a live `CBaseEntity` of TF2's loaded game DLL, and
/// the call must be made on the server's main thread.
#[doc(alias("GetMaxHealth"))]
pub unsafe fn get_max_health(entity: *mut sys::CBaseEntity) -> c_int {
	// SAFETY: The entity is live, and its vtable is TF2's, as generated. The
	// method only reads the entity, or computes from its class and attributes.
	unsafe { vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_GetMaxHealth()) }
}

/// Calls `CBaseEntity::IsAlive`, which tells whether the entity's
/// `m_lifeState` is `LIFE_ALIVE`, unless its class overrides it, as props,
/// which are never alive, do (`CBaseProp::IsAlive`).
///
/// The method is called through the generated vtable, whose slot of it,
/// [`IS_ALIVE_SLOT`], every game DLL shares.
///
/// # Safety
///
/// `entity` must point to a live `CBaseEntity` of the loaded game DLL, and the
/// call must be made on the server's main thread.
#[doc(alias("IsAlive"))]
pub unsafe fn is_alive(entity: *mut sys::CBaseEntity) -> bool {
	// SAFETY: The entity is live, and every game DLL's vtable has `IsAlive`
	// where the generated one does.
	unsafe { vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_IsAlive()) }
}

/// Calls `CBaseEntity::TakeHealth`, which heals the entity by `amount` up to
/// its maximum health, and returns the health it gained.
///
/// `damage_type` holds `DMG_*` bits, which `CBasePlayer` clears from the
/// damage it is suffering from. The base method heals only networked
/// entities whose `m_takedamage` is at least [`DAMAGE_YES`], and fires the
/// `take_health` event for players. TF2's players first scale `amount` by
/// their active weapon's `mult_healing_received` attribute, and with
/// `DMG_IGNORE_MAXHEALTH`, add it without the base method: beyond their
/// maximum, without its checks or event, clearing the damage bits
/// themselves.
///
/// The method is called through the generated vtable, whose slot of it,
/// [`TAKE_HEALTH_SLOT`], every game DLL shares.
///
/// # Safety
///
/// - `entity` must point to a live `CBaseEntity` of the loaded game DLL, and
///   the call must be made on the server's main thread.
/// - `amount`, as TF2's players scale it, must convert to an `int`, and adding
///   that `int` to the entity's `m_iHealth` must not overflow it: the game
///   adds it to the member as an `int`.
/// - Everything the method runs, such as TF2's attribute hooks, must free
///   entities only through deferred deletion.
#[doc(alias("TakeHealth"))]
pub unsafe fn take_health(entity: *mut sys::CBaseEntity, amount: f32, damage_type: c_int) -> c_int {
	// SAFETY: The entity is live, and every game DLL's vtable has
	// `TakeHealth` where the generated one does. The caller upholds the rest.
	unsafe {
		vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_TakeHealth(
			amount,
			damage_type,
		))
	}
}
