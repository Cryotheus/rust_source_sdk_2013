//! TF2 players' reserve ammo, `m_iAmmo`, which pickups, dispensers and
//! resupply cabinets fill up to a max that the game computes from the
//! player's class and items (`CTFPlayer::GetMaxAmmo`).
//!
//! [`give_ammo`] gives a player reserve ammo as a pickup does, through the
//! game's own `CTFPlayer::GiveAmmo`, which applies that max. [`reserve`] and
//! [`set_reserve`] read and set the reserve itself, past the max if need be,
//! and [`max_reserve`] reads the max.

use crate::Game;
use crate::datatables::{NetProp, NetPropError};
use crate::entities::Entity;
use crate::server::{InterfaceError, Server};
use crate::tf2::game_rules::{GameRules, GameRulesError};
use crate::tf2::player::TfPlayer;
use sdk_raw::tf2::ammo as raw;
use std::ffi::c_int;
use std::ptr::NonNull;

/// The most reserve ammo [`give_ammo`] gives at once, 2^20: beyond any
/// type's max, and small enough to stay an `int` once scaled by the metal
/// pickup multipliers of TF2's items, which stay below 64.
pub const MAX_GIVEN: c_int = 1 << 20;

/// Why reserve ammo could not be given, read or set.
#[derive(Debug, thiserror::Error)]
pub enum AmmoError {
	/// The server does not run TF2, or the entity is not a TF2 player.
	#[error("reserve ammo requires a TF2 server and a CTFPlayer entity")]
	NotTfPlayer,

	/// The count is negative or more than [`MAX_GIVEN`].
	#[error("cannot give {0} reserve ammo at once, only from 0 to {MAX_GIVEN}")]
	InvalidCount(c_int),

	/// [`set_reserve`] was given a negative count.
	#[error("cannot hold {0} reserve ammo, less than none")]
	InvalidReserve(c_int),

	/// The player holds less than none of the type, which the game cannot
	/// compute their room for.
	#[error("the player holds {0} of the ammo type, less than none")]
	NegativeReserve(c_int),

