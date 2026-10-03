//! Shorthands for the engine's identifiers of players.

use crate::players::UserId;

/// The user ID `id`.
///
/// For tests only.
///
/// # Panics
///
/// If `id` is 0, which the engine never assigns.
pub fn user(id: u16) -> UserId {
	UserId::new(id).unwrap()
}
