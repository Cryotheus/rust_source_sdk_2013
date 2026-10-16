//! Hand-written ABI of TF2's taunt attacks: the two `CTFPlayer` members in
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
