//! TF2's taunts: starting, stopping and reading a player's taunt, through
//! the game's native script methods and the player's networked variables,
//! and the attack it has scheduled, which the game keeps in members it does
//! not network ([`ScheduledAttack`]).
//!
//! [`TfPlayer::set_forced_taunt_cam`](crate::tf2::player::TfPlayer::set_forced_taunt_cam)
//! forces a player's view into third person, as taunts do.

#[cfg(test)]
#[path = "../tests/tf2/taunts.rs"]
mod tests;

use crate::Server;
use crate::datatables::NetProp;
use crate::entities::Entity;
use crate::tf2::conditions::Condition;
use crate::tf2::effects::EffectError;
use crate::tf2::player_methods;
use crate::tf2::weapons::ItemDefinitionIndex;
use sdk_raw::tf2::conditions as raw_conditions;
use sdk_raw::tf2::script_binding::{boolean, float, int};
use sdk_raw::tf2::taunts as raw_taunts;
use std::ffi::{CStr, c_int};
use std::fmt;
use std::ptr::NonNull;

/// The most taunt slots a loadout has, which
/// `CTFPlayer::HandleTauntCommand` numbers from 1.
pub const TAUNT_SLOTS: u8 = 8;

/// One player's taunts within the current engine callback.
#[derive(Debug, Clone, Copy)]
pub struct PlayerTaunts<'s> {
	server: Server<'s>,
	player: Entity<'s>,
}

impl<'s> PlayerTaunts<'s> {
	/// Wraps `player` for taunt calls. Fails with [`EffectError::NotTfPlayer`]
	/// unless the server runs TF2 and `player`'s class name is `player`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, EffectError> {
		if !player_methods::is_tf_player(server, player) {
			return Err(EffectError::NotTfPlayer);
		}

