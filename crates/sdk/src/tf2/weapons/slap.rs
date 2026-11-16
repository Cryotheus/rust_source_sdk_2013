//! The Hot Hand's second slap. Its smack (`CTFSlap::Smack`) runs the base
//! melee smack, and then, after the first slap of a swing (`m_bFirstHit`),
//! times a second smack with the weapon's secondary mode, which lands without
//! an animation of its own. A hook that replaces the smack replaces that
//! scheduling too; [`LandingSlap`] does it in the smack's place.
//!
//! The smack's timer (`CTFWeaponBaseMelee::m_flSmackTime`) is neither
//! networked nor in a datamap, so it is placed by the generated layout. In
//! Bravo's `server.dll`, `CTFSlap::Smack` writes the timer at 0x9e8 and
//! `m_bFirstHit` at 0xa10, as the generated Windows layout and the networked
//! variable place them; the Linux layout's weapon fields lie 0x28 further,
//! as the Shortstop's natively witnessed timer does ([`super::shortstop`]).
//! Each landing checks the layout again: `CTFWeaponBaseMelee::ItemPostFrame`
//! sets the timer to -1 just before it calls the smack, so a timer read
//! there as anything else refuses.

#[cfg(test)]
#[path = "../../tests/tf2/weapons/slap.rs"]
mod tests;

use super::{Weapon, check_live};
use crate::Server;
use crate::datatables::{NetProp, PropKind, ServerClass};
use crate::entities::Entity;
use sdk_raw::vcall;
use std::ffi::c_int;
use std::mem::offset_of;

/// What `CTFWeaponBaseMelee::ItemPostFrame` sets the smack's timer to as the
/// smack lands, and what a weapon whose smack is not timed holds.
const NO_SMACK: f32 = -1.0;

/// The weapon mode of the second slap (`TF_WEAPON_SECONDARY_MODE`).
const SECONDARY_MODE: c_int = 1;

/// A Hot Hand whose smack lands now, checked so that what `CTFSlap::Smack`
/// does after the base smack can be done in its place.
#[derive(Debug, Clone, Copy)]
pub struct LandingSlap<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
	first_hit: NetProp<'s>,
}

impl<'s> LandingSlap<'s> {
	/// Checks that `weapon` is a Hot Hand whose smack lands now, from its
	/// `ItemPostFrame`, such as in a hook before its `Smack`. Fails with
	/// [`SlapError::NotSlap`] for other weapons, and with
	/// [`SlapError::Layout`] for a smack that lands otherwise, or a game
	/// whose layout differs.
	pub fn new(weapon: Weapon<'s>) -> Result<Self, SlapError> {
		let entity = weapon.entity();

		// The server's `CTFSlap` declares no datamap of its own, so its class is
		// told by its network name.
		if check_live(entity).is_err()
			|| !entity.has_data_map_class(c"CTFWeaponBaseMelee")
			|| entity.server_class().map(ServerClass::name) != Some(c"CTFSlap")
		{
			return Err(SlapError::NotSlap);
		}

		let server = weapon.server;
		let first_hit = server
			.server_game_dll()
			.map_err(|_| SlapError::Interface)?
			.entity_net_prop(entity, c"m_bFirstHit")
			.map_err(|_| SlapError::Property)?;

		if first_hit.prop().kind() != PropKind::Int || first_hit.get::<bool>(entity).is_err() {
			return Err(SlapError::Property);
		}

		let landing = Self {
			server,
			entity,
			first_hit,
		};

		// The timer may not overlap the networked variable.
		let timer = smack_time_offset();

		if timer.abs_diff(first_hit.offset()) < size_of::<f32>()
			|| landing.smack_time().to_bits() != NO_SMACK.to_bits()
		{
			return Err(SlapError::Layout);
		}

		Ok(landing)
	}

	/// Does what `CTFSlap::Smack` does after the base smack: after the first
	/// slap of a swing, times the second, with the weapon's secondary mode
	/// (`GetSmackTime`, which hooks on it see). Whether it timed one.
	pub fn finish(self) -> Result<bool, SlapError> {
		check_live(self.entity).map_err(|_| SlapError::NotSlap)?;

		if !self
			.first_hit
			.get::<bool>(self.entity)
			.map_err(|_| SlapError::Property)?
		{
			return Ok(false);
		}

		let engine = self
			.server
			.valve_engine()
			.map_err(|_| SlapError::Interface)?;

		// SAFETY: `new` found the variable a networked bool of the weapon's
		// class, which the game sets to false itself here.
		unsafe { self.first_hit.set(engine, self.entity, false) }
			.map_err(|_| SlapError::Property)?;

		let this = self.entity.as_ptr().cast::<sys::CTFWeaponBaseMelee>();

		// SAFETY: The weapon is a live `CTFSlap`, which derives from
		// `CTFWeaponBaseMelee` through its primary bases, during the callback.
		// The generated `GetSmackTime` entry reads the weapon's script and the
		// current time.
		let time = unsafe {
			vcall!(this as sys::CTFWeaponBaseMelee__bindgen_vtable => CTFWeaponBaseMelee_GetSmackTime(SECONDARY_MODE))
		};

		// SAFETY: `new` checked that the generated offset holds the -1 the game
		// wrote there for this smack, within the live weapon. The game writes
		// the timer through its own pointers, on the main thread.
		unsafe {
			self.entity
				.as_ptr()
				.byte_add(smack_time_offset())
				.cast::<f32>()
				.write(time)
		};

		Ok(true)
	}

	/// The smack's timer, as the generated layout places it.
	fn smack_time(self) -> f32 {
		// SAFETY: The weapon is a live `CTFWeaponBaseMelee`, whose generated
		// layout places an aligned `float` at the offset, which `new` checks
		// holds what the game wrote there. It is read without forming a
		// reference.
		unsafe {
			self.entity
				.as_ptr()
				.byte_add(smack_time_offset())
				.cast::<f32>()
				.read()
		}
	}
}

/// Why [`LandingSlap::new`] refused a weapon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SlapError {
	/// The weapon is no Hot Hand (a melee weapon of the server class
	/// `CTFSlap`), or is marked for deletion.
	#[error("the weapon is not a live Hot Hand")]
	NotSlap,

	/// The game DLL's interfaces are unavailable.
	#[error("the game DLL's interfaces are unavailable")]
	Interface,

	/// `m_bFirstHit` is not a networked integer of the weapon's class.
	#[error("the Hot Hand's `m_bFirstHit` is not a usable networked variable")]
	Property,

	/// The smack's timer, where the generated layout places it, does not
	/// hold -1, as it does while a smack lands.
	#[error("the Hot Hand's smack timer does not hold -1 where its layout places it")]
	Layout,
}

/// The offset of `m_flSmackTime` in the generated layout of
/// `CTFWeaponBaseMelee`, whose entity base is at its start.
const fn smack_time_offset() -> usize {
	offset_of!(sys::CTFWeaponBaseMelee, m_flSmackTime)
}
