//! Hand-written ABI of TF2 players' reserve ammo: the ammo types of
//! `game/shared/tf/tf_shareddefs.h`, by their index in `m_iAmmo`, and the
//! vtable slot of `CTFPlayer::GiveAmmo`, through which pickups fill a
//! player's reserve ammo up to the max of their class and items.

use crate::abi::CppDestructors;
use crate::vcall;
use crate::vtable_slot;
use std::ffi::c_int;

/// The signature of `CTFPlayer::GiveAmmo`, `int (int, int, bool)`, with the
/// player as its receiver: the count, the ammo type, and whether to skip the
/// pickup sound.
///
/// The generated method takes a `CTFPlayer` receiver. It is the player's
/// primary base, `CBaseEntity`, at the same address, so the method can be
/// called with an entity receiver.
#[doc(alias("GiveAmmo"))]
pub type GiveAmmoFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	count: c_int,
	ammo_type: c_int,
	suppress_sound: bool,
) -> c_int;

// SourceMod's `gamedata/sdktools.games/game.tf.txt` lists `GiveAmmo` as 263
// on Windows and 264 on Linux, whose Itanium vtables start with two
// destructor slots instead of MSVC's one. The overload that takes an ammo
// name is not virtual, so no other `GiveAmmo` shares the slot's group.
const _: () = {
	assert!(GIVE_AMMO_SLOT == 262 + CppDestructors::VTABLE_SLOTS);
	assert!(
		GIVE_AMMO_SLOT
			== vtable_slot!(
				sys::CBaseCombatCharacter__bindgen_vtable,
				CBaseCombatCharacter_GiveAmmo
			)
	);
};

// The generated method has the signature of `GiveAmmoFn`.
const _: fn(
	&sys::CTFPlayer__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CTFPlayer, c_int, c_int, bool) -> c_int =
	|vtable| vtable.CTFPlayer_GiveAmmo;

/// `TF_AMMO_COUNT`, the number of ammo types, which TF2's max ammo tables
/// hold one entry for each of.
pub const AMMO_COUNT: c_int = 7;

/// `TF_AMMO_DUMMY`, which nothing uses.
pub const AMMO_DUMMY: c_int = 0;

/// `TF_AMMO_GRENADES1`, which some consumables count their uses in.
pub const AMMO_GRENADES1: c_int = 4;

/// `TF_AMMO_GRENADES2`, which other consumables count their uses in.
pub const AMMO_GRENADES2: c_int = 5;

/// `TF_AMMO_GRENADES3`, of which every class carries at most 1, for
/// throwables in the action slot.
pub const AMMO_GRENADES3: c_int = 6;

/// `TF_AMMO_METAL`, the Engineer's metal.
pub const AMMO_METAL: c_int = 3;

/// `TF_AMMO_PRIMARY`, the reserve of most primary weapons.
pub const AMMO_PRIMARY: c_int = 1;

/// `TF_AMMO_SECONDARY`, the reserve of most secondary weapons.
pub const AMMO_SECONDARY: c_int = 2;

/// The slot of `CTFPlayer::GiveAmmo` in a TF2 player's primary vtable, from
/// the generated binding.
#[doc(alias("GiveAmmo"))]
pub const GIVE_AMMO_SLOT: usize = vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_GiveAmmo);

/// Calls `CTFPlayer::GiveAmmo`, which gives the player up to `count` reserve
/// ammo of `ammo_type`, as a pickup does, up to the max of their class and
/// items, and returns how much they gained. It never lowers what they have.
///
/// Metal counts are first scaled by the player's `mult_metal_pickup`
/// attribute. Consumables' types are refused to players whose items deny
/// their resupply. A gain plays the pickup sound, unless `suppress_sound`,
/// and fires the `ammo_pickup` event.
///
/// The method is called through the generated vtable, at [`GIVE_AMMO_SLOT`].
///
/// # Safety
///
/// - `player` must point to a live `CTFPlayer` of TF2's loaded game DLL, and
///   the call must be made on the server's main thread.
/// - `ammo_type` must be from 0 up to, but not including, [`AMMO_COUNT`]: the
///   game indexes its max ammo tables with it unchecked.
/// - `count`, scaled by the player's metal pickup multiplier for metal, must
///   convert to an `int`, and the player's reserve of the type must not be
///   negative: the game subtracts it from the type's max as an `int`.
/// - Everything the method runs, such as TF2's attribute hooks and the
///   `ammo_pickup` event's listeners, must free entities only through
///   deferred deletion.
#[doc(alias("GiveAmmo"))]
pub unsafe fn give_ammo(
	player: *mut sys::CBaseEntity,
	count: c_int,
	ammo_type: c_int,
	suppress_sound: bool,
) -> c_int {
	// SAFETY: As the caller promises; the player's primary vtable is TF2's
	// `CTFPlayer` vtable, as generated.
	unsafe {
		vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_GiveAmmo(count, ammo_type, suppress_sound))
	}
}
