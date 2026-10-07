//! TF2's class meters on players, and the Medic's medigun: cloak, rage, hype,
//! shield charge, drink and item charge meters, heads taken, revenge crits,
//! and an ÜberCharge with its healing.
//!
//! Meters are read and written through their networked variables, which the
//! game keeps in `CTFPlayerShared`: its script setters refuse some writes,
//! such as rage while it drains, so they are not used.

#[cfg(test)]
#[path = "../tests/tf2/meters.rs"]
mod tests;

use crate::datatables::{NetProp, NetVar};
use crate::entities::{Entity, EntityHandle};
use crate::tf2::effects::EffectError;
use crate::tf2::player_methods;
use crate::{Game, Server};
use std::ffi::{CStr, c_int};

/// The most heads `m_iDecapitations` networks, in 8 bits.
const DECAPITATIONS_MAX: c_int = 255;

/// The most a meter holds, as the game networks most of them.
const METER_MAX: f32 = 100.0;

/// A Medic's medigun, `tf_weapon_medigun`, within the current engine
/// callback: its ÜberCharge and its healing.
#[derive(Debug, Clone, Copy)]
pub struct Medigun<'s> {
	server: Server<'s>,
	weapon: Entity<'s>,
}

impl<'s> Medigun<'s> {
	/// Wraps `weapon` for medigun calls. Fails with
	/// [`EffectError::NotTfPlayer`] unless the server runs TF2 and `weapon`'s
	/// class name is `tf_weapon_medigun`.
	pub fn new(server: Server<'s>, weapon: Entity<'s>) -> Result<Self, EffectError> {
		if server.game() != Game::TeamFortress2 || weapon.class_name() != c"tf_weapon_medigun" {
			return Err(EffectError::NotTfPlayer);
		}

		Ok(Self { server, weapon })
	}

	/// The ÜberCharge, from 0 to 1 (`m_flChargeLevel`).
	#[doc(alias("m_flChargeLevel", "GetChargeLevel"))]
	pub fn charge(self) -> Result<f32, EffectError> {
		self.get(c"m_flChargeLevel")
	}

	/// Reads one of the medigun's networked variables.
	fn get<T: NetVar>(self, name: &CStr) -> Result<T, EffectError> {
		Ok(self.net_prop(name)?.get::<T>(self.weapon)?)
	}

	/// Reads one of the medigun's entity handles, `None` for none.
	fn get_handle(self, name: &CStr) -> Result<Option<EntityHandle>, EffectError> {
		let handle = self.net_prop(name)?.get_handle(self.weapon)?;

		Ok(handle.is_valid().then_some(handle))
	}

	/// What the medigun heals, if anything (`m_hHealingTarget`).
	#[doc(alias("m_hHealingTarget", "GetHealTarget"))]
	pub fn healing_target(self) -> Result<Option<EntityHandle>, EffectError> {
		self.get_handle(c"m_hHealingTarget")
	}

	/// Whether the Medic holds down the medigun's fire (`m_bAttacking`).
	#[doc(alias("m_bAttacking"))]
	pub fn is_attacking(self) -> Result<bool, EffectError> {
		self.get(c"m_bAttacking")
	}

	/// Whether the medigun heals something (`m_bHealing`).
	#[doc(alias("m_bHealing"))]
	pub fn is_healing(self) -> Result<bool, EffectError> {
		self.get(c"m_bHealing")
	}

	/// Whether the Medic has put the medigun away (`m_bHolstered`).
	#[doc(alias("m_bHolstered"))]
	pub fn is_holstered(self) -> Result<bool, EffectError> {
		self.get(c"m_bHolstered")
	}

	/// Whether the ÜberCharge is being released (`m_bChargeRelease`). This
	/// stays set while the medigun is holstered, when the game's
	/// `IsReleasingCharge` reports false.
	#[doc(alias("m_bChargeRelease"))]
	pub fn is_releasing(self) -> Result<bool, EffectError> {
		self.get(c"m_bChargeRelease")
	}

	/// What the medigun healed last, if anything (`m_hLastHealingTarget`).
	#[doc(alias("m_hLastHealingTarget"))]
	pub fn last_healing_target(self) -> Result<Option<EntityHandle>, EffectError> {
		self.get_handle(c"m_hLastHealingTarget")
	}

