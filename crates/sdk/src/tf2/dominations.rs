//! TF2's dominations: which players dominate which, and ending them.
//!
//! A player who kills another
//! [`TF_KILLS_DOMINATION`](sdk_raw::tf2::dominations::TF_KILLS_DOMINATION)
//! times without being killed by them in between dominates them, as does an
//! assister who helps as often. The kill feed and both players' HUDs announce
//! it, the dominated player's scoreboard marks their nemesis, and every
//! scoreboard shows how many players the dominating player dominates. Killing
//! one's nemesis is a revenge, which ends the domination.
//! [`sdk_raw::tf2::dominations`] describes where the game decides both, and
//! holds the slots of the methods that keep the count.
//!
//! [`PlayerDominations::end_all`] ends every domination of a player and over
//! them, as the game does as a player leaves, without the notices a revenge
//! brings. Nothing here starts one. The game decides each from the victim's
//! unanswered kills, which
//! [`PlayerScore::reset_unanswered_kills`](crate::tf2::scoreboard::PlayerScore::reset_unanswered_kills)
//! resets: done before each of a player's deaths is handled, it keeps every
//! kill of them from starting a domination, and so from being a revenge.
//!
//! # Unverified
//!
//! No outside source lists the vtable slots of the count's methods, which
//! come from the generated bindings. The 64-bit Windows `server.dll` holds
//! them at the same slots, but the Linux slots are unchecked, and no running
//! server has had its dominations ended yet.

#[cfg(test)]
#[path = "../tests/tf2/dominations.rs"]
mod tests;

use crate::datatables::{NetProp, NetPropError};
use crate::entities::Entity;
use crate::{Game, InterfaceError, Server};
use sdk_raw::tf2::scoreboard::MAX_PLAYERS_ARRAY_SAFE;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};

/// `m_Shared.m_bPlayerDominated`: whether the player dominates each player,
/// by entity index.
const DOMINATED: &CStr = c"m_bPlayerDominated";

/// `m_Shared.m_bPlayerDominatingMe`: whether each player, by entity index,
/// dominates the player.
const DOMINATING_ME: &CStr = c"m_bPlayerDominatingMe";

/// Why a player's dominations could not be read or changed.
#[derive(Debug, thiserror::Error)]
pub enum DominationError {
	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// A networked variable could not be read or written.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// The entity is not a TF2 player with an entity index from 1 to
	/// `MAX_PLAYERS`, or the server does not run TF2.
	#[error("dominations require a TF2 player")]
	NotTfPlayer,
}

/// A TF2 player's dominations, scoped to one engine callback.
#[derive(Debug, Clone, Copy)]
pub struct PlayerDominations<'s> {
	index: usize,
	player: Entity<'s>,
	server: Server<'s>,
}

impl<'s> PlayerDominations<'s> {
	/// Wraps a player's dominations, or returns
	/// [`DominationError::NotTfPlayer`] unless the server runs TF2,
	/// `player`'s datamaps include `CTFPlayer`, and its entity index is from 1
	/// to `MAX_PLAYERS`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, DominationError> {
		if server.game() != Game::TeamFortress2 || !player.has_data_map_class(c"CTFPlayer") {
			return Err(DominationError::NotTfPlayer);
		}

		let index = player
			.index()
			.filter(|index| (1..MAX_PLAYERS_ARRAY_SAFE).contains(index))
			.ok_or(DominationError::NotTfPlayer)?;

