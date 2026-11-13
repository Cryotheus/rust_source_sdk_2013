//! Hand-written ABI of TF2's miniguns (`CTFMinigun`): the state of their
//! barrel, `m_iWeaponState`, a `MinigunState_t`, and the game time their ring
//! of fire burns next, `m_flNextRingOfFireAttackTime`. The state is networked,
//! the time is neither networked nor described, so both are read at their
//! offsets in the generated layout, where the game's own code reads them too.
//!
//! `CTFMinigun::SharedAttack`, which the minigun's primary and secondary
//! attacks run from its `ItemPostFrame`, calls `RingOfFireAttack` while the
//! barrel spins or fires and its Heavy has ammo, if the weapon has the
//! `ring_of_fire_while_aiming` attribute. Once the time has passed, that burns
//! the enemy players around the Heavy and sets the time half a second ahead.
//! `SetWeaponState` also sets it half a second ahead whenever the barrel
//! leaves the idle, starting or dry-firing state, and `WeaponReset` sets it to
//! 0.

use std::ffi::c_uint;
use std::mem::offset_of;
use std::ptr::NonNull;

const _: () = {
	assert!(STATE_OFFSET.is_multiple_of(align_of::<c_uint>()));
	assert!(RING_OF_FIRE_TIME_OFFSET.is_multiple_of(align_of::<f32>()));
};

/// `AC_STATE_DRYFIRE`: the barrel spins, firing without ammo.
pub const AC_STATE_DRYFIRE: c_uint = 4;

/// `AC_STATE_FIRING`: the barrel spins and fires.
pub const AC_STATE_FIRING: c_uint = 2;

/// `AC_STATE_IDLE`: the barrel stands still.
pub const AC_STATE_IDLE: c_uint = 0;

/// `AC_STATE_SPINNING`: the barrel spins without firing.
pub const AC_STATE_SPINNING: c_uint = 3;

/// `AC_STATE_STARTFIRING`: the barrel winds up.
pub const AC_STATE_STARTFIRING: c_uint = 1;

/// How long after a ring of fire burns, or after the barrel leaves the idle,
/// starting or dry-firing state, the next may burn, in seconds of game time.
pub const RING_OF_FIRE_INTERVAL: f32 = 0.5;

/// The offset of `CTFMinigun::m_flNextRingOfFireAttackTime` in miniguns: the
/// game time after which the ring of fire may burn next, or 0 before the
/// barrel first spun up.
pub const RING_OF_FIRE_TIME_OFFSET: usize =
	offset_of!(sys::CTFMinigun, m_flNextRingOfFireAttackTime);

/// The offset of `CTFMinigun::m_iWeaponState` in miniguns, whose network
/// property's offset should match it.
pub const STATE_OFFSET: usize = offset_of!(sys::CTFMinigun, m_iWeaponState);

/// The state of `minigun`'s barrel, a `MinigunState_t`, and the game time
/// after which its ring of fire may burn next, both as the minigun keeps them.
///
/// # Safety
///
/// `minigun` is a live `CTFMinigun`, which nothing writes to during the call.
#[doc(alias("m_iWeaponState", "m_flNextRingOfFireAttackTime"))]
pub unsafe fn ring_of_fire(minigun: NonNull<sys::CBaseEntity>) -> (c_uint, f32) {
	let minigun = minigun.as_ptr().cast::<u8>();

	// SAFETY: The caller vouches for the minigun, whose members are at these
	// offsets of the generated layout. Both are 4-byte members, read as the game
	// reads them; `MinigunState_t` is a C enum, whose values fit an unsigned int.
	unsafe {
		(
			minigun.byte_add(STATE_OFFSET).cast::<c_uint>().read(),
			minigun
				.byte_add(RING_OF_FIRE_TIME_OFFSET)
				.cast::<f32>()
				.read(),
		)
	}
}
