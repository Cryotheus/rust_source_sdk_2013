//! Hand-written ABI of TF2's player resource and scoreboard statistics.
//!
//! TF2's `CTFPlayerResource` (`game/server/tf/tf_player_resource.h`) and its
//! base `CPlayerResource` (`game/server/player_resource.h`) network one array
//! per scoreboard column, which the generated bindings do not lay out.

use crate::players::FIRST_GAME_TEAM;
use std::ffi::c_int;

// Each player slot's group of streaks includes its kill streak.
const _: () = assert!(KILL_STREAK < STREAKS_PER_SLOT);

/// Bytes between the elements of the player resource's arrays of `int`s
/// (`CNetworkArray( int, ... )`), which include every scoreboard column.
pub const ELEMENT_SIZE: usize = size_of::<c_int>();

/// The element of a player slot's `m_iStreaks` group the scoreboard shows:
/// `CTFPlayerShared::kTFStreak_Kills`.
#[doc(alias("kTFStreak_Kills"))]
pub const KILL_STREAK: usize = sys::CTFPlayerShared_ETFStreak_kTFStreak_Kills as usize;

/// `m_iStreaks` elements per player slot: `CTFPlayerShared::kTFStreak_COUNT`.
///
/// `CTFPlayerResource` keeps each slot's streaks together, the slot's group
/// starting at its index times this (`tf_player_resource.cpp:282`).
#[doc(alias("kTFStreak_COUNT"))]
pub const STREAKS_PER_SLOT: usize = sys::CTFPlayerShared_ETFStreak_kTFStreak_COUNT as usize;

/// BLU's team number.
///
/// This is `TF_TEAM_BLUE` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_TEAM_BLUE: c_int = TF_TEAM_RED + 1;

/// RED's team number, the first of the game's own teams.
///
/// This is `TF_TEAM_RED` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_TEAM_RED: c_int = FIRST_GAME_TEAM;
