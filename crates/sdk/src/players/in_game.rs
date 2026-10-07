//! The players in the game, each with its client, edict and entity, and
//! kicking them.

#[cfg(test)]
#[path = "../tests/players/in_game.rs"]
mod tests;

use crate::edicts::Edict;
use crate::entities::Entity;
use crate::interfaces::{GameClient, ValveEngine};
use crate::players::UserId;
use crate::{InterfaceError, Server};
use std::ffi::{CStr, CString};

/// Why [`GameClient::kick`] queued no kick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KickError {
	/// The reason holds a character that would end the `kickid` command or
	/// start another: a control character, `"` or `;`.
	#[error("the kick's reason holds a character that would end its command")]
	InvalidReason,

	/// No player has the client's slot.
	#[error("no player has the client's slot")]
	NoPlayer,
}

/// A player in the game: a connected client of the server's, with its edict
/// and entity, as [`Server::players`] yields them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Player<'s> {
	/// The engine's client for the player's slot.
	pub client: GameClient<'s>,

	/// The player's edict, whose index is the slot's plus 1.
	pub edict: Edict<'s>,

	/// The player's entity.
	pub entity: Entity<'s>,
}

impl<'s> Player<'s> {
	/// Whether the player is a person, not a bot or SourceTV.
	pub fn is_human(self) -> bool {
		!self.client.is_fake()
	}

	/// The player's user ID, or `None` if the engine has given it none.
	pub fn user_id(self) -> Option<UserId> {
		self.client.user_id()
	}
}

impl<'s> GameClient<'s> {
	/// Queues `kickid` with the client's user ID, which disconnects the player
	/// showing it `reason`, or `Kicked by Console` for an empty one, when the
	/// server next runs its command buffer, normally on the next frame.
	///
	/// Unlike [`Self::disconnect`], nothing is freed before this returns.
	/// Fails if no player has the slot, or the reason holds a character that
	/// would end the command or start another.
	#[doc(alias("kickid", "KickClient"))]
	pub fn kick(self, engine: ValveEngine<'_>, reason: &CStr) -> Result<(), KickError> {
		let reason = reason.to_bytes();

		if reason
			.iter()
			.any(|&byte| byte.is_ascii_control() || matches!(byte, b'"' | b';'))
		{
			return Err(KickError::InvalidReason);
		}

		let user_id = self.user_id().ok_or(KickError::NoPlayer)?;
		let mut command = format!("kickid {}", user_id.to_raw()).into_bytes();

		if !reason.is_empty() {
			command.push(b' ');
			command.extend_from_slice(reason);
		}

		command.push(b'\n');
		engine.server_command(&CString::new(command).expect("the reason has no NUL"));
		Ok(())
	}
}

impl<'s> Server<'s> {
	/// Iterates over the players in the game: the clients that have
	/// connected, from the first slot, whose edicts hold their entities.
	/// A player still connecting may have no entity yet, and is left out.
	///
	/// Fails if the engine does not export its interface, and yields nothing
	/// without a game server.
	pub fn players(&self) -> Result<impl Iterator<Item = Player<'s>> + use<'s>, InterfaceError> {
		let engine = self.valve_engine()?;

		let players = engine
			.game_server()
			.into_iter()
			.flat_map(|server| server.clients())
			.filter(|client| client.is_connected())
			.filter_map(move |client| {
				let edict = engine.edict_of_index(client.entity_index())?;
				let entity = edict.entity()?;

				Some(Player {
					client,
					edict,
					entity,
				})
			});

		Ok(players)
	}
}