	/// Resolves one of the medigun's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, EffectError> {
		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.weapon, name)?)
	}

	/// The resistance the Vaccinator gives, or `None` for a number the game
	/// does not use (`m_nChargeResistType`).
	#[doc(alias("m_nChargeResistType", "GetResistType"))]
	pub fn resistance(self) -> Result<Option<MedigunResistance>, EffectError> {
		Ok(MedigunResistance::from_raw(
			self.get(c"m_nChargeResistType")?,
		))
	}

	/// Sets the ÜberCharge, from 0 to 1, as the game's own healing does. Fails
	/// with [`EffectError::OutOfRange`] outside that.
	#[doc(alias("SetChargeLevel"))]
	pub fn set_charge(self, charge: f32) -> Result<(), EffectError> {
		if !(0.0..=1.0).contains(&charge) {
			return Err(EffectError::OutOfRange("charge"));
		}

		let engine = self.server.valve_engine()?;
		let variable = self.net_prop(c"m_flChargeLevel")?;

		// SAFETY: The game keeps the charge between 0 and 1 itself.
		Ok(unsafe { variable.set(engine, self.weapon, charge) }?)
	}

	/// The medigun.
	pub const fn weapon(self) -> Entity<'s> {
		self.weapon
	}
}

/// The resistance the Vaccinator gives (`medigun_resist_types_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MedigunResistance {
	/// `MEDIGUN_BULLET_RESIST`: against bullets.
	#[doc(alias("MEDIGUN_BULLET_RESIST"))]
	Bullet = 0,

	/// `MEDIGUN_BLAST_RESIST`: against explosions.
	#[doc(alias("MEDIGUN_BLAST_RESIST"))]
	Blast = 1,

	/// `MEDIGUN_FIRE_RESIST`: against fire.
	#[doc(alias("MEDIGUN_FIRE_RESIST"))]
	Fire = 2,
}

impl MedigunResistance {
	/// The resistance numbered `raw`, or `None` for one the game does not
	/// declare.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			0 => Some(Self::Bullet),
			1 => Some(Self::Blast),
			2 => Some(Self::Fire),
			_ => None,
		}
	}

	/// The game's number for the resistance.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

/// One player's meters within the current engine callback.
///
/// Setters fail with [`EffectError::OutOfRange`] for a value the game would
/// not hold, and record the change so the engine sends it to clients.
#[derive(Debug, Clone, Copy)]
pub struct PlayerMeters<'s> {
	server: Server<'s>,
	player: Entity<'s>,
}

impl<'s> PlayerMeters<'s> {
	/// Wraps `player` for meter calls. Fails with [`EffectError::NotTfPlayer`]
	/// unless the server runs TF2 and `player`'s class name is `player`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, EffectError> {
		if !player_methods::is_tf_player(server, player) {
			return Err(EffectError::NotTfPlayer);
		}

