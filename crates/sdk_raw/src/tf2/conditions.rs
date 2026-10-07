//! Hand-written ABI of TF2's player conditions: the native `CTFPlayer`
//! condition methods, called through the player's script bindings, and
//! `game/shared/tf/tf_player_shared.h`'s `PERMANENT_CONDITION`.
//!
//! The methods run the game's normal add and remove notifications, durations
//! and cleanup; no condition bits are written directly.

use super::script_binding::{self as binding, BindingError};
use std::ffi::CStr;
use std::ptr::NonNull;

/// The class whose script bindings declare the condition methods.
const CLASS: &CStr = c"CTFPlayer";

/// The duration of a condition that does not expire, in seconds.
pub const PERMANENT_CONDITION: f32 = -1.0;

/// Adds `condition` to `player` for `duration` seconds, or
/// [`PERMANENT_CONDITION`], with `provider`, which may be null, as the entity
/// providing it, through `CTFPlayer::AddCondEx`.
///
/// The game may keep a longer existing duration, and can refuse the addition,
/// for example on a dead player.
///
/// # Safety
///
/// As for [`binding::call`], with `player` a live `CTFPlayer` and `condition`
/// a valid `ETFCond`.
#[doc(alias("AddCondEx"))]
pub unsafe fn add_cond_ex(
	player: NonNull<sys::CBaseEntity>,
	condition: sys::ETFCond,
	duration: f32,
	provider: sys::HSCRIPT,
) -> Result<(), BindingError> {
	// SAFETY: As the caller promises; the method takes these three values.
	unsafe {
		binding::call(
			player,
			CLASS,
			c"AddCondEx",
			&mut [
				binding::int(condition),
				binding::float(duration),
				binding::handle(provider),
			],
			binding::VOID,
		)
	}
	.map(drop)
}

/// The seconds left of `player`'s `condition`, through
/// `CTFPlayer::GetCondDuration`: [`PERMANENT_CONDITION`] for one that does not
/// expire, and 0 for one the player does not have.
///
/// # Safety
///
/// As for [`add_cond_ex`].
#[doc(alias("GetCondDuration"))]
pub unsafe fn get_cond_duration(
	player: NonNull<sys::CBaseEntity>,
	condition: sys::ETFCond,
) -> Result<f32, BindingError> {
	// SAFETY: As the caller promises; the read-only query takes the condition.
	let result = unsafe {
		binding::call(
			player,
			CLASS,
			c"GetCondDuration",
			&mut [binding::int(condition)],
			binding::FLOAT,
		)
	}?;

	// SAFETY: `call` checked that the method's adapter returned a
	// `FIELD_FLOAT` variant, which it assigns through the float member.
	Ok(unsafe { result.__bindgen_anon_1.m_float })
}

/// Whether `player` has `condition`, from both its object-backed conditions
/// and all of its condition bits, through `CTFPlayer::InCond`.
///
/// # Safety
///
/// As for [`add_cond_ex`].
#[doc(alias("InCond"))]
pub unsafe fn in_cond(
	player: NonNull<sys::CBaseEntity>,
	condition: sys::ETFCond,
) -> Result<bool, BindingError> {
	// SAFETY: As the caller promises; the read-only query takes the condition.
	let result = unsafe {
		binding::call(
			player,
			CLASS,
			c"InCond",
			&mut [binding::int(condition)],
			binding::BOOL,
		)
	}?;

	// SAFETY: `call` checked that the method's adapter returned a
	// `FIELD_BOOLEAN` variant, which it assigns through the bool member.
	Ok(unsafe { result.__bindgen_anon_1.m_bool })
}

/// Removes every condition from `player`, including its object-backed ones,
/// through `CTFPlayer::RemoveAllCond`.
///
/// # Safety
///
/// As for [`binding::call`], with `player` a live `CTFPlayer`.
#[doc(alias("RemoveAllCond"))]
pub unsafe fn remove_all_cond(player: NonNull<sys::CBaseEntity>) -> Result<(), BindingError> {
	// SAFETY: As the caller promises; the method takes no arguments.
	unsafe { binding::call(player, CLASS, c"RemoveAllCond", &mut [], binding::VOID) }.map(drop)
}

/// Removes `condition` from `player` through `CTFPlayer::RemoveCondEx`.
/// `ignore_duration` bypasses the condition's minimum duration, which, for
/// example, a critical boost has.
///
/// # Safety
///
/// As for [`add_cond_ex`].
#[doc(alias("RemoveCondEx"))]
pub unsafe fn remove_cond_ex(
	player: NonNull<sys::CBaseEntity>,
	condition: sys::ETFCond,
	ignore_duration: bool,
) -> Result<(), BindingError> {
	// SAFETY: As the caller promises; the method takes these two values.
	unsafe {
		binding::call(
			player,
			CLASS,
			c"RemoveCondEx",
			&mut [binding::int(condition), binding::boolean(ignore_duration)],
			binding::VOID,
		)
	}
	.map(drop)
}

/// Sets the seconds left of `player`'s `condition`, through
/// `CTFPlayer::SetCondDuration`, or makes it permanent with
/// [`PERMANENT_CONDITION`]. The game counts the time down from there, and
/// removes the condition once it reaches zero.
///
/// The time is set even for a condition the player does not have, where it
/// stays until the condition is next added: that addition keeps the longer of
/// the two, or stays permanent (`CTFPlayerShared::AddCond`).
///
/// # Safety
///
/// As for [`add_cond_ex`].
#[doc(alias("SetCondDuration"))]
pub unsafe fn set_cond_duration(
	player: NonNull<sys::CBaseEntity>,
	condition: sys::ETFCond,
	duration: f32,
) -> Result<(), BindingError> {
	// SAFETY: As the caller promises; the method takes these two values.
	unsafe {
		binding::call(
			player,
			CLASS,
			c"SetCondDuration",
			&mut [binding::int(condition), binding::float(duration)],
			binding::VOID,
		)
	}
	.map(drop)
}
