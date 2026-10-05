//! TF2 players' reserve ammo, `m_iAmmo`, which pickups, dispensers and
//! resupply cabinets fill up to a max that the game computes from the
//! player's class and items (`CTFPlayer::GetMaxAmmo`).
//!
//! [`give_ammo`] gives a player reserve ammo as a pickup does, through the
//! game's own `CTFPlayer::GiveAmmo`, which applies that max.

use crate::Game;
use crate::datatables::NetPropError;
use crate::entities::Entity;
use crate::server::{InterfaceError, Server};
use sdk_raw::tf2::ammo as raw;
use std::ffi::c_int;

/// The most reserve ammo [`give_ammo`] gives at once, 2^20: beyond any
/// type's max, and small enough to stay an `int` once scaled by the metal
/// pickup multipliers of TF2's items, which stay below 64.
pub const MAX_GIVEN: c_int = 1 << 20;

/// Why reserve ammo could not be given.
#[derive(Debug, thiserror::Error)]
pub enum AmmoError {
	/// The server does not run TF2, or the entity is not a TF2 player.
	#[error("reserve ammo requires a TF2 server and a CTFPlayer entity")]
	NotTfPlayer,

	/// The count is negative or more than [`MAX_GIVEN`].
	#[error("cannot give {0} reserve ammo at once, only from 0 to {MAX_GIVEN}")]
	InvalidCount(c_int),

	/// The player holds less than none of the type, which the game cannot
	/// compute their room for.
	#[error("the player holds {0} of the ammo type, less than none")]
	NegativeReserve(c_int),

	/// The game DLL does not export an interface it needs.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The player's reserve ammo could not be found or read.
	#[error(transparent)]
	NetProp(#[from] NetPropError),
}

/// A TF2 ammo type that players hold in reserve, by its index in `m_iAmmo`
/// (`ETFAmmoType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AmmoType {
	/// `TF_AMMO_PRIMARY`, the reserve of most primary weapons.
	#[doc(alias("TF_AMMO_PRIMARY"))]
	Primary,

	/// `TF_AMMO_SECONDARY`, the reserve of most secondary weapons.
	#[doc(alias("TF_AMMO_SECONDARY"))]
	Secondary,

	/// `TF_AMMO_METAL`, the Engineer's metal.
	#[doc(alias("TF_AMMO_METAL"))]
	Metal,

	/// `TF_AMMO_GRENADES1`, which some consumables count their uses in.
	#[doc(alias("TF_AMMO_GRENADES1"))]
	Grenades1,

	/// `TF_AMMO_GRENADES2`, which other consumables count their uses in.
	#[doc(alias("TF_AMMO_GRENADES2"))]
	Grenades2,

	/// `TF_AMMO_GRENADES3`, for throwables in the action slot.
	#[doc(alias("TF_AMMO_GRENADES3"))]
	Grenades3,
}

impl AmmoType {
	/// The type at `index` in `m_iAmmo`, such as a weapon's
	/// `m_iPrimaryAmmoType`, or `None` for the unused `TF_AMMO_DUMMY`, a
	/// weapon's -1 for none, and indices past the last type.
	pub const fn from_raw(index: c_int) -> Option<Self> {
		match index {
			raw::AMMO_PRIMARY => Some(Self::Primary),
			raw::AMMO_SECONDARY => Some(Self::Secondary),
			raw::AMMO_METAL => Some(Self::Metal),
			raw::AMMO_GRENADES1 => Some(Self::Grenades1),
			raw::AMMO_GRENADES2 => Some(Self::Grenades2),
			raw::AMMO_GRENADES3 => Some(Self::Grenades3),
			_ => None,
		}
	}

	/// The type's index in `m_iAmmo`.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Primary => raw::AMMO_PRIMARY,
			Self::Secondary => raw::AMMO_SECONDARY,
			Self::Metal => raw::AMMO_METAL,
			Self::Grenades1 => raw::AMMO_GRENADES1,
			Self::Grenades2 => raw::AMMO_GRENADES2,
			Self::Grenades3 => raw::AMMO_GRENADES3,
		}
	}
}

/// Gives `player` up to `count` reserve ammo of `ammo_type`, as a pickup
/// does (`CTFPlayer::GiveAmmo`), and returns how much they gained: never
/// more than the room left below the max of their class and items, and
/// nothing if they hold that max or more.
///
/// Metal counts are first scaled by the player's `mult_metal_pickup`
/// attribute. Consumables' types are refused to players whose items deny
/// their resupply. A gain plays the pickup sound, unless `suppress_sound`,
/// and fires the `ammo_pickup` event.
///
/// Fails with [`AmmoError::NotTfPlayer`] unless the server runs TF2 and
/// `player` is a TF2 player, and with [`AmmoError::InvalidCount`] for a
/// count outside 0 to [`MAX_GIVEN`].
#[doc(alias("GiveAmmo"))]
pub fn give_ammo(
	server: Server<'_>,
	player: Entity<'_>,
	ammo_type: AmmoType,
	count: c_int,
	suppress_sound: bool,
) -> Result<c_int, AmmoError> {
	if server.game() != Game::TeamFortress2
		|| !player
			.server_class()
			.is_some_and(|class| class.name() == c"CTFPlayer")
	{
		return Err(AmmoError::NotTfPlayer);
	}

	if !(0..=MAX_GIVEN).contains(&count) {
		return Err(AmmoError::InvalidCount(count));
	}

	let index = ammo_type.to_raw();
	let held = server
		.server_game_dll()?
		.entity_net_prop(player, c"m_iAmmo")?
		.element(index as usize)?
		.get::<c_int>(player)?;

	if held < 0 {
		return Err(AmmoError::NegativeReserve(held));
	}

	// SAFETY: The player is a live TF2 player during `'s`, on the main thread,
	// whose class belongs to TF2's loaded game DLL (`Server::new` condition 2).
	// The type is below `TF_AMMO_COUNT`. The count is at most 2^20, so scaled
	// by a metal pickup multiplier, which TF2's items keep below 64, it stays
	// an `int`, and the reserve is not negative. Attribute values outside
	// those domains can only be set with unsafe calls whose callers rule them
	// out. The gain, including TF2's attribute hooks and the `ammo_pickup`
	// event, frees no entity (`Server::new` condition 4).
	Ok(unsafe { raw::give_ammo(player.as_ptr(), count, index, suppress_sound) })
}