		Ok(Self { server, player })
	}

	/// Calls one of the player's native methods that returns nothing.
	///
	/// # Safety
	///
	/// The method accepts these arguments, and runs only the game's own taunt
	/// code on the player, which frees entities only through deferred
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

	/// Stops the player's taunt, as [`Self::stop`] does with its prop, and
	/// calls off a Eureka Effect teleport the taunt began.
	#[doc(alias("CancelTaunt"))]
	pub fn cancel(self) -> Result<(), EffectError> {
		// SAFETY: As for `call`. The method stops the taunt.
		unsafe { self.call(c"CancelTaunt", &mut []) }
	}

	/// Ends the player's press-and-hold taunt as letting go of the taunt key
	/// does, after its least time and through its outro, if it has them.
	/// Returns whether the player was in such a taunt; the game would read
	/// the taunt's item without checking it, so no other taunt is ended.
	#[doc(alias("EndLongTaunt"))]
	pub fn end_long_taunt(self) -> Result<bool, EffectError> {
		match self.state()? {
			Some(state) if state.is_long() && state.item.is_some() => {}
			_ => return Ok(false),
		}

		// SAFETY: As for `call`. The player is in a press-and-hold taunt from an
		// item, whose definition the method reads, which the game clears only
		// along with the taunt's item index.
		unsafe { self.call(c"EndLongTaunt", &mut []) }?;
		Ok(true)
	}

	/// Whether the game may end the player's taunt once its time is up, which
	/// a press-and-hold taunt holds off while the key is held.
	#[doc(alias("IsAllowedToRemoveTaunt"))]
	pub fn is_allowed_to_remove_taunt(self) -> Result<bool, EffectError> {
		self.predicate(c"IsAllowedToRemoveTaunt")
	}

	/// Whether the player could start a taunt now: alive, on the ground, not
	/// already taunting, charging, carrying a building, or holding a weapon
	/// that forbids it, among the game's other checks.
	#[doc(alias("IsAllowedToTaunt"))]
	pub fn is_allowed_to_taunt(self) -> Result<bool, EffectError> {
		self.predicate(c"IsAllowedToTaunt")
	}

	/// Whether the player is taunting ([`Condition::TAUNTING`]).
	#[doc(alias("IsTaunting"))]
	pub fn is_taunting(self) -> Result<bool, EffectError> {
		self.predicate(c"IsTaunting")
	}

	/// How fast the player's taunt moves them.
	#[doc(alias("GetCurrentTauntMoveSpeed"))]
	pub fn move_speed(self) -> Result<f32, EffectError> {
		// SAFETY: As for `call`. The method reads a member.
		Ok(unsafe {
			player_methods::call_float(self.player, c"GetCurrentTauntMoveSpeed", &mut [])
		}?)
	}

	/// Resolves one of the player's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, EffectError> {
		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.player, name)?)
	}

	/// The player whose taunts these are.
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

	/// The game time at which the player's taunt may end, or 0 once it has
	/// been stopped.
	#[doc(alias("GetTauntRemoveTime"))]
	pub fn remove_time(self) -> Result<f32, EffectError> {
		// SAFETY: As for `call`. The method reads a member.
		Ok(unsafe { player_methods::call_float(self.player, c"GetTauntRemoveTime", &mut []) }?)
	}

	/// The attack the player's taunt has scheduled, if any, which the game
	/// runs as they think after moving once its time has passed: see
	/// [`ScheduledAttack`].
	#[doc(alias("m_iTauntAttack", "m_flTauntAttackTime"))]
	pub fn scheduled_attack(self) -> Option<ScheduledAttack> {
		// SAFETY: A TF2 `player` is a CTFPlayer, live on the main thread for this
		// callback, on which alone the game writes its members.
		let (attack, time) = unsafe { raw_taunts::scheduled_attack(self.raw_player()) };

		(attack != 0 && time != 0.0).then_some(ScheduledAttack {
			attack: TauntAttack::from_raw(attack),
			time,
		})
	}

	/// Sets how fast the player's taunt moves them, for a taunt that lets them
	/// move. Fails with [`EffectError::OutOfRange`] for a negative or infinite
	/// speed.
	#[doc(alias("SetCurrentTauntMoveSpeed"))]
	pub fn set_move_speed(self, speed: f32) -> Result<(), EffectError> {
		if !(speed.is_finite() && speed >= 0.0) {
			return Err(EffectError::OutOfRange("speed"));
		}

		// SAFETY: As for `call`. The method writes a member.
		unsafe { self.call(c"SetCurrentTauntMoveSpeed", &mut [float(speed)]) }
	}

	/// The taunt the player is in, or `None` if they are not taunting.
	#[doc(alias("m_iTauntIndex", "m_iTauntConcept", "m_iTauntItemDefIndex"))]
	pub fn state(self) -> Result<Option<TauntState>, EffectError> {
		// SAFETY: As for `call`. The method reads the player's condition bits.
		let taunting =
			unsafe { raw_conditions::in_cond(self.raw_player(), Condition::TAUNTING.to_raw()) }?;

		if !taunting {
			return Ok(None);
		}

		let item = self
			.net_prop(c"m_iTauntItemDefIndex")?
			.get::<c_int>(self.player)?;

		Ok(Some(TauntState {
			can_move: self
				.net_prop(c"m_bAllowMoveDuringTaunt")?
				.get::<bool>(self.player)?,
			concept: self
				.net_prop(c"m_iTauntConcept")?
				.get::<c_int>(self.player)?,
			item: u16::try_from(item).ok().and_then(ItemDefinitionIndex::new),
			kind: self.net_prop(c"m_iTauntIndex")?.get::<c_int>(self.player)?,
			move_speed: self
				.net_prop(c"m_flCurrentTauntMoveSpeed")?
				.get::<f32>(self.player)?,
		}))
	}

	/// Stops the player's taunt at once, with its scene, sound and partner
	/// taunt, and removes its prop, if any, unless `remove_prop` is false and
	/// the prop removes itself after its outro.
	#[doc(alias("StopTaunt"))]
	pub fn stop(self, remove_prop: bool) -> Result<(), EffectError> {
		// SAFETY: As for `call`. The method stops the taunt, and removes its
		// prop through the engine's deferred deletion.
		unsafe { self.call(c"StopTaunt", &mut [boolean(remove_prop)]) }
	}

	/// Starts a taunt of `kind`, as the game's code does, and returns whether
	/// the player is taunting afterwards. The game refuses a player who is
	/// not [allowed to taunt](Self::is_allowed_to_taunt), or whose class and
	/// weapon have no such taunt.
	#[doc(alias("Taunt"))]
	pub fn taunt(self, kind: TauntKind) -> Result<bool, EffectError> {
		// SAFETY: As for `call`. Both kinds pick their own response concept, so
		// the concept passed is unused.
		unsafe { self.call(c"Taunt", &mut [int(kind.to_raw()), int(0)]) }?;
		self.is_taunting()
	}

	/// Starts a taunt as the taunt key does: the taunt equipped in loadout
	/// taunt slot `slot`, from 1 to [`TAUNT_SLOTS`], or for 0, joining a
	/// nearby partner's taunt or else the active weapon's taunt. Returns
	/// whether the player is taunting afterwards.
	///
	/// Fails with [`EffectError::OutOfRange`] for a slot past
	/// [`TAUNT_SLOTS`].
	#[doc(alias("HandleTauntCommand"))]
	pub fn taunt_slot(self, slot: u8) -> Result<bool, EffectError> {
		if slot > TAUNT_SLOTS {
			return Err(EffectError::OutOfRange("slot"));
		}

		// SAFETY: As for `call`. The method checks the slot, and plays the
		// equipped taunt item, if any.
		unsafe { self.call(c"HandleTauntCommand", &mut [int(c_int::from(slot))]) }?;
		self.is_taunting()
	}
}