		Ok(Self { server, player })
	}

	/// A Spy's cloak, from 0 to 100 (`m_flCloakMeter`).
	#[doc(alias("m_flCloakMeter", "GetSpyCloakMeter"))]
	pub fn cloak(self) -> Result<f32, EffectError> {
		self.get(c"m_flCloakMeter")
	}

	/// How many heads the player has taken with a sword such as the Eyelander
	/// (`m_iDecapitations`), from which the game works out their bonuses.
	#[doc(alias("m_iDecapitations", "GetDecapitations"))]
	pub fn decapitations(self) -> Result<c_int, EffectError> {
		self.get(c"m_iDecapitations")
	}

	/// A Scout's drink meter, such as Bonk! Atomic Punch's, from 0 to 100
	/// (`m_flEnergyDrinkMeter`).
	#[doc(alias("m_flEnergyDrinkMeter", "GetScoutEnergyDrinkMeter"))]
	pub fn energy_drink(self) -> Result<f32, EffectError> {
		self.get(c"m_flEnergyDrinkMeter")
	}

	/// Reads one of the player's networked variables.
	fn get<T: NetVar>(self, name: &CStr) -> Result<T, EffectError> {
		Ok(self.net_prop(name)?.get::<T>(self.player)?)
	}

	/// How many players and objects heal the player (`m_nNumHealers`).
	#[doc(alias("m_nNumHealers", "GetNumHealers"))]
	pub fn healers(self) -> Result<c_int, EffectError> {
		self.get(c"m_nNumHealers")
	}

	/// A Scout's hype, as the Soda Popper's and the Baby Face's Blaster's,
	/// from 0 to 100 (`m_flHypeMeter`).
	#[doc(alias("m_flHypeMeter", "GetScoutHypeMeter"))]
	pub fn hype(self) -> Result<f32, EffectError> {
		self.get(c"m_flHypeMeter")
	}

	/// Whether the player's rage is draining, as while a banner's buff lasts
	/// (`m_bRageDraining`).
	#[doc(alias("m_bRageDraining", "IsRageDraining"))]
	pub fn is_rage_draining(self) -> Result<bool, EffectError> {
		self.get(c"m_bRageDraining")
	}

	/// The charge meter of the item in loadout position `slot`, from 0 to 100,
	/// as a recharging throwable's (`m_flItemChargeMeter`). The game keeps one
	/// for each loadout position up to the last with such a meter.
	#[doc(alias("m_flItemChargeMeter", "GetItemChargeMeter"))]
	pub fn item_charge(self, slot: usize) -> Result<f32, EffectError> {
		Ok(self
			.net_prop(c"m_flItemChargeMeter")?
			.element(slot)?
			.get::<f32>(self.player)?)
	}

	/// Resolves one of the player's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, EffectError> {
		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.player, name)?)
	}

	/// The player whose meters these are.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// A rage meter, from 0 to 100, as a banner's, the Phlogistinator's, the
	/// Hitman's Heatmaker's or a Medic's shield's (`m_flRageMeter`).
	#[doc(alias("m_flRageMeter", "GetRageMeter"))]
	pub fn rage(self) -> Result<f32, EffectError> {
		self.get(c"m_flRageMeter")
	}

	/// How many revenge crits the player has stored, as from the Frontier
	/// Justice or the Diamondback (`m_iRevengeCrits`).
	///
	/// This has no setter: the game's own adds or removes the player's
	/// critical boost along with the count.
	#[doc(alias("m_iRevengeCrits", "GetRevengeCrits"))]
	pub fn revenge_crits(self) -> Result<c_int, EffectError> {
		self.get(c"m_iRevengeCrits")
	}

	/// Writes one of the player's networked variables, after `valid` accepts
	/// the value.
	fn set<T: NetVar>(self, name: &'static CStr, value: T, valid: bool) -> Result<(), EffectError> {
		if !valid {
			return Err(EffectError::OutOfRange("value"));
		}

		let engine = self.server.valve_engine()?;
		let variable = self.net_prop(name)?;

		// SAFETY: The value lies within what the game holds in this variable,
		// which it reads only as a meter or count.
		Ok(unsafe { variable.set(engine, self.player, value) }?)
	}

	/// Sets a Spy's cloak, from 0 to 100.
	#[doc(alias("SetSpyCloakMeter"))]
	pub fn set_cloak(self, cloak: f32) -> Result<(), EffectError> {
		self.set(c"m_flCloakMeter", cloak, is_meter(cloak))
	}

	/// Sets how many heads the player has taken, from 0 to 255. The game
	/// works out the bonuses from them anew as it next needs them, such as
	/// the player's speed when it next changes.
	#[doc(alias("SetDecapitations"))]
	pub fn set_decapitations(self, heads: c_int) -> Result<(), EffectError> {
		self.set(
			c"m_iDecapitations",
			heads,
			(0..=DECAPITATIONS_MAX).contains(&heads),
		)
	}

	/// Sets a Scout's drink meter, from 0 to 100.
	#[doc(alias("SetScoutEnergyDrinkMeter"))]
	pub fn set_energy_drink(self, meter: f32) -> Result<(), EffectError> {
		self.set(c"m_flEnergyDrinkMeter", meter, is_meter(meter))
	}

	/// Sets a Scout's hype, from 0 to 100, even while the hype buffs them,
	/// which the game's `SetScoutHypeMeter` refuses.
	#[doc(alias("SetScoutHypeMeter"))]
	pub fn set_hype(self, hype: f32) -> Result<(), EffectError> {
		self.set(c"m_flHypeMeter", hype, is_meter(hype))
	}

	/// Sets the charge meter of the item in loadout position `slot`, from 0
	/// to 100.
	pub fn set_item_charge(self, slot: usize, charge: f32) -> Result<(), EffectError> {
		if !is_meter(charge) {
			return Err(EffectError::OutOfRange("charge"));
		}

		let engine = self.server.valve_engine()?;
		let variable = self.net_prop(c"m_flItemChargeMeter")?.element(slot)?;

		// SAFETY: As for `set`.
		Ok(unsafe { variable.set(engine, self.player, charge) }?)
	}

	/// Sets a rage meter, from 0 to 100, even while it is draining or before
	/// the player may earn more, which the game's `SetRageMeter` refuses.
	#[doc(alias("SetRageMeter"))]
	pub fn set_rage(self, rage: f32) -> Result<(), EffectError> {
		self.set(c"m_flRageMeter", rage, is_meter(rage))
	}

	/// Sets a Demoman's shield charge, from 0 to 100.
	#[doc(alias("SetDemomanChargeMeter"))]
	pub fn set_shield_charge(self, charge: f32) -> Result<(), EffectError> {
		self.set(c"m_flChargeMeter", charge, is_meter(charge))
	}

	/// A Demoman's shield charge, from 0 to 100 (`m_flChargeMeter`).
	#[doc(alias("m_flChargeMeter", "GetDemomanChargeMeter"))]
	pub fn shield_charge(self) -> Result<f32, EffectError> {
		self.get(c"m_flChargeMeter")
	}
}

/// Whether `value` is a meter's, from 0 to 100.
fn is_meter(value: f32) -> bool {
	(0.0..=METER_MAX).contains(&value)
}
