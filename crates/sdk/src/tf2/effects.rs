//! TF2's status effects on players: stuns, burning and bleeding, a Spy's
//! disguise and invisibility, Mannpower powerups, Halloween spells, and the
//! movement state that jumps and knockback leave.
//!
//! Effects are applied through the game's native script methods, which run
//! its own effect code, and read from the player's networked variables.

#[cfg(test)]
#[path = "../tests/tf2/effects.rs"]
mod tests;

use crate::datatables::NetProp;
use crate::datatables::NetPropError;
use crate::entities::{Entity, EntityHandle};
use crate::tf2::PlayerClass;
use crate::tf2::conditions::Condition;
use crate::tf2::player_methods;
use crate::tf2::script_instances::{ScriptInstance, ScriptInstanceError};
use crate::{InterfaceError, Server};
use sdk_raw::tf2::conditions as raw_conditions;

use sdk_raw::tf2::effects::{
	TF_STUN_BY_TRIGGER, TF_STUN_CONTROLS, TF_STUN_DODGE_COOLDOWN, TF_STUN_LOSER_STATE,
	TF_STUN_MOVEMENT, TF_STUN_MOVEMENT_FORWARD_ONLY, TF_STUN_NO_EFFECTS, TF_STUN_SOUND,
	TF_STUN_SPECIAL_SOUND,
};

use sdk_raw::tf2::script_binding::{BindingError, boolean, float, handle, int};
use std::ffi::{CStr, c_int};
use std::ptr::NonNull;

/// The most a stun can slow a player, as `CTFPlayerShared::StunPlayer`
/// scales its slowdown to.
const STUN_AMOUNT_MAX: f32 = 255.0;

/// `TEAM_ANY` of `game/shared/shareddefs.h`: a powerup dropped for anyone.
const TEAM_ANY: c_int = -1;

bitflags::bitflags! {
	/// How a stun affects a player: the `TF_STUN_*` flags of
	/// `game/shared/tf/tf_shareddefs.h`.
	///
	/// Flags read from the game keep every bit, including those without a
	/// constant here.
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
	pub struct StunFlags: c_int {
		/// `TF_STUN_BY_TRIGGER`: the stun is a scare, as a Halloween ghost's
		/// or `trigger_stun`'s: the player shows fright rather than stars, is
		/// slowed less in the [losing state](Self::LOSER_STATE), and hears the
		/// stun's sound only when not already stunned.
		#[doc(alias("TF_STUN_BY_TRIGGER"))]
		const BY_TRIGGER = TF_STUN_BY_TRIGGER;

		/// `TF_STUN_CONTROLS`: the stun takes away the player's control, as a
		/// sapped robot's in Mann vs. Machine.
		#[doc(alias("TF_STUN_CONTROLS"))]
		const CONTROLS = TF_STUN_CONTROLS;

		/// `TF_STUN_DODGE_COOLDOWN`: declared for the Scout's dodge, and unused
		/// by the game.
		#[doc(alias("TF_STUN_DODGE_COOLDOWN"))]
		const DODGE_COOLDOWN = TF_STUN_DODGE_COOLDOWN;

		/// `TF_STUN_LOSER_STATE`: the stun puts the player in the losing
		/// team's state after a round: slowed, and unable to attack.
		#[doc(alias("TF_STUN_LOSER_STATE"))]
		const LOSER_STATE = TF_STUN_LOSER_STATE;

		/// `TF_STUN_MOVEMENT`: the stun slows the player's movement.
		#[doc(alias("TF_STUN_MOVEMENT"))]
		const MOVEMENT = TF_STUN_MOVEMENT;

		/// `TF_STUN_MOVEMENT_FORWARD_ONLY`: the stun slows only the player's
		/// forward movement.
		#[doc(alias("TF_STUN_MOVEMENT_FORWARD_ONLY"))]
		const MOVEMENT_FORWARD_ONLY = TF_STUN_MOVEMENT_FORWARD_ONLY;

		/// `TF_STUN_NO_EFFECTS`: the stun shows no particles over the player's
		/// head.
		#[doc(alias("TF_STUN_NO_EFFECTS"))]
		const NO_EFFECTS = TF_STUN_NO_EFFECTS;

		/// `TF_STUN_SOUND`: the stun plays its sound.
		#[doc(alias("TF_STUN_SOUND"))]
		const SOUND = TF_STUN_SOUND;

		/// `TF_STUN_SPECIAL_SOUND`: the stun plays the sound of a long-range
		/// stun.
		#[doc(alias("TF_STUN_SPECIAL_SOUND"))]
		const SPECIAL_SOUND = TF_STUN_SPECIAL_SOUND;

		/// `TF_STUN_BOTH`: [`MOVEMENT`](Self::MOVEMENT) and
		/// [`CONTROLS`](Self::CONTROLS).
		#[doc(alias("TF_STUN_BOTH"))]
		const BOTH = TF_STUN_MOVEMENT | TF_STUN_CONTROLS;

		const _ = !0;
	}
}

