//! Hand-written ABI of TF2's taunt attacks, and of the delay of a taunt's
//! looping sound ([`SOUND_LOOP_TIME_OFFSET`]): the two `CTFPlayer` members in
//! which a taunt schedules its attack, `m_iTauntAttack`, a `taunt_attack_t`,
//! and `m_flTauntAttackTime`, the game time it lands after. Neither is
//! networked, so they are read at their offsets in the generated layout,
//! where the game's own code reads them too.
//!
//! `CTFPlayer::PostThink` runs a scheduled attack through `DoTauntAttack`, a
//! function no vtable holds, in the player's first think after moving once its
//! time has passed: it sets the time back to 0 and calls the function, which
//! does nothing unless the player still taunts, lives, and has an attack, and
//! otherwise sets the attack back to `TAUNTATK_NONE` and runs it. The stages
//! of an attack that hits more than once, such as the Spy's fencing, schedule
//! the next stage as they run.

use std::ffi::c_int;
use std::mem::offset_of;
use std::ptr::NonNull;

/// The offset of `CTFPlayer::m_iTauntAttack` in players: the attack the
/// player's taunt has scheduled, a `taunt_attack_t`, or `TAUNTATK_NONE`.
pub const ATTACK_OFFSET: usize = offset_of!(sys::CTFPlayer, m_iTauntAttack);

/// The offset of `CTFPlayer::m_flTauntAttackTime` in players: the game time
/// after which the scheduled attack lands, or 0 for none.
pub const ATTACK_TIME_OFFSET: usize = offset_of!(sys::CTFPlayer, m_flTauntAttackTime);

/// The offset of `CTFPlayer::m_flTauntSoundLoopTime` in players: the game
/// time after which the player's think starts their taunt's looping sound
/// (the item's `taunt success sound loop`), or 0 for none pending.
pub const SOUND_LOOP_TIME_OFFSET: usize = offset_of!(sys::CTFPlayer, m_flTauntSoundLoopTime);

/// Cancels the attack `player`'s taunt has scheduled, if any, as the game
/// does once it runs one: the attack is set back to `TAUNTATK_NONE` and its
/// time to 0, so that no think runs it. The taunt itself goes on.
///
/// # Safety
///
/// `player` is a live `CTFPlayer`, which nothing else reads or writes during
/// the call.
#[doc(alias("m_iTauntAttack", "m_flTauntAttackTime"))]
pub unsafe fn cancel_scheduled_attack(player: NonNull<sys::CBaseEntity>) {
	let player = player.as_ptr().cast::<u8>();

	// SAFETY: As for `scheduled_attack`; the caller vouches that nothing else
	// uses the members meanwhile. Neither is networked, so the engine need not
	// be told of the change.
	unsafe {
		player.byte_add(ATTACK_OFFSET).cast::<c_int>().write(0);
		player.byte_add(ATTACK_TIME_OFFSET).cast::<f32>().write(0.0);
	}
}

/// Keeps `player`'s taunt from starting its looping sound, if it has not yet:
/// sets the time it starts after to 0, as the game does once it has started
/// it. The game starts the loop in `CTFPlayer::TFPlayerThink`, by a reliable
/// `PlayerTauntSoundLoopStart` user message to every client, which plays it on
/// the player; the sound never goes through the engine's sound calls. The
/// loop's name stays, so the taunt's end still sends
/// `PlayerTauntSoundLoopEnd`, which stops nothing.
///
/// # Safety
///
/// `player` is a live `CTFPlayer`, which nothing else reads or writes during
/// the call.
#[doc(alias("m_flTauntSoundLoopTime"))]
pub unsafe fn cancel_sound_loop(player: NonNull<sys::CBaseEntity>) {
	// SAFETY: The caller vouches for the player, a `CTFPlayer` as every TF2
	// player is, whose member is at this offset of the generated layout, a
	// 4-byte float the game writes the same way. It is not networked.
	unsafe {
		player
			.as_ptr()
			.cast::<u8>()
			.byte_add(SOUND_LOOP_TIME_OFFSET)
			.cast::<f32>()
			.write(0.0);
	}
}

/// The attack `player`'s taunt has scheduled, a `taunt_attack_t`, and the
/// game time after which it lands, both as the player keeps them: either may
/// be left over from an attack the game no longer runs.
///
/// # Safety
///
/// `player` is a live `CTFPlayer`, which nothing writes to during the call.
#[doc(alias("m_iTauntAttack", "m_flTauntAttackTime"))]
pub unsafe fn scheduled_attack(player: NonNull<sys::CBaseEntity>) -> (c_int, f32) {
	let player = player.as_ptr().cast::<u8>();

	// SAFETY: The caller vouches for the player, a `CTFPlayer` as every TF2
	// player is, whose members are at these offsets of the generated layout.
	// Both are 4-byte members, read as the game reads them; `taunt_attack_t` is
	// a C enum, whose values fit an int.
	unsafe {
		(
			player.byte_add(ATTACK_OFFSET).cast::<c_int>().read(),
			player.byte_add(ATTACK_TIME_OFFSET).cast::<f32>().read(),
		)
	}
}
