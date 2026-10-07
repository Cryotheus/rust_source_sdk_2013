//! Mocks of what Team Fortress 2's game DLL adds to the SDK's.

pub mod game_rules;
pub mod objectives;
pub mod script_binding;

#[cfg(test)]
pub(crate) mod player;