		Ok(Self {
			index,
			player,
			server,
		})
	}

	/// How many players the player dominates, as the game counts them
	/// (`CTFPlayer::GetNumberofDominations`), and as the player resource
	/// copies it into `m_iActiveDominations` for every scoreboard.
	#[doc(alias("GetNumberofDominations", "m_iNumberofDominations"))]
	pub fn count(self) -> c_int {
		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: `new` found `CTFPlayer` in the player's datamaps, so it is one,
		// whose entity base `sdk_raw::tf2` asserts is at offset zero, and whose
		// generated TF2 vtable has this entry under the target's ABI, at the slot
		// `sdk_raw::tf2::dominations` checks. The callback keeps the player
		// allocated, and the method only reads a field.
		unsafe {
			vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_GetNumberofDominations())
		}
	}

	/// Whether the player dominates `other`.
	#[doc(alias("IsPlayerDominated", "m_bPlayerDominated"))]
	pub fn dominates(self, other: Self) -> Result<bool, DominationError> {
		self.flag(self.net_prop(DOMINATED)?, other.index)
	}

	/// Ends the player's domination of `victim`, if any, as a revenge ends it,
	/// without the revenge: no event, notice, or statistic. Returns whether
	/// the player dominated them.
	///
	/// Both players' halves of the pair are cleared even if only one is set,
	/// and the player's count drops by one if they dominated `victim`.
	pub fn end(self, victim: Self) -> Result<bool, DominationError> {
		let dominated = self.dominates(victim)?;

		self.set_flag(self.net_prop(DOMINATED)?, victim.index, false)?;
		victim.set_flag(victim.net_prop(DOMINATING_ME)?, self.index, false)?;

		if dominated {
			self.set_count(self.count().saturating_sub(1));
		}

		Ok(dominated)
	}

	/// Ends every domination of the player and over them, as the game does as
	/// a player leaves (`CTFPlayer::RemoveNemesisRelationships`), without the
	/// notices a revenge brings, and returns how many ended.
	///
	/// The other players' halves of each pair are cleared too, and the counts
	/// of those who dominated the player drop by one each. The player's count
	/// becomes 0. A pair with an entity index that holds no TF2 player, such
	/// as one who left, is cleared on the player's side only.
	#[doc(alias("RemoveNemesisRelationships"))]
	pub fn end_all(self) -> Result<usize, DominationError> {
		let tools = self.server.server_tools()?;
		let dominated = self.net_prop(DOMINATED)?;
		let dominating_me = self.net_prop(DOMINATING_ME)?;
		let len = [dominated, dominating_me]
			.into_iter()
			.filter_map(NetProp::element_count)
			.fold(MAX_PLAYERS_ARRAY_SAFE, usize::min);
		let other_at = |index: usize| {
			let entity = tools.entity_by_index(c_int::try_from(index).ok()?)?;

			Self::new(self.server, entity)
				.ok()
				.filter(|other| other.index == index)
		};
		let mut ended = 0;

		for index in (1..len).filter(|&index| index != self.index) {
			if self.flag(dominated, index)? {
				self.set_flag(dominated, index, false)?;
				ended += 1;

				if let Some(other) = other_at(index) {
					other.set_flag(other.net_prop(DOMINATING_ME)?, self.index, false)?;
				}
			}

			if self.flag(dominating_me, index)? {
				self.set_flag(dominating_me, index, false)?;
				ended += 1;

				if let Some(other) = other_at(index) {
					other.set_flag(other.net_prop(DOMINATED)?, self.index, false)?;
					other.set_count(other.count().saturating_sub(1));
				}
			}
		}

		if self.count() != 0 {
			self.set_count(0);
		}

		Ok(ended)
	}

	/// Element `index` of the player's array `array`.
	fn flag(self, array: NetProp<'_>, index: usize) -> Result<bool, DominationError> {
		Ok(array.element(index)?.get::<bool>(self.player)?)
	}

	/// Whether `other` dominates the player.
	#[doc(alias("IsPlayerDominatingMe", "m_bPlayerDominatingMe"))]
	pub fn is_dominated_by(self, other: Self) -> Result<bool, DominationError> {
		self.flag(self.net_prop(DOMINATING_ME)?, other.index)
	}

	/// Resolves one of the player's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, DominationError> {
		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.player, name)?)
	}

	/// The player.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// Sets the player's count, which the game clamps from 0 to 100
	/// (`CTFPlayer::SetNumberofDominations`).
	fn set_count(self, count: c_int) {
		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: As for `count`. The method only clamps the count and stores
		// it in the player.
		unsafe {
			vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_SetNumberofDominations(count.max(0)))
		};
	}

	/// Sets element `index` of the player's array `array` to `value`, marking
	/// it changed for the player's client, unless it holds `value` already.
	fn set_flag(
		self,
		array: NetProp<'_>,
		index: usize,
		value: bool,
	) -> Result<(), DominationError> {
		let element = array.element(index)?;

		if element.get::<bool>(self.player)? != value {
			// SAFETY: Both values are ones the game assigns the flags itself.
			unsafe { element.set(self.server.valve_engine()?, self.player, value) }?;
		}

		Ok(())
	}
}
