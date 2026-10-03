//! Hand-written values of TF2's player classes: the `ETFClass` constants of
//! `game/shared/tf/tf_shareddefs.h`, which the generated bindings omit.

use std::ffi::c_int;

/// The Civilian, which players cannot choose.
pub const TF_CLASS_CIVILIAN: c_int = 10;

/// The Demoman.
pub const TF_CLASS_DEMOMAN: c_int = 4;

/// The Engineer, the last class players can choose.
pub const TF_CLASS_ENGINEER: c_int = 9;

/// The Heavy.
pub const TF_CLASS_HEAVYWEAPONS: c_int = 6;

/// The Medic.
pub const TF_CLASS_MEDIC: c_int = 5;

/// The Pyro.
pub const TF_CLASS_PYRO: c_int = 7;

/// The Scout, the first class players can choose.
pub const TF_CLASS_SCOUT: c_int = 1;

/// The Sniper.
pub const TF_CLASS_SNIPER: c_int = 2;

/// The Soldier.
pub const TF_CLASS_SOLDIER: c_int = 3;

/// The Spy.
pub const TF_CLASS_SPY: c_int = 8;

/// No class, as before a player first chooses one.
pub const TF_CLASS_UNDEFINED: c_int = 0;
