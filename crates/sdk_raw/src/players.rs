//! Values from `public/const.h` and `game/shared/shareddefs.h` about the
//! clients the engine hosts and their players, which the generated bindings
//! omit.

use std::ffi::c_int;
use std::mem::MaybeUninit;

// `CBaseEntity` keeps its life state in one `char`, which the `LIFE_*` values
// are read from as a byte.
const _: () = {
	let entity = MaybeUninit::<sys::CBaseEntity>::uninit();

	// SAFETY: The place is only projected to, never read.
	let life_state = unsafe { &raw const (*entity.as_ptr()).m_lifeState };

	assert!(pointee_size(life_state) == size_of::<u8>());
};

/// The most clients the engine can host at once.
///
/// This is `ABSOLUTE_PLAYER_LIMIT` from `public/const.h`. The player of each
/// client uses the edict one past the client's slot, so no player's edict
/// index exceeds this limit.
pub const ABSOLUTE_PLAYER_LIMIT: c_int = 255;

/// The team numbers of TF2's and other multiplayer games' own teams start
/// here, after the teams every game shares.
///
/// This is `FIRST_GAME_TEAM` from `game/shared/shareddefs.h`, one past
/// `LAST_SHARED_TEAM`, which is [`TEAM_SPECTATOR`].
pub const FIRST_GAME_TEAM: c_int = TEAM_SPECTATOR + 1;

/// The `m_lifeState` of a living entity.
///
/// This is `LIFE_ALIVE` from `public/const.h`.
pub const LIFE_ALIVE: u8 = 0;

/// The `m_lifeState` of a dead entity, lying still.
///
/// This is `LIFE_DEAD` from `public/const.h`.
pub const LIFE_DEAD: u8 = 2;

/// The `m_lifeState` of an entity still playing its death animation, or
/// falling until it hits the ground.
///
/// This is `LIFE_DYING` from `public/const.h`.
pub const LIFE_DYING: u8 = 1;

/// The `m_lifeState` of a dead player waiting to respawn, which TF2's players
/// take once their death animation and freeze cam are over
/// (`CTFPlayer::StateThinkDYING`).
///
/// This is `LIFE_RESPAWNABLE` from `public/const.h`.
pub const LIFE_RESPAWNABLE: u8 = 3;

/// The team number of spectators, the last of the teams every game shares.
///
/// This is `TEAM_SPECTATOR` from `game/shared/shareddefs.h`, which also names
/// it `LAST_SHARED_TEAM`.
#[doc(alias = "LAST_SHARED_TEAM")]
pub const TEAM_SPECTATOR: c_int = 1;

/// The team number of a player not assigned to any team yet.
///
/// This is `TEAM_UNASSIGNED` from `game/shared/shareddefs.h`.
pub const TEAM_UNASSIGNED: c_int = 0;

/// The size of the place `_place` points to.
const fn pointee_size<T>(_place: *const T) -> usize {
	size_of::<T>()
}
