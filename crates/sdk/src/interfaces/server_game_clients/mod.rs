//! `IServerGameClients`, the game's handling of connected clients.

use crate::ffi::vcall;
use std::ffi::c_int;

interface! {
	/// The game's handling of connected clients (`IServerGameClients`).
	#[doc(alias = "IServerGameClients")]
	pub struct ServerGameClients(sys::IServerGameClients) = GameServer c"ServerGameClients005";
}

/// The player counts a game supports, from [`ServerGameClients::player_limits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerLimits {
	pub minimum: c_int,
	pub maximum: c_int,

	/// The player count used when `maxplayers` is not given.
	pub default: c_int,
}

impl<'s> ServerGameClients<'s> {
	#[doc(alias = "GetPlayerLimits")]
	pub fn player_limits(self) -> PlayerLimits {
		let (mut minimum, mut maximum, mut default) = (0, 0, 0);

		// SAFETY: `Server::new` guarantees the interface is live, and each
		// output is a local.
		unsafe {
			vcall!(self.as_ptr() => IServerGameClients_GetPlayerLimits(&mut minimum, &mut maximum, &mut default))
		};

		PlayerLimits {
			minimum,
			maximum,
			default,
		}
	}
}