/// An attack a player's taunt has scheduled, as
/// [`PlayerTaunts::scheduled_attack`] reads it.
///
/// `CTFPlayer::PostThink` runs the attack through `DoTauntAttack`, which no
/// vtable holds, in the player's first think after moving once [`Self::time`]
/// has passed, if they still taunt and live. The game looks for its victims
/// among players, and for a few attacks among buildings, by their flags and
/// `IsPlayer`, and so hurts no other entity. The one exception is the
/// Soldier's Kamikaze, whose blast hurts whatever it reaches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScheduledAttack {
	/// The attack.
	pub attack: TauntAttack,

	/// The game time after which it lands.
	pub time: f32,
}

impl ScheduledAttack {
	/// Whether the player's think after moving at game time `now` runs the
	/// attack, as long as they still taunt and live.
	pub fn is_due(&self, now: f32) -> bool {
		self.time < now
	}
}

/// Defines [`TauntAttack`]'s attacks from the generated `taunt_attack_t`
/// values, each with TF2's name for it, along with [`TauntAttack::ALL`] and
/// [`TauntAttack::name`].
macro_rules! attacks {
	($(
		$(#[doc = $doc:literal])*
		$attack:ident = $native:ident, $name:literal;
	)*) => {
		#[allow(
			clippy::unnecessary_cast,
			reason = "`taunt_attack_t` is `c_int` on Windows but `c_uint` on Linux"
		)]
		impl TauntAttack {
			$(
				$(#[doc = $doc])*
				#[doc(alias = $name)]
				pub const $attack: Self = Self(sys::$native as c_int);
			)*

			/// Every attack TF2 numbers, in its order, from [`Self::NONE`], 0.
			pub const ALL: [Self; [$($name),*].len()] = [$(Self::$attack),*];

			/// TF2's name for the attack, such as `TAUNTATK_PYRO_HADOUKEN`, or
			/// `None` for a number it does not name.
			pub const fn name(self) -> Option<&'static str> {
				Some(match self {
					$(Self::$attack => $name,)*
					_ => return None,
				})
			}
		}

		/// The generated values' names, by attack, which the tests compare
		/// with the names the attacks give.
		#[cfg(test)]
		const NATIVE_NAMES: [&str; TauntAttack::ALL.len()] = [$(stringify!($native)),*];
	};
}