/// The stun a player is under, as the game networks it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActiveStun {
	/// How long the stun lasts in all, in seconds, from when it began
	/// (`m_flMovementStunTime`).
	pub duration: f32,

	/// How the stun affects the player (`m_iStunFlags`).
	pub flags: StunFlags,

	/// How much the stun slows the player, from 0 to 1
	/// (`m_iMovementStunAmount`, out of 255).
	pub slowdown: f32,

	/// The player credited with the stun, if any (`m_hStunner`).
	pub stunner: Option<EntityHandle>,
}

/// A Spy's disguise, as the game networks it.
///
/// The game shows a Spy with [`Condition::DISGUISED_AS_DISPENSER`] to
/// Engineers as an Engineer's dispenser, whatever class this gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disguise {
	/// The class the Spy looks like (`m_nDisguiseClass`), or `None` for a
	/// number outside the playable classes.
	pub class: Option<PlayerClass>,

	/// The health enemies see on the Spy (`m_iDisguiseHealth`).
	pub health: c_int,

	/// The player whose name enemies see on the Spy, if any
	/// (`m_hDisguiseTarget`).
	pub target: Option<EntityHandle>,

	/// The number of the team the Spy looks like (`m_nDisguiseTeam`).
	pub team: c_int,
}

/// Why an effect, meter or taunt operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EffectError {
	/// The server is not running TF2, or the entity is not what the wrapper
	/// takes: a player, of class name `player`, or for a
	/// [`Medigun`](crate::tf2::meters::Medigun), a `tf_weapon_medigun`.
	#[error("the wrapper requires a TF2 player, or a TF2 medigun")]
	NotTfPlayer,

	/// The player's script class descriptors lack the native method, or its
	/// signature differs from the SDK's.
	#[error("the game does not expose the expected native method")]
	UnsupportedMethod,

	/// The native method's binding adapter reported failure.
	#[error("the native method rejected its arguments")]
	Rejected,

	/// An argument lies outside what the game accepts.
	#[error("`{0}` is out of range")]
	OutOfRange(&'static str),

	/// The script instance of an entity the game takes, such as a stun's
	/// attacker, could not be found or made.
	#[error("the entity has no script instance: {0}")]
	ScriptInstance(#[source] ScriptInstanceError),

	/// A required engine or game interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// A networked variable could not be found, read or written.
	#[error(transparent)]
	NetProp(#[from] NetPropError),
}

impl From<BindingError> for EffectError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// One player's status effects within the current engine callback.
#[derive(Debug, Clone, Copy)]
pub struct PlayerEffects<'s> {
	server: Server<'s>,
	player: Entity<'s>,
}

impl<'s> PlayerEffects<'s> {
	/// Wraps `player` for effect calls. Fails with [`EffectError::NotTfPlayer`]
	/// unless the server runs TF2 and `player`'s class name is `player`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, EffectError> {
		if !player_methods::is_tf_player(server, player) {
			return Err(EffectError::NotTfPlayer);
		}

		Ok(Self { server, player })
	}

	/// The stun the player is under, or `None` if they are not stunned.
	#[doc(alias("m_iStunFlags", "m_hStunner", "GetActiveStunInfo"))]
	pub fn active_stun(self) -> Result<Option<ActiveStun>, EffectError> {
		if !self.in_cond(Condition::STUNNED)? {
			return Ok(None);
		}

		let flags = self.net_prop(c"m_iStunFlags")?.get::<c_int>(self.player)?;
		let amount = self
			.net_prop(c"m_iMovementStunAmount")?
			.get::<c_int>(self.player)?;
		let stunner = self.net_prop(c"m_hStunner")?.get_handle(self.player)?;

		Ok(Some(ActiveStun {
			duration: self
				.net_prop(c"m_flMovementStunTime")?
				.get::<f32>(self.player)?,
			flags: StunFlags::from_bits_retain(flags),
			slowdown: amount as f32 / STUN_AMOUNT_MAX,
			stunner: stunner.is_valid().then_some(stunner),
		}))
	}

	/// Bleeds the player for `duration` seconds, or without end if
	/// `permanent`, taking `damage` every half second, as the game's own
	/// bleeds take 4, until they die or the bleeding condition is removed.
	///
	/// The player is the attacker, so a death from it counts as a suicide, and
	/// their active weapon, if any, is the bleed's source, so another bleed
	/// from here extends this one rather than adding to it. The game refuses
	/// dead players (`CTFPlayerShared::MakeBleed`).
	///
	/// Fails with [`EffectError::OutOfRange`] for a negative or infinite
	/// duration, or negative damage.
	#[doc(alias("BleedPlayer", "BleedPlayerEx", "MakeBleed"))]
	pub fn bleed(self, duration: f32, damage: c_int, permanent: bool) -> Result<(), EffectError> {
		if !(duration.is_finite() && duration >= 0.0) {
			return Err(EffectError::OutOfRange("duration"));
		}

		if damage < 0 {
			return Err(EffectError::OutOfRange("damage"));
		}

		// SAFETY: As for `call`. The method adds the bleed with the player as its
		// attacker, and the game's own bleeding damage type.
		unsafe {
			self.call(
				c"BleedPlayerEx",
				&mut [
					float(duration),
					int(damage),
					boolean(permanent),
					int(sys::ETFDmgCustom_TF_DMG_CUSTOM_BLEEDING as c_int),
				],
			)
		}
	}

	/// Calls one of the player's native methods that returns nothing.
	///
	/// # Safety
	///
	/// The method accepts these arguments, and runs only the game's own
	/// effect code on the player, which frees entities only through deferred
	/// deletion.
	unsafe fn call(
		self,
		method: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
	) -> Result<(), EffectError> {
		// SAFETY: A TF2 `player` is a CTFPlayer, live on the main thread for
		// this callback, whose game module stays loaded, and the caller vouches
		// for the method.
		Ok(unsafe { player_methods::call_void(self.player, method, arguments) }?)
	}

	/// Whether the player could jump in the air now, as a Scout can, or anyone
	/// with Halloween's speed boost (`CTFPlayer::CanAirDash`).
	#[doc(alias("CanAirDash"))]
	pub fn can_air_dash(self) -> Result<bool, EffectError> {
		self.predicate(c"CanAirDash")
	}

	/// Removes the player's Halloween spells, as a round restart does.
	#[doc(alias("ClearSpells"))]
	pub fn clear_spells(self) -> Result<(), EffectError> {
		// SAFETY: As for `call`. The method clears the spellbook's spell, if
		// the player has a spellbook.
		unsafe { self.call(c"ClearSpells", &mut []) }
	}

	/// The player's disguise, or `None` if they are not disguised.
	#[doc(alias("m_nDisguiseTeam", "m_nDisguiseClass", "GetDisguiseTarget"))]
	pub fn disguise(self) -> Result<Option<Disguise>, EffectError> {
		if !self.in_cond(Condition::DISGUISED)? {
			return Ok(None);
		}

		let class = self
			.net_prop(c"m_nDisguiseClass")?
			.get::<c_int>(self.player)?;
		let target = self
			.net_prop(c"m_hDisguiseTarget")?
			.get_handle(self.player)?;

		Ok(Some(Disguise {
			class: PlayerClass::from_raw(class),
			health: self
				.net_prop(c"m_iDisguiseHealth")?
				.get::<c_int>(self.player)?,
			target: target.is_valid().then_some(target),
			team: self
				.net_prop(c"m_nDisguiseTeam")?
				.get::<c_int>(self.player)?,
		}))
	}

	/// The ammunition the player's disguise weapon shows.
	#[doc(alias("GetDisguiseAmmoCount"))]
	pub fn disguise_ammo(self) -> Result<c_int, EffectError> {
		// SAFETY: As for `call`. The method reads a member.
		Ok(unsafe { player_methods::call_int(self.player, c"GetDisguiseAmmoCount", &mut []) }?)
	}

	/// Drops the player's Mannpower powerup, for anyone to pick up, as a
	/// player dropping it does. Returns whether they carried one.
	#[doc(alias("DropRune"))]
	pub fn drop_rune(self) -> Result<bool, EffectError> {
		if !self.is_carrying_rune()? {
			return Ok(false);
		}

		// SAFETY: As for `call`. The method takes the powerup from the player,
		// and creates the powerup entity in front of them.
		unsafe { self.call(c"DropRune", &mut [boolean(true), int(TEAM_ANY)]) }?;
		Ok(true)
	}

	/// Puts out the player's flames, with the sound of it, if they burn.
	#[doc(alias("ExtinguishPlayerBurning"))]
	pub fn extinguish(self) -> Result<(), EffectError> {
		// SAFETY: As for `call`. The method removes the burning condition.
		unsafe { self.call(c"ExtinguishPlayerBurning", &mut []) }
	}

	/// Sets the player alight, as the `IgnitePlayer` input does: with the
	/// player as the attacker, and with no weapon to give the flames
	/// afterburn, so they go out at once unless something keeps them burning.
	/// The game refuses dead and phasing players (`CTFPlayerShared::Burn`).
	#[doc(alias("IgnitePlayer", "Burn"))]
	pub fn ignite(self) -> Result<(), EffectError> {
		// SAFETY: As for `call`. The method burns the player, as their own
		// attacker, without a weapon.
		unsafe { self.call(c"IgnitePlayer", &mut []) }
	}

	/// Whether the player is in the air from an explosion, as a blast jump, or
	/// flies with the Thermal Thruster (`CTFPlayer::InAirDueToExplosion`).
	#[doc(alias("InAirDueToExplosion"))]
	pub fn in_air_due_to_explosion(self) -> Result<bool, EffectError> {
		self.predicate(c"InAirDueToExplosion")
	}

	/// Whether the player is in the air from knockback: from an explosion,
	/// being knocked into the air, or a grappling hook, and not in water.
	#[doc(alias("InAirDueToKnockback"))]
	pub fn in_air_due_to_knockback(self) -> Result<bool, EffectError> {
		self.predicate(c"InAirDueToKnockback")
	}

	/// Whether the player has `condition`, for the effects that read their
	/// conditions.
	fn in_cond(self, condition: Condition) -> Result<bool, EffectError> {
		// SAFETY: As for `call`. The method reads the player's condition bits.
		Ok(unsafe { raw_conditions::in_cond(self.raw_player(), condition.to_raw()) }?)
	}

	/// Whether the player has jumped in the air since leaving the ground.
	#[doc(alias("IsAirDashing"))]
	pub fn is_air_dashing(self) -> Result<bool, EffectError> {
		self.predicate(c"IsAirDashing")
	}

	/// Whether the player carries a Mannpower powerup.
	#[doc(alias("IsCarryingRune"))]
	pub fn is_carrying_rune(self) -> Result<bool, EffectError> {
		self.predicate(c"IsCarryingRune")
	}

	/// Whether the player is in a jump they made.
	#[doc(alias("IsJumping"))]
	pub fn is_jumping(self) -> Result<bool, EffectError> {
		self.predicate(c"IsJumping")
	}

	/// Whether the player has a parachute equipped.
	#[doc(alias("IsParachuteEquipped"))]
	pub fn is_parachute_equipped(self) -> Result<bool, EffectError> {
		self.predicate(c"IsParachuteEquipped")
	}

	/// Resolves one of the player's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, EffectError> {
		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.player, name)?)
	}

	/// The player whose effects these are.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// Calls one of the player's native predicates, which take nothing.
	fn predicate(self, method: &CStr) -> Result<bool, EffectError> {
		// SAFETY: As for `call`. The predicates only read the player's state.
		Ok(unsafe { player_methods::call_bool(self.player, method, &mut []) }?)
	}

	/// The player's pointer, for the raw condition methods.
	fn raw_player(self) -> NonNull<sys::CBaseEntity> {
		// SAFETY: An entity's pointer is never null.
		unsafe { NonNull::new_unchecked(self.player.as_ptr()) }
	}

	/// Takes away the player's disguise, if they have one or are putting one
	/// on.
	#[doc(alias("RemoveDisguise"))]
	pub fn remove_disguise(self) -> Result<(), EffectError> {
		// SAFETY: As for `call`. The method removes the disguise conditions.
		unsafe { self.call(c"RemoveDisguise", &mut []) }
	}

	/// Makes the player visible again, fading out their cloak or invisibility
	/// over half a second, or two for an invisibility another Spy gave them.
	#[doc(alias("RemoveInvisibility"))]
	pub fn remove_invisibility(self) -> Result<(), EffectError> {
		// SAFETY: As for `call`. The method fades the player's invisibility.
		unsafe { self.call(c"RemoveInvisibility", &mut []) }
	}

	/// Gives the player a rare Halloween spell if they have a spellbook, and
	/// fires `cross_spectral_bridge` either way.
	#[doc(alias("RollRareSpell"))]
	pub fn roll_rare_spell(self) -> Result<(), EffectError> {
		// SAFETY: As for `call`. The method rolls the spellbook's spell.
		unsafe { self.call(c"RollRareSpell", &mut []) }
	}

	/// How charged the ability of the player's Mannpower powerup is, from 0 to
	/// 100 (`m_flRuneCharge`).
	#[doc(alias("m_flRuneCharge", "GetRuneCharge"))]
	pub fn rune_charge(self) -> Result<f32, EffectError> {
		Ok(self.net_prop(c"m_flRuneCharge")?.get::<f32>(self.player)?)
	}

	/// Sets the ammunition the player's disguise weapon shows. Fails with
	/// [`EffectError::OutOfRange`] for a negative count.
	#[doc(alias("SetDisguiseAmmoCount"))]
	pub fn set_disguise_ammo(self, ammo: c_int) -> Result<(), EffectError> {
		if ammo < 0 {
			return Err(EffectError::OutOfRange("ammo"));
		}

		// SAFETY: As for `call`. The method writes a member.
		unsafe { self.call(c"SetDisguiseAmmoCount", &mut [int(ammo)]) }
	}

	/// Stuns the player for `duration` seconds, slowing them by `slowdown`,
	/// from 0 to 1, in the ways `flags` give, as the game's stuns do
	/// (`CTFPlayerShared::StunPlayer`).
	///
	/// A stun credited to `attacker`, a player, names them as the stunner, and
	/// is refused while the game's truce holds, if they are on a playing team.
	/// Other entities count as no attacker. The game takes the attacker as its
	/// script instance, which is made for it, with a script scope, if it has
	/// none yet ([`ScriptInstance::of`]).
	///
	/// The game refuses the stun for a player who is phasing, intercepting a
	/// PASS Time pass, under the Quick-Fix's ÜberCharge, or hidden
	/// invulnerable, and ignores one weaker than the player's stun that would
	/// end sooner. A stun that becomes the active one fires `player_stunned`,
	/// and one of the controls or the losing state also stops the player's
	/// taunt.
	///
	/// Fails with [`EffectError::OutOfRange`] for a negative or infinite
	/// duration, or a slowdown outside 0 to 1.
	#[doc(alias("StunPlayer"))]
	pub fn stun(
		self,
		duration: f32,
		slowdown: f32,
		flags: StunFlags,
		attacker: Option<Entity<'s>>,
	) -> Result<(), EffectError> {
		if !(duration.is_finite() && duration >= 0.0) {
			return Err(EffectError::OutOfRange("duration"));
		}

		if !(0.0..=1.0).contains(&slowdown) {
			return Err(EffectError::OutOfRange("slowdown"));
		}

		let attacker = match attacker {
			Some(attacker) => ScriptInstance::of(self.server, attacker)
				.map_err(EffectError::ScriptInstance)?
				.as_raw(),

			None => std::ptr::null_mut(),
		};

		// SAFETY: As for `call`. The method stuns the player; the attacker is
		// null, or a script instance the VM registered, which the game resolves
		// to its entity, checks is a player, and keeps by handle.
		unsafe {
			self.call(
				c"StunPlayer",
				&mut [
					float(duration),
					float(slowdown),
					int(flags.bits()),
					handle(attacker),
				],
			)
		}
	}
}
