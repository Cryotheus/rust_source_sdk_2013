//! `IServerGameClients`, the game's handling of connected clients.

use sdk_raw::vcall;
use std::ffi::c_int;

interface! {
	/// The game's handling of connected clients (`IServerGameClients`).
	///
	/// A plugin hooks its `ClientCommand`, at
	/// [`CLIENT_COMMAND_SLOT`](sdk_raw::interfaces::server_game_clients::CLIENT_COMMAND_SLOT),
	/// to run clients' commands through
	/// [`route_client_command`](crate::commands::route_client_command).
	#[doc(alias("IServerGameClients", "CServerGameClients"))]
	pub struct ServerGameClients(sys::IServerGameClients) = GameServer sdk_raw::interfaces::server_game_clients::VERSION;
}

/// The player counts a game supports, from [`ServerGameClients::player_limits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerLimits {
	/// The fewest player slots `maxplayers` may set.
	pub minimum: c_int,

	/// The most player slots `maxplayers` may set.
	pub maximum: c_int,

	/// The player count used when `maxplayers` is not given.
	pub default: c_int,
}

impl<'s> ServerGameClients<'s> {
	/// The player counts the game supports for `maxplayers`.
	#[doc(alias("GetPlayerLimits"))]
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