attacks! {
	/// No attack.
	NONE = taunt_attack_t_TAUNTATK_NONE, "TAUNTATK_NONE";

	/// The Pyro's Hadouken.
	PYRO_HADOUKEN = taunt_attack_t_TAUNTATK_PYRO_HADOUKEN, "TAUNTATK_PYRO_HADOUKEN";

	/// The Heavy eating a lunchbox item.
	HEAVY_EAT = taunt_attack_t_TAUNTATK_HEAVY_EAT, "TAUNTATK_HEAVY_EAT";

	/// The Heavy's taunt that buffs those around them.
	HEAVY_RADIAL_BUFF = taunt_attack_t_TAUNTATK_HEAVY_RADIAL_BUFF, "TAUNTATK_HEAVY_RADIAL_BUFF";

	/// The Heavy's High Noon.
	HEAVY_HIGH_NOON = taunt_attack_t_TAUNTATK_HEAVY_HIGH_NOON, "TAUNTATK_HEAVY_HIGH_NOON";

	/// The Scout drinking.
	SCOUT_DRINK = taunt_attack_t_TAUNTATK_SCOUT_DRINK, "TAUNTATK_SCOUT_DRINK";

	/// The Scout's Grand Slam, with the Sandman.
	SCOUT_GRAND_SLAM = taunt_attack_t_TAUNTATK_SCOUT_GRAND_SLAM, "TAUNTATK_SCOUT_GRAND_SLAM";

	/// The Medic's inhale.
	MEDIC_INHALE = taunt_attack_t_TAUNTATK_MEDIC_INHALE, "TAUNTATK_MEDIC_INHALE";

	/// The first slash of the Spy's Fencing.
	SPY_FENCING_SLASH_A = taunt_attack_t_TAUNTATK_SPY_FENCING_SLASH_A, "TAUNTATK_SPY_FENCING_SLASH_A";

	/// The second slash of the Spy's Fencing.
	SPY_FENCING_SLASH_B = taunt_attack_t_TAUNTATK_SPY_FENCING_SLASH_B, "TAUNTATK_SPY_FENCING_SLASH_B";

	/// The stab that ends the Spy's Fencing.
	SPY_FENCING_STAB = taunt_attack_t_TAUNTATK_SPY_FENCING_STAB, "TAUNTATK_SPY_FENCING_STAB";

	/// The winner's kill at the end of Rock, Paper, Scissors.
	RPS_KILL = taunt_attack_t_TAUNTATK_RPS_KILL, "TAUNTATK_RPS_KILL";

	/// The Sniper's Skewer, with the Huntsman: the impaling stab, which stuns.
	SNIPER_ARROW_STAB_IMPALE = taunt_attack_t_TAUNTATK_SNIPER_ARROW_STAB_IMPALE, "TAUNTATK_SNIPER_ARROW_STAB_IMPALE";

	/// The Sniper's Skewer: the stab that kills.
	SNIPER_ARROW_STAB_KILL = taunt_attack_t_TAUNTATK_SNIPER_ARROW_STAB_KILL, "TAUNTATK_SNIPER_ARROW_STAB_KILL";

	/// The Soldier's Kamikaze, with the Equalizer or the Escape Plan.
	SOLDIER_GRENADE_KILL = taunt_attack_t_TAUNTATK_SOLDIER_GRENADE_KILL, "TAUNTATK_SOLDIER_GRENADE_KILL";

	/// The Demoman's Barbarian Swing, with a sword.
	DEMOMAN_BARBARIAN_SWING = taunt_attack_t_TAUNTATK_DEMOMAN_BARBARIAN_SWING, "TAUNTATK_DEMOMAN_BARBARIAN_SWING";

	/// The Medic's Spinal Tap, with the Ubersaw: the impaling stab, which
	/// stuns.
	MEDIC_UBERSLICE_IMPALE = taunt_attack_t_TAUNTATK_MEDIC_UBERSLICE_IMPALE, "TAUNTATK_MEDIC_UBERSLICE_IMPALE";

	/// The Medic's Spinal Tap: the stab that kills.
	MEDIC_UBERSLICE_KILL = taunt_attack_t_TAUNTATK_MEDIC_UBERSLICE_KILL, "TAUNTATK_MEDIC_UBERSLICE_KILL";

	/// A flip's landing particles.
	FLIP_LAND_PARTICLE = taunt_attack_t_TAUNTATK_FLIP_LAND_PARTICLE, "TAUNTATK_FLIP_LAND_PARTICLE";

	/// Rock, Paper, Scissors' particles.
	RPS_PARTICLE = taunt_attack_t_TAUNTATK_RPS_PARTICLE, "TAUNTATK_RPS_PARTICLE";

	/// A high five's particles.
	HIGHFIVE_PARTICLE = taunt_attack_t_TAUNTATK_HIGHFIVE_PARTICLE, "TAUNTATK_HIGHFIVE_PARTICLE";

	/// The Engineer's Dischord, with the Frontier Justice.
	ENGINEER_GUITAR_SMASH = taunt_attack_t_TAUNTATK_ENGINEER_GUITAR_SMASH, "TAUNTATK_ENGINEER_GUITAR_SMASH";

	/// The Engineer's Organ Grinder, with the Gunslinger: the impaling
	/// stab, which stuns.
	ENGINEER_ARM_IMPALE = taunt_attack_t_TAUNTATK_ENGINEER_ARM_IMPALE, "TAUNTATK_ENGINEER_ARM_IMPALE";

	/// The Organ Grinder: the blow that kills.
	ENGINEER_ARM_KILL = taunt_attack_t_TAUNTATK_ENGINEER_ARM_KILL, "TAUNTATK_ENGINEER_ARM_KILL";

	/// The Organ Grinder: the grinding between the stab and the kill, a hit
	/// each time it runs.
	ENGINEER_ARM_BLEND = taunt_attack_t_TAUNTATK_ENGINEER_ARM_BLEND, "TAUNTATK_ENGINEER_ARM_BLEND";

	/// The Soldier's Kamikaze in its Wormsign form.
	SOLDIER_GRENADE_KILL_WORMSIGN = taunt_attack_t_TAUNTATK_SOLDIER_GRENADE_KILL_WORMSIGN, "TAUNTATK_SOLDIER_GRENADE_KILL_WORMSIGN";

	/// Showing an item off.
	SHOW_ITEM = taunt_attack_t_TAUNTATK_SHOW_ITEM, "TAUNTATK_SHOW_ITEM";

	/// The Medic releasing doves.
	MEDIC_RELEASE_DOVES = taunt_attack_t_TAUNTATK_MEDIC_RELEASE_DOVES, "TAUNTATK_MEDIC_RELEASE_DOVES";

	/// The Pyro's Armageddon, with the Rainblower.
	PYRO_ARMAGEDDON = taunt_attack_t_TAUNTATK_PYRO_ARMAGEDDON, "TAUNTATK_PYRO_ARMAGEDDON";

	/// The Pyro's Scorch Shot taunt, which fires a flare.
	PYRO_SCORCHSHOT = taunt_attack_t_TAUNTATK_PYRO_SCORCHSHOT, "TAUNTATK_PYRO_SCORCHSHOT";

	/// The all-class guitar riff.
	ALLCLASS_GUITAR_RIFF = taunt_attack_t_TAUNTATK_ALLCLASS_GUITAR_RIFF, "TAUNTATK_ALLCLASS_GUITAR_RIFF";

	/// The Medic's heroic taunt.
	MEDIC_HEROIC_TAUNT = taunt_attack_t_TAUNTATK_MEDIC_HEROIC_TAUNT, "TAUNTATK_MEDIC_HEROIC_TAUNT";

	/// The Pyro's Gas Blast.
	PYRO_GASBLAST = taunt_attack_t_TAUNTATK_PYRO_GASBLAST, "TAUNTATK_PYRO_GASBLAST";

	/// The Engineer's trick shot.
	ENGINEER_TRICKSHOT = taunt_attack_t_TAUNTATK_ENGINEER_TRICKSHOT, "TAUNTATK_ENGINEER_TRICKSHOT";
}