	/// The game rules, which [`max_reserve`] asks, could not be found.
	#[error(transparent)]
	GameRules(#[from] GameRulesError),

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

/// Fails with [`AmmoError::NotTfPlayer`] unless the server runs TF2 and
/// `player`'s server class is `CTFPlayer`.
fn check_player(server: Server<'_>, player: Entity<'_>) -> Result<(), AmmoError> {
	if server.game() != Game::TeamFortress2
		|| !player
			.server_class()
			.is_some_and(|class| class.name() == c"CTFPlayer")
	{
		return Err(AmmoError::NotTfPlayer);
	}

	Ok(())
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
	check_player(server, player)?;

	if !(0..=MAX_GIVEN).contains(&count) {
		return Err(AmmoError::InvalidCount(count));
	}

	let held = reserve_prop(server, player, ammo_type)?.get::<c_int>(player)?;

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
	Ok(unsafe { raw::give_ammo(player.as_ptr(), count, ammo_type.to_raw(), suppress_sound) })
}

/// The lowest count, from none up, for which `room` answers that there is no
/// room left, given that it answers that there is room for exactly the counts
/// below some max, or [`c_int::MAX`] if it answers that there is room even
/// then.
///
/// It doubles a count from none until one has no room, then halves the range
/// below that count, so a max of `n` takes about `2 log2(n)` answers.
fn lowest_without_room<E>(mut room: impl FnMut(c_int) -> Result<bool, E>) -> Result<c_int, E> {
	// Every count below `low` has room. Once found, no count from `high` up has.
	let mut low = 0;
	let mut high = 0;

	while room(high)? {
		if high == c_int::MAX {
			return Ok(c_int::MAX);
		}

		low = high + 1;
		high = high.saturating_mul(2).max(1);
	}

	while low < high {
		let middle = low + (high - low) / 2;

		if room(middle)? {
			low = middle + 1;
		} else {
			high = middle;
		}
	}

	Ok(high)
}

/// The most reserve ammo of `ammo_type` that `player` has room for, as their
/// class and items make it (`CTFPlayer::GetMaxAmmo`): what pickups,
/// dispensers and resupply cabinets fill their reserve up to, and where
/// [`give_ammo`] stops. A max below none reads as none.
///
/// The game computes the max only within other calls, so this asks the game
/// rules whether the player has room for more
/// ([`GameRules::can_have_ammo`]) with their reserve set to other counts, and
/// returns the lowest count that leaves no room. It puts their reserve back
/// before it returns, and records none of these counts as changes, so clients
/// never see them. Hooks of `CanHaveAmmo` see each count asked about, and what
/// they answer decides the max read.
///
/// While the game tests items, bots count as holding 999 of every type,
/// whatever their reserve, so their max reads as none, or as [`c_int::MAX`] if
/// it is more than 999.
///
/// Fails with [`AmmoError::NotTfPlayer`] unless the server runs TF2 and
/// `player` is a TF2 player, and with [`AmmoError::GameRules`] if the game
/// rules cannot be found, as before a level's entities are created.
#[doc(alias("GetMaxAmmo", "max_ammo"))]
pub fn max_reserve(
	server: Server<'_>,
	player: Entity<'_>,
	ammo_type: AmmoType,
) -> Result<c_int, AmmoError> {
	check_player(server, player)?;

	let tf_player = TfPlayer::new(server, player).map_err(|_| AmmoError::NotTfPlayer)?;
	let rules = GameRules::get(server)?;
	let prop = reserve_prop(server, player, ammo_type)?;
	let held = prop.get::<c_int>(player)?;

	// SAFETY: An entity's pointer is never null.
	let base = unsafe { NonNull::new_unchecked(player.as_ptr()) }.cast();

	let max = lowest_without_room(|count| {
		// SAFETY: The player's class derives from the one the reserve was
		// resolved in, which reading the held count checked, so the reserve is
		// at its offset from the player. The game holds any count from none up, as for
		// `set_reserve`, and only `CanHaveAmmo` reads this one before the held
		// count is put back.
		unsafe { prop.set_at(base, count) }?;

		Ok::<_, AmmoError>(rules.can_have_ammo(tf_player, ammo_type))
	});

	// SAFETY: As above, for the count the game held.
	unsafe { prop.set_at(base, held) }?;

	max
}

/// How much reserve ammo of `ammo_type` `player` holds (`m_iAmmo`), which
/// can be more than the max of their class and items, if set so.
///
/// Fails with [`AmmoError::NotTfPlayer`] unless the server runs TF2 and
/// `player` is a TF2 player.
#[doc(alias("GetAmmoCount", "m_iAmmo"))]
pub fn reserve(
	server: Server<'_>,
	player: Entity<'_>,
	ammo_type: AmmoType,
) -> Result<c_int, AmmoError> {
	check_player(server, player)?;

	Ok(reserve_prop(server, player, ammo_type)?.get::<c_int>(player)?)
}

/// Resolves the element of `m_iAmmo` that holds `ammo_type`.
fn reserve_prop<'s>(
	server: Server<'s>,
	player: Entity<'s>,
	ammo_type: AmmoType,
) -> Result<NetProp<'s>, AmmoError> {
	Ok(server
		.server_game_dll()?
		.entity_net_prop(player, c"m_iAmmo")?
		.element(ammo_type.to_raw() as usize)?)
}

/// Sets how much reserve ammo of `ammo_type` `player` holds (`m_iAmmo`), as
/// the game's `SetAmmoCount` does, and records the change so the engine
/// sends it to the player.
///
/// Any count from none up is kept, more than the max of the player's class
/// and items included: pickups, dispensers and resupply cabinets then give
/// them nothing more of it, but take none away, while firing spends it as
/// usual.
///
/// Fails with [`AmmoError::NotTfPlayer`] unless the server runs TF2 and
/// `player` is a TF2 player, and with [`AmmoError::InvalidReserve`] for a
/// negative count, before anything is written.
#[doc(alias("SetAmmoCount", "m_iAmmo"))]
pub fn set_reserve(
	server: Server<'_>,
	player: Entity<'_>,
	ammo_type: AmmoType,
	count: c_int,
) -> Result<(), AmmoError> {
	check_player(server, player)?;

	if count < 0 {
		return Err(AmmoError::InvalidReserve(count));
	}

	let engine = server.valve_engine()?;

	// SAFETY: The game sets reserve ammo to any count from none up, as
	// `SetAmmoCount` does, and computes the room left below the max with it.
	unsafe { reserve_prop(server, player, ammo_type)?.set(engine, player, count) }?;

	Ok(())
}
