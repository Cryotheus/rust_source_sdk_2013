//! Identifiers for the clients connected to the server.

use std::error::Error;
use std::ffi::c_int;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::hint;
use std::num::NonZero;

/// The most clients the engine can host at once.
///
/// This is `ABSOLUTE_PLAYER_LIMIT` from `public/const.h`. The player of each
/// client uses the edict one past the client's slot, so no player's edict
/// index exceeds this limit.
pub const ABSOLUTE_PLAYER_LIMIT: c_int = 255;

const RAW_USER_ID_MAX: c_int = u16::MAX as c_int;

/// The server's identifier for a connected client, as used by game events.
///
/// The engine assigns each client a user ID when it connects. Game events
/// identify players by it, such as through the `userid` and `attacker` keys of
/// [`player_death`].
///
/// A user ID is never 0. Game events use 0 for "no player", such as for the
/// `attacker` of a player killed by the world, and the engine can report 0 for
/// a client slot nobody occupies.
///
/// Use [`ValveEngine::edict_of_user_id`] and [`ValveEngine::user_id_of_edict`]
/// to convert between user IDs and player edicts.
///
/// [`ValveEngine::edict_of_user_id`]: crate::interfaces::valve_engine::ValveEngine::edict_of_user_id
/// [`ValveEngine::user_id_of_edict`]: crate::interfaces::valve_engine::ValveEngine::user_id_of_edict
///
/// [`player_death`]: crate::interfaces::game_event::GameEventId::PlayerDeath
#[doc(alias = "userid")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct UserId(NonZero<u16>);

impl UserId {
	/// Wraps a user ID, or returns `None` for 0.
	pub const fn new(user_id: u16) -> Option<Self> {
		match NonZero::new(user_id) {
			Some(user_id) => Some(Self(user_id)),
			None => None,
		}
	}

	/// Converts the `int` representation used by the engine and game events.
	pub const fn from_raw(raw: c_int) -> Result<Self, InvalidUserId> {
		match raw {
			1..=RAW_USER_ID_MAX => Ok(Self(NonZero::new(raw as u16).unwrap())),
			erroneous => Err(InvalidUserId(erroneous)),
		}
	}

	pub const fn get(self) -> u16 {
		self.0.get()
	}

	/// Returns the `int` representation used by the engine and game events.
	pub const fn to_raw(self) -> c_int {
		self.0.get() as c_int
	}
}

impl Display for UserId {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		Display::fmt(&self.0, f)
	}
}

impl From<NonZero<u16>> for UserId {
	fn from(user_id: NonZero<u16>) -> Self {
		Self(user_id)
	}
}

impl From<UserId> for NonZero<u16> {
	fn from(user_id: UserId) -> Self {
		user_id.0
	}
}

impl From<UserId> for u16 {
	fn from(user_id: UserId) -> Self {
		user_id.get()
	}
}

impl From<UserId> for c_int {
	fn from(user_id: UserId) -> Self {
		user_id.to_raw()
	}
}

impl TryFrom<c_int> for UserId {
	type Error = InvalidUserId;

	fn try_from(raw: c_int) -> Result<Self, Self::Error> {
		Self::from_raw(raw)
	}
}

/// An `int` which is not a [`UserId`].
///
/// `-1` is used as a sentinel value by the engine to indicate an unassigned
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(transparent)]
pub struct InvalidUserId(c_int);

impl InvalidUserId {
	/// Returns `true` if the underlying value represents any of the non-player states such as "no player" or "world" staes.
	///
	/// If `false` is returned, the value is seen as having originating from somewhere other than the engine,
	/// and may be worthy of a panic.
	pub const fn is_sentinel(&self) -> bool {
		matches!(self.0, -1 | 0)
	}
}

impl Display for InvalidUserId {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		const C_INT_MIN: c_int = c_int::MIN;

		match self.0 {
			// Represents the user id of a player entity, even if they haven't been created yet
			1..=RAW_USER_ID_MAX => unreachable!(),

			// Sentinel used by the engine for unassigned user ids
			-1 => write!(f, "No edict assigned"),

			// All other negative values and zero are considered impossible representations.
			value @ (C_INT_MIN..-1 | 0) => {
				hint::cold_path();
				write!(f, "Invalid UserId representation: {value}")
			}

			// Positive value greater than the 16-bit networkable value.
			value => write!(f, "Non-networkable UserId {value}"),
		}
	}
}

impl Error for InvalidUserId {}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn raw_conversion_accepts_only_non_zero_16_bit_values() {
		assert_eq!(UserId::from_raw(-1), Err(InvalidUserId(-1)));
		assert_eq!(UserId::from_raw(c_int::MIN), Err(InvalidUserId(c_int::MIN)));
		assert_eq!(UserId::from_raw(0), Err(InvalidUserId(0)));
		assert_eq!(UserId::from_raw(1).map(UserId::get), Ok(1));
		assert_eq!(UserId::from_raw(65_535).map(UserId::get), Ok(u16::MAX));
		assert_eq!(UserId::from_raw(65_536), Err(InvalidUserId(65_536)));
		assert_eq!(UserId::new(0), None);
		assert_eq!(UserId::new(7), UserId::from_raw(7).ok());

		assert_eq!(UserId::try_from(0), Err(InvalidUserId(0)));
		assert_eq!(UserId::try_from(-1), Err(InvalidUserId(-1)));
		assert_eq!(
			UserId::try_from(3),
			Ok(UserId::from(NonZero::new(3).unwrap()))
		);
		assert_eq!(c_int::from(UserId::new(u16::MAX).unwrap()), 65_535);
		assert_eq!(u16::from(UserId::new(7).unwrap()), 7);
		assert_eq!(UserId::new(42).unwrap().to_raw(), 42);
		assert_eq!(UserId::new(42).unwrap().to_string(), "42");
	}

	#[test]
	fn invalid_user_ids_describe_why_they_are_invalid() {
		assert!(InvalidUserId(0).is_sentinel());
		assert!(InvalidUserId(-1).is_sentinel());

		assert_eq!(InvalidUserId(-1).to_string(), "No edict assigned");
		assert_eq!(
			InvalidUserId(65_536).to_string(),
			"Non-networkable UserId 65536"
		);
		assert_eq!(
			InvalidUserId(c_int::MAX).to_string(),
			format!("Non-networkable UserId {}", c_int::MAX)
		);
		assert_eq!(
			InvalidUserId(0).to_string(),
			"Invalid UserId representation: 0"
		);
		assert_eq!(
			InvalidUserId(-2).to_string(),
			"Invalid UserId representation: -2"
		);
		assert_eq!(
			InvalidUserId(c_int::MIN).to_string(),
			format!("Invalid UserId representation: {}", c_int::MIN)
		);
	}
}
