//! Hand-written ABI of TF2's spectating: the observer modes of
//! `game/shared/shareddefs.h`, and the vtable slots of a TF2 player's observer
//! methods.
//!
//! The observer modes are TF2's: its `shareddefs.h` inserts
//! [`OBS_MODE_POI`] for PASS Time before [`OBS_MODE_ROAMING`], which other
//! Source SDK 2013 games number one lower.

use crate::abi::CppDestructors;
use crate::vtable_slot;
use std::ffi::c_int;

/// The signature of `CTFPlayer::GetObserverMode`, `int ()`, with the player as
/// its receiver.
#[doc(alias("GetObserverMode"))]
pub type GetObserverModeFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity) -> c_int;

/// The signature of `CTFPlayer::SetObserverMode`, `bool (int)`, with the player
/// as its receiver.
///
/// The generated method takes a `CTFPlayer` receiver. It is the player's
/// primary base, `CBaseEntity`, at the same address, so the method can be
/// called and hooked with an entity receiver.
#[doc(alias("SetObserverMode"))]
pub type SetObserverModeFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity, mode: c_int) -> bool;

/// The signature of `CTFPlayer::SetObserverTarget`, `bool (CBaseEntity *)`,
/// with the player as its receiver.
#[doc(alias("SetObserverTarget"))]
pub type SetObserverTargetFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, target: *mut sys::CBaseEntity) -> bool;

// The slots lie between those of `ForceRespawn` and `GiveNamedItem`, which
// SourceMod's `gamedata/sm-tf2.games.txt` and `sdktools.games/game.tf.txt`
// list as 337 and 413 on Windows, one more on Linux, as the generated vtable
// has them. Every other slot of `CTFPlayer` that SourceMod's TF2 gamedata
// lists matches the generated vtable too. The generated methods have the
// signatures above, with `CTFPlayer` receivers.
const _: () = {
	assert!(GET_OBSERVER_MODE_SLOT == 385 + CppDestructors::VTABLE_SLOTS);
	assert!(SET_OBSERVER_MODE_SLOT == 384 + CppDestructors::VTABLE_SLOTS);
	assert!(SET_OBSERVER_TARGET_SLOT == 386 + CppDestructors::VTABLE_SLOTS);

	let _: fn(
		&sys::CTFPlayer__bindgen_vtable,
	) -> unsafe extern "C" fn(*mut sys::CTFPlayer) -> c_int =
		|vtable| vtable.CTFPlayer_GetObserverMode;
	let _: fn(
		&sys::CTFPlayer__bindgen_vtable,
	) -> unsafe extern "C" fn(*mut sys::CTFPlayer, c_int) -> bool =
		|vtable| vtable.CTFPlayer_SetObserverMode;
	let _: fn(
		&sys::CTFPlayer__bindgen_vtable,
	) -> unsafe extern "C" fn(*mut sys::CTFPlayer, *mut sys::CBaseEntity) -> bool =
		|vtable| vtable.CTFPlayer_SetObserverTarget;
};

/// The slot of `CTFPlayer::GetObserverMode` in a TF2 player's primary vtable,
/// from the generated binding.
#[doc(alias("GetObserverMode"))]
pub const GET_OBSERVER_MODE_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_GetObserverMode);

/// The last observer mode a player can choose by cycling through them
/// (`spec_mode`).
///
/// This is `LAST_PLAYER_OBSERVERMODE` from `game/shared/shareddefs.h`.
pub const LAST_PLAYER_OBSERVERMODE: c_int = OBS_MODE_ROAMING;

/// The number of observer modes, one past the last.
///
/// This is `NUM_OBSERVER_MODES` from `game/shared/shareddefs.h`.
pub const NUM_OBSERVER_MODES: c_int = OBS_MODE_ROAMING + 1;

/// Follows a player in third person.
///
/// This is `OBS_MODE_CHASE` from `game/shared/shareddefs.h`.
pub const OBS_MODE_CHASE: c_int = 5;

/// The death cam, which turns a player who just died towards their killer.
///
/// This is `OBS_MODE_DEATHCAM` from `game/shared/shareddefs.h`.
pub const OBS_MODE_DEATHCAM: c_int = 1;

/// Views from a fixed position.
///
/// This is `OBS_MODE_FIXED` from `game/shared/shareddefs.h`.
pub const OBS_MODE_FIXED: c_int = 3;

/// The freeze cam, which zooms in on a dead player's killer and freezes the
/// frame.
///
/// This is `OBS_MODE_FREEZECAM` from `game/shared/shareddefs.h`.
pub const OBS_MODE_FREEZECAM: c_int = 2;

/// Follows a player in first person.
///
/// This is `OBS_MODE_IN_EYE` from `game/shared/shareddefs.h`.
pub const OBS_MODE_IN_EYE: c_int = 4;

/// Not spectating.
///
/// This is `OBS_MODE_NONE` from `game/shared/shareddefs.h`.
pub const OBS_MODE_NONE: c_int = 0;

/// Follows PASS Time's point of interest, such as its ball. Outside PASS
/// Time, TF2 turns it into [`OBS_MODE_ROAMING`] (`CTFPlayer::SetObserverMode`).
///
/// This is `OBS_MODE_POI` from `game/shared/shareddefs.h`.
pub const OBS_MODE_POI: c_int = 6;

/// Roams freely, as a spectator.
///
/// This is `OBS_MODE_ROAMING` from `game/shared/shareddefs.h`.
pub const OBS_MODE_ROAMING: c_int = 7;

/// The slot of `CTFPlayer::SetObserverMode` in a TF2 player's primary vtable,
/// from the generated binding.
#[doc(alias("SetObserverMode"))]
pub const SET_OBSERVER_MODE_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_SetObserverMode);

/// The slot of `CTFPlayer::SetObserverTarget` in a TF2 player's primary vtable,
/// from the generated binding.
#[doc(alias("SetObserverTarget"))]
pub const SET_OBSERVER_TARGET_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_SetObserverTarget);
