//! TF2's miniguns (`CTFMinigun`): the state of their barrel, and when the ring
//! of fire of those that have one may burn next.
//!
//! A minigun with the `ring_of_fire_while_aiming` attribute, such as the
//! Huo-Long Heater, burns the enemy players around its Heavy every half second
//! while its barrel spins or fires and the Heavy has ammo
//! (`CTFMinigun::RingOfFireAttack`), and only players. [`RingOfFire`] reads
//! when the next ring may burn, from where the game keeps it, which nothing
//! networks: comparing it before and after the minigun's `ItemPostFrame` tells
//! whether a ring burned in it.

#[cfg(test)]
#[path = "../../tests/tf2/weapons/minigun.rs"]
mod tests;

use crate::entities::Entity;
use crate::{Game, Server};
use sdk_raw::tf2::minigun as raw;
use std::ffi::{CStr, c_uint};
use std::ptr::NonNull;

/// The C++ class of miniguns.
const CLASS: &CStr = c"CTFMinigun";

/// How long after a ring of fire burns, or after the barrel leaves a state
/// that [delays it](MinigunState::delays_ring_of_fire), the next may burn, in
/// seconds of game time.
pub const RING_OF_FIRE_INTERVAL: f32 = raw::RING_OF_FIRE_INTERVAL;

/// What went wrong reading a minigun.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MinigunError {
	/// The game has no minigun class, or its server DLL interface is missing.
	#[error("the game has no networked `CTFMinigun` class")]
	Class,

	/// The minigun's networked barrel state does not lie where the generated
	/// layout puts it, so the layout does not match the running game.
	#[error(
		"`CTFMinigun::m_iWeaponState` lies at {networked}, not at {generated} as the generated layout has it"
	)]
	Layout {
		/// The offset its network property gives.
		networked: usize,

		/// The offset in the generated layout.
		generated: usize,
	},

	/// The entity is not a live minigun of a TF2 server.
	#[error("the entity is not a live TF2 minigun")]
	NotMinigun,
}

/// The layout of the running game's miniguns, checked against the network
/// property of their barrel's state.
#[derive(Debug, Clone, Copy)]
pub struct MinigunLayout {
	_checked: (),
}

impl MinigunLayout {
	/// Checks that the minigun's networked barrel state,
	/// `CTFMinigun::m_iWeaponState`, lies where the generated layout puts it.
	/// The ring of fire's time lies a few members after it in the same class,
	/// which [`RingOfFire::confirms_layout`] can confirm as the game runs.
	pub fn load(server: Server<'_>) -> Result<Self, MinigunError> {
		if server.game() != Game::TeamFortress2 {
			return Err(MinigunError::Class);
		}

		let dll = server.server_game_dll().map_err(|_| MinigunError::Class)?;
		let class = dll.server_class(CLASS).ok_or(MinigunError::Class)?;
		let prop = dll
			.net_prop(class, c"m_iWeaponState")
			.map_err(|_| MinigunError::Class)?;

		if prop.offset() != raw::STATE_OFFSET {
			return Err(MinigunError::Layout {
				networked: prop.offset(),
				generated: raw::STATE_OFFSET,
			});
		}

		Ok(Self { _checked: () })
	}

	/// The state of `minigun`'s barrel and when its ring of fire may burn
	/// next. Fails with [`MinigunError::NotMinigun`] unless `minigun` is a
	/// live `CTFMinigun`.
	#[doc(alias("m_iWeaponState", "m_flNextRingOfFireAttackTime"))]
	pub fn ring_of_fire(self, minigun: Entity<'_>) -> Result<RingOfFire, MinigunError> {
		if minigun.is_marked_for_deletion() || !minigun.has_data_map_class(CLASS) {
			return Err(MinigunError::NotMinigun);
		}

		// SAFETY: An entity's pointer is never null.
		let pointer = unsafe { NonNull::new_unchecked(minigun.as_ptr()) };

		// SAFETY: The entity is a live `CTFMinigun`, as its data maps tell, on the
		// main thread for this callback, on which alone the game writes its
		// members. `load` checked the layout this reads.
		let (state, next) = unsafe { raw::ring_of_fire(pointer) };

		Ok(RingOfFire {
			state: MinigunState::from_raw(state),
			next,
		})
	}
}

