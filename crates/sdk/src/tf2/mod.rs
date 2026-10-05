//! Team Fortress 2: the parts of its game and engine that other Source SDK 2013
//! games do not share.
//!
//! Everything here assumes a server running TF2, whose game DLL is built with
//! `TF_DLL` ([`Game::TeamFortress2`](crate::Game::TeamFortress2)). Most
//! wrappers check the [`Game`](crate::Game) and the entity classes they are
//! given, and return an error for anything else.

pub mod achievements;
pub mod attributes;
mod class;
pub mod conditions;
pub mod damage;
pub mod game_events;
pub mod gc;
pub mod host_timescale;
pub mod overlays;
pub mod ragdolls;
pub mod scoreboard;
mod script_binding;
pub mod sound;
pub mod user_messages;
pub mod voice;
pub mod voting;
pub mod weapons;
pub mod wearables;

#[cfg(feature = "tf2_loadout")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2_loadout")))]
pub mod loadout;

pub use class::PlayerClass;
