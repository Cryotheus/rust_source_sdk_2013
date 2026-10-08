//! Header values of TF2's game events: the `death_flags` bits of
//! `player_death`, from `game/shared/tf/tf_shareddefs.h`.

use std::ffi::c_int;

/// The assister is dominating the victim.
pub const TF_DEATH_ASSISTER_DOMINATION: c_int = 0x0002;

/// The assister got revenge on the victim.
pub const TF_DEATH_ASSISTER_REVENGE: c_int = 0x0008;

/// The victim was killed by an Australium weapon.
pub const TF_DEATH_AUSTRALIUM: c_int = 0x0400;

/// The killer is dominating the victim.
pub const TF_DEATH_DOMINATION: c_int = 0x0001;

/// A feigned death, by a Spy's Dead Ringer.
pub const TF_DEATH_FEIGN_DEATH: c_int = 0x0020;

/// The death triggered a first blood.
pub const TF_DEATH_FIRST_BLOOD: c_int = 0x0010;

/// The victim was gibbed.
pub const TF_DEATH_GIBBED: c_int = 0x0080;

/// The death interrupted the victim doing an important game event, like
/// capturing a point or carrying the flag.
pub const TF_DEATH_INTERRUPTED: c_int = 0x0040;

/// The victim was a miniboss.
pub const TF_DEATH_MINIBOSS: c_int = 0x0200;

/// The victim died while in purgatory.
pub const TF_DEATH_PURGATORY: c_int = 0x0100;

/// The killer got revenge on the victim.
pub const TF_DEATH_REVENGE: c_int = 0x0004;