/// The state of a minigun's barrel (`MinigunState_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[doc(alias("MinigunState_t"))]
pub enum MinigunState {
	/// The barrel stands still (`AC_STATE_IDLE`).
	Idle,

	/// The barrel winds up (`AC_STATE_STARTFIRING`).
	StartFiring,

	/// The barrel spins and fires (`AC_STATE_FIRING`).
	Firing,

	/// The barrel spins without firing (`AC_STATE_SPINNING`).
	Spinning,

	/// The barrel spins, firing without ammo (`AC_STATE_DRYFIRE`).
	DryFire,
}

impl MinigunState {
	/// The state of the raw `MinigunState_t` value, if it is one.
	pub const fn from_raw(raw: c_uint) -> Option<Self> {
		Some(match raw {
			raw::AC_STATE_IDLE => Self::Idle,
			raw::AC_STATE_STARTFIRING => Self::StartFiring,
			raw::AC_STATE_FIRING => Self::Firing,
			raw::AC_STATE_SPINNING => Self::Spinning,
			raw::AC_STATE_DRYFIRE => Self::DryFire,
			_ => return None,
		})
	}

	/// Whether the game sets the ring of fire's time [half a second
	/// ahead](RING_OF_FIRE_INTERVAL) as the barrel leaves this state
	/// (`CTFMinigun::SetWeaponState`): idle, winding up or dry firing.
	pub const fn delays_ring_of_fire(self) -> bool {
		matches!(self, Self::Idle | Self::StartFiring | Self::DryFire)
	}
}

/// The state of a minigun's barrel and when its ring of fire may burn next,
/// as the minigun keeps them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RingOfFire {
	/// The barrel's state, or `None` for a value no state has.
	pub state: Option<MinigunState>,

	/// The game time after which the ring of fire may burn next, or 0 before
	/// the barrel first spun up.
	#[doc(alias("m_flNextRingOfFireAttackTime"))]
	pub next: f32,
}

impl RingOfFire {
	/// Whether a ring of fire burned between `before` and this, both read from
	/// one minigun around its `ItemPostFrame` at the game time `now`: the
	/// barrel spun or fired before, the ring was due, and its time moved
	/// [half a second](RING_OF_FIRE_INTERVAL) past `now`.
	///
	/// Only a burning ring moves the time while the barrel spins or fires;
	/// a change of state moves it only as the barrel leaves the idle, winding
	/// up or dry-firing states.
	pub fn burned_since(self, before: Self, now: f32) -> bool {
		matches!(
			before.state,
			Some(MinigunState::Firing | MinigunState::Spinning)
		) && before.next != 0.0
			&& before.next <= now
			&& self.next == now + RING_OF_FIRE_INTERVAL
	}

	/// Whether this, read after the minigun's `ItemPostFrame` at the game time
	/// `now`, confirms where the generated layout puts the ring's time, given
	/// what was read before: `Some(true)` if the barrel left a state that
	/// [delays the ring](MinigunState::delays_ring_of_fire) and the time moved
	/// [half a second](RING_OF_FIRE_INTERVAL) past `now`, `Some(false)` if it
	/// left one and the time did not, and `None` if the barrel did not leave
	/// one.
	pub fn confirms_layout(self, before: Self, now: f32) -> Option<bool> {
		let left = before.state.is_some_and(MinigunState::delays_ring_of_fire)
			&& self.state != before.state;

		left.then_some(self.next == now + RING_OF_FIRE_INTERVAL)
	}
}
