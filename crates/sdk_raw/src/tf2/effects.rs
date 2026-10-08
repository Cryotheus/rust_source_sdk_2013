//! Hand-written values of TF2's player effects: the stun flags of
//! `game/shared/tf/tf_shareddefs.h`, which `CTFPlayerShared::StunPlayer`
//! takes, and which the generated bindings omit, being macros.

use std::ffi::c_int;

/// The stun is a scare, as a Halloween ghost's or `trigger_stun`'s: the
/// player shows fright rather than stars, is slowed less in the losing state,
/// and hears the stun's sound only when not already stunned.
pub const TF_STUN_BY_TRIGGER: c_int = 1 << 7;

/// The stun takes away the player's control, as a sapped robot's in Mann vs.
/// Machine.
pub const TF_STUN_CONTROLS: c_int = 1 << 1;

/// Declared for the Scout's dodge, and unused by the game.
pub const TF_STUN_DODGE_COOLDOWN: c_int = 1 << 4;

/// The stun puts the player in the losing team's state after a round: slowed,
/// and unable to attack.
pub const TF_STUN_LOSER_STATE: c_int = 1 << 6;

/// The stun slows the player's movement.
pub const TF_STUN_MOVEMENT: c_int = 1 << 0;

/// The stun slows only the player's forward movement.
pub const TF_STUN_MOVEMENT_FORWARD_ONLY: c_int = 1 << 2;

/// The stun shows no particles over the player's head.
pub const TF_STUN_NO_EFFECTS: c_int = 1 << 5;

/// No stun flags.
pub const TF_STUN_NONE: c_int = 0;

/// The stun plays its sound.
pub const TF_STUN_SOUND: c_int = 1 << 8;

/// The stun plays the sound of a long-range stun.
pub const TF_STUN_SPECIAL_SOUND: c_int = 1 << 3;
