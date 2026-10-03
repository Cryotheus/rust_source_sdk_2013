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
#[doc(alias = "AddCondEx")]
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

/// Whether `player` has `condition`, from both its object-backed conditions
/// and all of its condition bits, through `CTFPlayer::InCond`.
///
/// # Safety
///
/// As for [`add_cond_ex`].
#[doc(alias = "InCond")]
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
#[doc(alias = "RemoveAllCond")]
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
#[doc(alias = "RemoveCondEx")]
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
