//! Values from `game/shared/in_buttons.h`: the bits of a player command's
//! `buttons`, one for each input the player holds down as their client
//! samples it, which the generated bindings omit.

use std::ffi::c_int;

// A command keeps its buttons in one `int`, and its impulse in one byte.
const _: () = {
	let _: fn(&sys::CUserCmd) -> c_int = |command| command.buttons;
	let _: fn(&sys::CUserCmd) -> u8 = |command| command.impulse;
};

/// `IN_ALT1`: an input no game of the SDK binds.
pub const IN_ALT1: c_int = 1 << 14;

/// `IN_ALT2`: an input no game of the SDK binds.
pub const IN_ALT2: c_int = 1 << 15;

/// `IN_ATTACK`: the primary attack.
pub const IN_ATTACK: c_int = 1 << 0;

/// `IN_ATTACK2`: the secondary attack.
pub const IN_ATTACK2: c_int = 1 << 11;

/// `IN_ATTACK3`: the special attack, such as TF2's `+attack3`.
pub const IN_ATTACK3: c_int = 1 << 25;

/// `IN_BACK`: moving back.
pub const IN_BACK: c_int = 1 << 4;

/// `IN_BULLRUSH`: an input no game of the SDK binds.
pub const IN_BULLRUSH: c_int = 1 << 22;

/// `IN_CANCEL`: cancelling, which no client of the SDK's games sends.
pub const IN_CANCEL: c_int = 1 << 6;

/// `IN_DUCK`: ducking.
pub const IN_DUCK: c_int = 1 << 2;

/// `IN_FORWARD`: moving forward.
pub const IN_FORWARD: c_int = 1 << 3;

/// `IN_GRENADE1`: the first grenade, which TF2 does not bind.
pub const IN_GRENADE1: c_int = 1 << 23;

/// `IN_GRENADE2`: the second grenade, which TF2 does not bind.
pub const IN_GRENADE2: c_int = 1 << 24;

/// `IN_JUMP`: jumping.
pub const IN_JUMP: c_int = 1 << 1;

/// `IN_LEFT`: turning left, with the keyboard.
pub const IN_LEFT: c_int = 1 << 7;

/// `IN_MOVELEFT`: strafing left.
pub const IN_MOVELEFT: c_int = 1 << 9;

/// `IN_MOVERIGHT`: strafing right.
pub const IN_MOVERIGHT: c_int = 1 << 10;

/// `IN_RELOAD`: reloading.
pub const IN_RELOAD: c_int = 1 << 13;

/// `IN_RIGHT`: turning right, with the keyboard.
pub const IN_RIGHT: c_int = 1 << 8;

/// `IN_RUN`: running.
pub const IN_RUN: c_int = 1 << 12;

/// `IN_SCORE`: showing the scoreboard.
pub const IN_SCORE: c_int = 1 << 16;

/// `IN_SPEED`: the speed key, held to sprint where a game has sprinting.
pub const IN_SPEED: c_int = 1 << 17;

/// `IN_USE`: using what the player looks at, which TF2 does not bind.
pub const IN_USE: c_int = 1 << 5;

/// `IN_WALK`: walking.
pub const IN_WALK: c_int = 1 << 18;

/// `IN_WEAPON1`: a bit weapons define.
pub const IN_WEAPON1: c_int = 1 << 20;

/// `IN_WEAPON2`: a bit weapons define.
pub const IN_WEAPON2: c_int = 1 << 21;

/// `IN_ZOOM`: zooming the HUD.
pub const IN_ZOOM: c_int = 1 << 19;