/// One of TF2's taunt attacks (`taunt_attack_t`), as a taunt schedules it
/// ([`ScheduledAttack`]).
///
/// Any number is an attack, since the player keeps whatever number is set,
/// and TF2's updates add attacks: [`Self::name`] tells TF2's own apart.
#[doc(alias("taunt_attack_t", "m_iTauntAttack"))]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TauntAttack(c_int);

impl TauntAttack {
	/// The attack TF2 numbers `raw`, whether or not it names it.
	pub const fn from_raw(raw: c_int) -> Self {
		Self(raw)
	}

	/// TF2's number for the attack.
	pub const fn to_raw(self) -> c_int {
		self.0
	}
}

impl fmt::Debug for TauntAttack {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.name() {
			Some(name) => f.write_str(name),
			None => f.debug_tuple("TauntAttack").field(&self.0).finish(),
		}
	}
}

/// A taunt the game starts from code rather than from an item, which
/// [`PlayerTaunts::taunt`] starts (`taunts_t`).
///
/// The game's other kinds play an item's taunt, which
/// [`PlayerTaunts::taunt_slot`] starts by its loadout slot, or are reserved
/// for its own code.
#[doc(alias("taunts_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TauntKind {
	/// `TAUNT_BASE_WEAPON`: the taunt of the player's active weapon, as the
	/// taunt key plays without a taunt item.
	#[doc(alias("TAUNT_BASE_WEAPON"))]
	BaseWeapon,

	/// `TAUNT_SHOW_ITEM`: showing off the active weapon to those nearby.
	#[doc(alias("TAUNT_SHOW_ITEM"))]
	ShowItem,
}

impl TauntKind {
	/// The game's number for the kind.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::BaseWeapon => sys::taunts_t_TAUNT_BASE_WEAPON as c_int,
			Self::ShowItem => sys::taunts_t_TAUNT_SHOW_ITEM as c_int,
		}
	}
}

/// A taunt a player is in, as the game networks it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TauntState {
	/// Whether the taunt lets the player move, as some press-and-hold taunts
	/// do (`m_bAllowMoveDuringTaunt`).
	pub can_move: bool,

	/// The response concept the taunt was played for (`m_iTauntConcept`).
	pub concept: c_int,

	/// The taunt item playing, if any (`m_iTauntItemDefIndex`).
	pub item: Option<ItemDefinitionIndex>,

	/// The kind of taunt, a `taunts_t` (`m_iTauntIndex`).
	pub kind: c_int,

	/// How fast the taunt moves the player (`m_flCurrentTauntMoveSpeed`).
	pub move_speed: f32,
}

impl TauntState {
	/// Whether the taunt is a press-and-hold taunt from an item
	/// (`TAUNT_LONG`), which [`PlayerTaunts::end_long_taunt`] ends.
	pub const fn is_long(&self) -> bool {
		self.kind == sys::taunts_t_TAUNT_LONG as c_int
	}
}
