//! TF2's player respawns: how the game brings a player back, dead or not, and
//! where to intercept it.
//!
//! Every respawn of a player goes through `CTFPlayer::ForceRespawn`, which the
//! game calls through the player's vtable: respawn waves, a round's restart,
//! `game_forcerespawn`'s inputs, choosing a class, and the script bindings'
//! `ForceRespawn` included. Only direct calls of the player's `Spawn` bypass
//! it: a player's first spawn as it is put in the server, and scripts'
//! `DispatchSpawn` of a player.
//!
//! [`sdk_raw::tf2::respawn`] holds the function's signature and its vtable
//! slot, [`FORCE_RESPAWN_SLOT`](sdk_raw::tf2::respawn::FORCE_RESPAWN_SLOT).
//! `metamod_source`'s `respawn_hooks` hook it, to see every such respawn
//! before the game and to refuse it. [`force_respawn`] calls it, to respawn a
//! player as the game would.

#[cfg(test)]
#[path = "../tests/tf2/respawn.rs"]
mod tests;

use crate::entities::Entity;
use crate::{Game, Server};
use sdk_raw::vcall;

/// Respawns a TF2 player or bot as the game's own respawns do, through its
/// `CTFPlayer::ForceRespawn`, dead or alive.
///
/// The game spawns the player afresh as the class they chose
/// (`m_iDesiredPlayerClass`), and removes their buildings and projectiles if
/// that changes their class. It does nothing for a player who has chosen no
/// class, and, in arena, nothing during its own pre-game (`GR_STATE_PREGAME`)
/// or while it waits for players without arena's queue (`tf_arena_use_queue`
/// 0). Otherwise it spawns them whatever the round's state, arena's stalemate
/// included, and whatever their team: it does not check that they are on a
/// playing team, so check that first, as the game's own callers do.
///
/// Hooks of `ForceRespawn` see the call as they see the game's own respawns,
/// and may refuse it, as `metamod_source`'s `respawn_hooks` can. Spawning runs
/// the game's, other plugins', and this plugin's own spawn callbacks
/// synchronously, before this returns. They must keep to the contract of
/// [`Server::new`].
///
/// # Errors
///
/// Fails with [`RespawnError::NotTfPlayer`] unless the server runs TF2 and
/// `player`'s datamaps include `CTFPlayer`, and with
/// [`RespawnError::MarkedForDeletion`] for a player marked for deletion,
/// before the game is called.
#[doc(alias("ForceRespawn"))]
pub fn force_respawn(server: Server<'_>, player: Entity<'_>) -> Result<(), RespawnError> {
	if server.game() != Game::TeamFortress2 || !player.has_data_map_class(c"CTFPlayer") {
		return Err(RespawnError::NotTfPlayer);
	}

	if player.is_marked_for_deletion() {
		return Err(RespawnError::MarkedForDeletion);
	}

	let player = player.as_ptr().cast::<sys::CTFPlayer>();

	// SAFETY: The checks above found `CTFPlayer` in the player's datamaps, so
	// it is one, whose entity base `sdk_raw::tf2` asserts is at offset zero,
	// and whose generated TF2 vtable has this entry under the target's ABI, at
	// the slot `sdk_raw::tf2::respawn` checks. The callback keeps the player
	// allocated, and the callbacks its spawn runs are bound by `Server::new`'s
	// contract, which lets them free entities only through deferred deletion.
	unsafe { vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_ForceRespawn()) };

	Ok(())
}

/// Why [`force_respawn`] could not respawn a player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RespawnError {
	/// The player is marked for deletion.
	#[error("the player is marked for deletion")]
	MarkedForDeletion,

	/// The entity is not a TF2 player, or the server does not run TF2.
	#[error("respawning requires a TF2 player")]
	NotTfPlayer,
}
