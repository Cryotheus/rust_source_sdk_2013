//! TF2 player conditions, including the object-backed conditions in
//! `tf_condition.cpp` and the remaining `CTFPlayerShared` conditions.
//!
//! Calls the game's native methods through their typed binding descriptors.
//! No script VM is needed, and no condition bits are written directly: the
//! game runs its normal add/remove notifications, durations and cleanup.

use crate::Server;
use crate::datatables::{ServerClass, Storage};
use crate::entities::Entity;
use crate::entities::EntityHandle;
use crate::tf2::player_methods;
use crate::tf2::script_instances::{ScriptInstance, ScriptInstanceError};
use sdk_raw::tf2::conditions as raw;
use sdk_raw::tf2::script_binding::BindingError;
use std::cell::Cell;
use std::collections::HashMap;
use std::ffi::CStr;
use std::ptr::NonNull;

/// The `CTFPlayerShared` variables holding each word of condition bits, in
/// order (`CConditionVars`).
const WORD_NAMES: [&CStr; WORDS] = [
	c"m_nPlayerCond",
	c"m_nPlayerCondEx",
	c"m_nPlayerCondEx2",
	c"m_nPlayerCondEx3",
	c"m_nPlayerCondEx4",
];

/// How many 32-bit words hold every condition's bit.
const WORDS: usize = (sys::ETFCond_TF_COND_LAST as usize).div_ceil(32);

/// A valid `ETFCond` identifier from TF2's `tf_shareddefs.h`. Each has a
/// constant here, searchable by the game's name, and [`Self::all`] lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[doc(alias("ETFCond"))]
pub struct Condition(i32);

impl Condition {
	/// Immune to afterburn.
	#[doc(alias("TF_COND_AFTERBURN_IMMUNE"))]
	pub const AFTERBURN_IMMUNE: Self = Self(sys::ETFCond_TF_COND_AFTERBURN_IMMUNE);

	/// A Sniper aiming or a Heavy using the minigun.
	#[doc(alias("TF_COND_AIMING"))]
	pub const AIMING: Self = Self(sys::ETFCond_TF_COND_AIMING);

	/// Caught in an air current: slides along surfaces, with less air control,
	/// until touching the ground.
	#[doc(alias("TF_COND_AIR_CURRENT"))]
	pub const AIR_CURRENT: Self = Self(sys::ETFCond_TF_COND_AIR_CURRENT);

	/// A balloon head: a larger head, and floatier jumps.
	#[doc(alias("TF_COND_BALLOON_HEAD"))]
	pub const BALLOON_HEAD: Self = Self(sys::ETFCond_TF_COND_BALLOON_HEAD);

	/// Immune to explosions.
	#[doc(alias("TF_COND_BLAST_IMMUNE"))]
	pub const BLAST_IMMUNE: Self = Self(sys::ETFCond_TF_COND_BLAST_IMMUNE);

	/// Blast jumping.
	#[doc(alias("TF_COND_BLASTJUMPING"))]
	pub const BLAST_JUMPING: Self = Self(sys::ETFCond_TF_COND_BLASTJUMPING);

	/// Bleeding.
	#[doc(alias("TF_COND_BLEEDING"))]
	pub const BLEEDING: Self = Self(sys::ETFCond_TF_COND_BLEEDING);

	/// Immune to bullets.
	#[doc(alias("TF_COND_BULLET_IMMUNE"))]
	pub const BULLET_IMMUNE: Self = Self(sys::ETFCond_TF_COND_BULLET_IMMUNE);

	/// On fire.
	#[doc(alias("TF_COND_BURNING"))]
	pub const BURNING: Self = Self(sys::ETFCond_TF_COND_BURNING);

	/// A Pyro on fire, as from the Dragon's Fury, despite Pyros' afterburn
	/// immunity.
	#[doc(alias("TF_COND_BURNING_PYRO"))]
	pub const BURNING_PYRO: Self = Self(sys::ETFCond_TF_COND_BURNING_PYRO);

	/// Unable to switch away from the melee weapon.
	#[doc(alias("TF_COND_CANNOT_SWITCH_FROM_MELEE"))]
	pub const CANNOT_SWITCH_FROM_MELEE: Self = Self(sys::ETFCond_TF_COND_CANNOT_SWITCH_FROM_MELEE);

	/// On the losing team of a competitive match.
	#[doc(alias("TF_COND_COMPETITIVE_LOSER"))]
	pub const COMPETITIVE_LOSER: Self = Self(sys::ETFCond_TF_COND_COMPETITIVE_LOSER);

	/// On the winning team of a competitive match.
	#[doc(alias("TF_COND_COMPETITIVE_WINNER"))]
	pub const COMPETITIVE_WINNER: Self = Self(sys::ETFCond_TF_COND_COMPETITIVE_WINNER);

	/// The critical boost reserved for the Kritzkrieg and revenge crits.
	#[doc(alias("TF_COND_CRITBOOSTED"))]
	pub const CRITBOOSTED: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED);

	/// The winning team's critical boost after a round.
	#[doc(alias("TF_COND_CRITBOOSTED_BONUS_TIME"))]
	pub const CRITBOOSTED_BONUS_TIME: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED_BONUS_TIME);

	/// A critical boost from a chance on dealing damage.
	#[doc(alias("TF_COND_CRITBOOSTED_CARD_EFFECT"))]
	pub const CRITBOOSTED_CARD_EFFECT: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED_CARD_EFFECT);

	/// The critical boost for capturing a flag.
	#[doc(alias("TF_COND_CRITBOOSTED_CTF_CAPTURE"))]
	pub const CRITBOOSTED_CTF_CAPTURE: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED_CTF_CAPTURE);

	/// The critical boost at the end of a Demoman's shield charge.
	#[doc(alias("TF_COND_CRITBOOSTED_DEMO_CHARGE"))]
	pub const CRITBOOSTED_DEMO_CHARGE: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED_DEMO_CHARGE);

	/// Arena's first blood critical boost.
	#[doc(alias("TF_COND_CRITBOOSTED_FIRST_BLOOD"))]
	pub const CRITBOOSTED_FIRST_BLOOD: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED_FIRST_BLOOD);

	/// A critical boost from a kill, as with the Killing Gloves of Boxing.
	#[doc(alias("TF_COND_CRITBOOSTED_ON_KILL"))]
	pub const CRITBOOSTED_ON_KILL: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED_ON_KILL);

	/// A critical boost from a Halloween pumpkin.
	#[doc(alias("TF_COND_CRITBOOSTED_PUMPKIN"))]
	pub const CRITBOOSTED_PUMPKIN: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED_PUMPKIN);

	/// The Phlogistinator's critical boost.
	#[doc(alias("TF_COND_CRITBOOSTED_RAGE_BUFF"))]
	pub const CRITBOOSTED_RAGE_BUFF: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED_RAGE_BUFF);

	/// Mannpower's temporary critical boost.
	#[doc(alias("TF_COND_CRITBOOSTED_RUNE_TEMP"))]
	pub const CRITBOOSTED_RUNE_TEMP: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED_RUNE_TEMP);

	/// A critical boost that items and scripts give.
	#[doc(alias("TF_COND_CRITBOOSTED_USER_BUFF"))]
	pub const CRITBOOSTED_USER_BUFF: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED_USER_BUFF);

	/// The Battalion's Backup: takes less damage, and no critical hits.
	#[doc(alias("TF_COND_DEFENSEBUFF"))]
	pub const DEFENSE_BUFF: Self = Self(sys::ETFCond_TF_COND_DEFENSEBUFF);

	/// Takes much less damage, while critical hits still land.
	#[doc(alias("TF_COND_DEFENSEBUFF_HIGH"))]
	pub const DEFENSE_BUFF_HIGH: Self = Self(sys::ETFCond_TF_COND_DEFENSEBUFF_HIGH);

	/// Takes less damage, while critical hits still land.
	#[doc(alias("TF_COND_DEFENSEBUFF_NO_CRIT_BLOCK"))]
	pub const DEFENSE_BUFF_NO_CRIT_BLOCK: Self =
		Self(sys::ETFCond_TF_COND_DEFENSEBUFF_NO_CRIT_BLOCK);

	/// The glowing eyes of a Demoman whose sword has taken heads.
	#[doc(alias("TF_COND_DEMO_BUFF"))]
	pub const DEMO_BUFF: Self = Self(sys::ETFCond_TF_COND_DEMO_BUFF);

	/// The half second after a Spy's disguise changes.
	#[doc(alias("TF_COND_DISGUISE_WEARINGOFF"))]
	pub const DISGUISE_WEARING_OFF: Self = Self(sys::ETFCond_TF_COND_DISGUISE_WEARINGOFF);

	/// Wearing a Spy's disguise.
	#[doc(alias("TF_COND_DISGUISED"))]
	pub const DISGUISED: Self = Self(sys::ETFCond_TF_COND_DISGUISED);

	/// A Spy disguised as a dispenser.
	#[doc(alias("TF_COND_DISGUISED_AS_DISPENSER"))]
	pub const DISGUISED_AS_DISPENSER: Self = Self(sys::ETFCond_TF_COND_DISGUISED_AS_DISPENSER);

	/// A Spy putting on a disguise.
	#[doc(alias("TF_COND_DISGUISING"))]
	pub const DISGUISING: Self = Self(sys::ETFCond_TF_COND_DISGUISING);

	/// Reserved, and unused by the game.
	#[doc(alias("TF_COND_DONOTUSE_0"))]
	pub const DO_NOT_USE_0: Self = Self(sys::ETFCond_TF_COND_DONOTUSE_0);

	/// Crit-a-Cola's mini-crits.
	#[doc(alias("TF_COND_ENERGY_BUFF"))]
	pub const ENERGY_BUFF: Self = Self(sys::ETFCond_TF_COND_ENERGY_BUFF);

	/// A Spy who just feigned death with the Dead Ringer.
	#[doc(alias("TF_COND_FEIGN_DEATH"))]
	pub const FEIGN_DEATH: Self = Self(sys::ETFCond_TF_COND_FEIGN_DEATH);

	/// Immune to fire.
	#[doc(alias("TF_COND_FIRE_IMMUNE"))]
	pub const FIRE_IMMUNE: Self = Self(sys::ETFCond_TF_COND_FIRE_IMMUNE);

	/// Frozen input: the player cannot act.
	#[doc(alias("TF_COND_FREEZE_INPUT"))]
	pub const FREEZE_INPUT: Self = Self(sys::ETFCond_TF_COND_FREEZE_INPUT);

	/// Covered in Gas Passer gas: catches fire from any damage.
	#[doc(alias("TF_COND_GAS"))]
	pub const GAS: Self = Self(sys::ETFCond_TF_COND_GAS);

	/// Pulled by another player's grappling hook.
	#[doc(alias("TF_COND_GRAPPLED_BY_PLAYER"))]
	pub const GRAPPLED_BY_PLAYER: Self = Self(sys::ETFCond_TF_COND_GRAPPLED_BY_PLAYER);

	/// Pulled toward a player by a grappling hook.
	#[doc(alias("TF_COND_GRAPPLED_TO_PLAYER"))]
	pub const GRAPPLED_TO_PLAYER: Self = Self(sys::ETFCond_TF_COND_GRAPPLED_TO_PLAYER);

	/// Pulled by a grappling hook.
	#[doc(alias("TF_COND_GRAPPLINGHOOK"))]
	pub const GRAPPLING_HOOK: Self = Self(sys::ETFCond_TF_COND_GRAPPLINGHOOK);

	/// Bleeding from an enemy's grappling hook.
	#[doc(alias("TF_COND_GRAPPLINGHOOK_BLEEDING"))]
	pub const GRAPPLING_HOOK_BLEEDING: Self = Self(sys::ETFCond_TF_COND_GRAPPLINGHOOK_BLEEDING);

	/// Hanging from a grappling hook.
	#[doc(alias("TF_COND_GRAPPLINGHOOK_LATCHED"))]
	pub const GRAPPLING_HOOK_LATCHED: Self = Self(sys::ETFCond_TF_COND_GRAPPLINGHOOK_LATCHED);

	/// Spared fall damage after a grappling hook's pull.
	#[doc(alias("TF_COND_GRAPPLINGHOOK_SAFEFALL"))]
	pub const GRAPPLING_HOOK_SAFE_FALL: Self = Self(sys::ETFCond_TF_COND_GRAPPLINGHOOK_SAFEFALL);

	/// A Halloween bomb head.
	#[doc(alias("TF_COND_HALLOWEEN_BOMB_HEAD"))]
	pub const HALLOWEEN_BOMB_HEAD: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_BOMB_HEAD);

	/// A Halloween ghost.
	#[doc(alias("TF_COND_HALLOWEEN_GHOST_MODE"))]
	pub const HALLOWEEN_GHOST_MODE: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_GHOST_MODE);

	/// Made giant by a Halloween spell.
	#[doc(alias("TF_COND_HALLOWEEN_GIANT"))]
	pub const HALLOWEEN_GIANT: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_GIANT);

	/// Healing in Halloween's underworld.
	#[doc(alias("TF_COND_HALLOWEEN_HELL_HEAL"))]
	pub const HALLOWEEN_HELL_HEAL: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_HELL_HEAL);

	/// In Halloween's underworld.
	#[doc(alias("TF_COND_HALLOWEEN_IN_HELL"))]
	pub const HALLOWEEN_IN_HELL: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_IN_HELL);

	/// Driving a Halloween bumper car.
	#[doc(alias("TF_COND_HALLOWEEN_KART"))]
	pub const HALLOWEEN_KART: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_KART);

	/// A cage around the player's Halloween bumper car.
	#[doc(alias("TF_COND_HALLOWEEN_KART_CAGE"))]
	pub const HALLOWEEN_KART_CAGE: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_KART_CAGE);

	/// A Halloween bumper car's speed boost.
	#[doc(alias("TF_COND_HALLOWEEN_KART_DASH"))]
	pub const HALLOWEEN_KART_DASH: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_KART_DASH);

	/// Halloween's quick healing.
	#[doc(alias("TF_COND_HALLOWEEN_QUICK_HEAL"))]
	pub const HALLOWEEN_QUICK_HEAL: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_QUICK_HEAL);

	/// A Halloween speed boost.
	#[doc(alias("TF_COND_HALLOWEEN_SPEED_BOOST"))]
	pub const HALLOWEEN_SPEED_BOOST: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_SPEED_BOOST);

	/// Made to dance by Halloween's thriller taunt.
	#[doc(alias("TF_COND_HALLOWEEN_THRILLER"))]
	pub const HALLOWEEN_THRILLER: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_THRILLER);

	/// Made tiny by a Halloween spell.
	#[doc(alias("TF_COND_HALLOWEEN_TINY"))]
	pub const HALLOWEEN_TINY: Self = Self(sys::ETFCond_TF_COND_HALLOWEEN_TINY);

	/// Receives less healing.
	#[doc(alias("TF_COND_HEALING_DEBUFF"))]
	pub const HEALING_DEBUFF: Self = Self(sys::ETFCond_TF_COND_HEALING_DEBUFF);

	/// Healed by a Medic or a dispenser, which can overheal.
	#[doc(alias("TF_COND_HEALTH_BUFF"))]
	pub const HEALTH_BUFF: Self = Self(sys::ETFCond_TF_COND_HEALTH_BUFF);

	/// Overhealed past maximum health.
	#[doc(alias("TF_COND_HEALTH_OVERHEALED"))]
	pub const HEALTH_OVERHEALED: Self = Self(sys::ETFCond_TF_COND_HEALTH_OVERHEALED);

	/// Immune to knockback.
	#[doc(alias("TF_COND_IMMUNE_TO_PUSHBACK"))]
	pub const IMMUNE_TO_PUSHBACK: Self = Self(sys::ETFCond_TF_COND_IMMUNE_TO_PUSHBACK);

	/// Invulnerability, as from a Medic's ÜberCharge.
	#[doc(alias("TF_COND_INVULNERABLE"))]
	pub const INVULNERABLE: Self = Self(sys::ETFCond_TF_COND_INVULNERABLE);

	/// Brief invulnerability from a chance on taking damage.
	#[doc(alias("TF_COND_INVULNERABLE_CARD_EFFECT"))]
	pub const INVULNERABLE_CARD_EFFECT: Self = Self(sys::ETFCond_TF_COND_INVULNERABLE_CARD_EFFECT);

	/// Invulnerability that shows only when the player is damaged.
	#[doc(alias("TF_COND_INVULNERABLE_HIDE_UNLESS_DAMAGED"))]
	pub const INVULNERABLE_HIDE_UNLESS_DAMAGED: Self =
		Self(sys::ETFCond_TF_COND_INVULNERABLE_HIDE_UNLESS_DAMAGED);

	/// Invulnerability that items and scripts give.
	#[doc(alias("TF_COND_INVULNERABLE_USER_BUFF"))]
	pub const INVULNERABLE_USER_BUFF: Self = Self(sys::ETFCond_TF_COND_INVULNERABLE_USER_BUFF);

	/// An ÜberCharge wearing off.
	#[doc(alias("TF_COND_INVULNERABLE_WEARINGOFF"))]
	pub const INVULNERABLE_WEARING_OFF: Self = Self(sys::ETFCond_TF_COND_INVULNERABLE_WEARINGOFF);

	/// Buffed by a nearby teammate's Mannpower King powerup.
	#[doc(alias("TF_COND_KING_BUFFED"))]
	pub const KING_BUFFED: Self = Self(sys::ETFCond_TF_COND_KING_BUFFED);

	/// Knocked into the air.
	#[doc(alias("TF_COND_KNOCKED_INTO_AIR"))]
	pub const KNOCKED_INTO_AIR: Self = Self(sys::ETFCond_TF_COND_KNOCKED_INTO_AIR);

	/// Lost footing: less friction, and no sticking to the ground until slowed
	/// below `tf_movement_lost_footing_restick`.
	#[doc(alias("TF_COND_LOST_FOOTING"))]
	pub const LOST_FOOTING: Self = Self(sys::ETFCond_TF_COND_LOST_FOOTING);

	/// Covered in Mad Milk: attackers heal from their hits.
	#[doc(alias("TF_COND_MAD_MILK"))]
	pub const MAD_MILK: Self = Self(sys::ETFCond_TF_COND_MAD_MILK);

	/// Marked for death: TF2 promotes non-critical damage to the player to a
	/// mini critical hit.
	#[doc(alias("TF_COND_MARKEDFORDEATH"))]
	pub const MARKED_FOR_DEATH: Self = Self(sys::ETFCond_TF_COND_MARKEDFORDEATH);

	/// Marked for death, without the sound.
	#[doc(alias("TF_COND_MARKEDFORDEATH_SILENT"))]
	pub const MARKED_FOR_DEATH_SILENT: Self = Self(sys::ETFCond_TF_COND_MARKEDFORDEATH_SILENT);

	/// Unused by the game.
	#[doc(alias("TF_COND_MEDIGUN_DEBUFF"))]
	pub const MEDIGUN_DEBUFF: Self = Self(sys::ETFCond_TF_COND_MEDIGUN_DEBUFF);

	/// The Vaccinator's healing resistance to explosions.
	#[doc(alias("TF_COND_MEDIGUN_SMALL_BLAST_RESIST"))]
	pub const MEDIGUN_SMALL_BLAST_RESIST: Self =
		Self(sys::ETFCond_TF_COND_MEDIGUN_SMALL_BLAST_RESIST);

	/// The Vaccinator's healing resistance to bullets.
	#[doc(alias("TF_COND_MEDIGUN_SMALL_BULLET_RESIST"))]
	pub const MEDIGUN_SMALL_BULLET_RESIST: Self =
		Self(sys::ETFCond_TF_COND_MEDIGUN_SMALL_BULLET_RESIST);

	/// The Vaccinator's healing resistance to fire.
	#[doc(alias("TF_COND_MEDIGUN_SMALL_FIRE_RESIST"))]
	pub const MEDIGUN_SMALL_FIRE_RESIST: Self =
		Self(sys::ETFCond_TF_COND_MEDIGUN_SMALL_FIRE_RESIST);

	/// The Vaccinator's ÜberCharge against explosions.
	#[doc(alias("TF_COND_MEDIGUN_UBER_BLAST_RESIST"))]
	pub const MEDIGUN_UBER_BLAST_RESIST: Self =
		Self(sys::ETFCond_TF_COND_MEDIGUN_UBER_BLAST_RESIST);

	/// The Vaccinator's ÜberCharge against bullets.
	#[doc(alias("TF_COND_MEDIGUN_UBER_BULLET_RESIST"))]
	pub const MEDIGUN_UBER_BULLET_RESIST: Self =
		Self(sys::ETFCond_TF_COND_MEDIGUN_UBER_BULLET_RESIST);

	/// The Vaccinator's ÜberCharge against fire.
	#[doc(alias("TF_COND_MEDIGUN_UBER_FIRE_RESIST"))]
	pub const MEDIGUN_UBER_FIRE_RESIST: Self = Self(sys::ETFCond_TF_COND_MEDIGUN_UBER_FIRE_RESIST);

	/// The Quick-Fix's ÜberCharge: immune to knockback and movement-impairing
	/// effects.
	#[doc(alias("TF_COND_MEGAHEAL"))]
	pub const MEGAHEAL: Self = Self(sys::ETFCond_TF_COND_MEGAHEAL);

	/// Limited to melee weapons.
	#[doc(alias("TF_COND_MELEE_ONLY"))]
	pub const MELEE_ONLY: Self = Self(sys::ETFCond_TF_COND_MELEE_ONLY);

	/// Mini-crits from a kill.
	#[doc(alias("TF_COND_MINICRITBOOSTED_ON_KILL"))]
	pub const MINI_CRITBOOSTED_ON_KILL: Self = Self(sys::ETFCond_TF_COND_MINICRITBOOSTED_ON_KILL);

	/// A Mann vs. Machine robot stunned by a radio wave; bots only.
	#[doc(alias("TF_COND_MVM_BOT_STUN_RADIOWAVE"))]
	pub const MVM_BOT_STUN_RADIOWAVE: Self = Self(sys::ETFCond_TF_COND_MVM_BOT_STUN_RADIOWAVE);

	/// Mini-crits, while unable to be healed.
	#[doc(alias("TF_COND_NOHEALINGDAMAGEBUFF"))]
	pub const NO_HEALING_DAMAGE_BUFF: Self = Self(sys::ETFCond_TF_COND_NOHEALINGDAMAGEBUFF);

	/// Obscured by smoke: attacks on the player can miss.
	#[doc(alias("TF_COND_OBSCURED_SMOKE"))]
	pub const OBSCURED_SMOKE: Self = Self(sys::ETFCond_TF_COND_OBSCURED_SMOKE);

	/// The Buff Banner's mini-crits.
	#[doc(alias("TF_COND_OFFENSEBUFF"))]
	pub const OFFENSE_BUFF: Self = Self(sys::ETFCond_TF_COND_OFFENSEBUFF);

	/// An open parachute.
	#[doc(alias("TF_COND_PARACHUTE_ACTIVE"))]
	pub const PARACHUTE_ACTIVE: Self = Self(sys::ETFCond_TF_COND_PARACHUTE_ACTIVE);

	/// Has opened a parachute since leaving the ground, which may have closed
	/// since ([`Self::PARACHUTE_ACTIVE`]).
	#[doc(alias("TF_COND_PARACHUTE_DEPLOYED"))]
	pub const PARACHUTE_DEPLOYED: Self = Self(sys::ETFCond_TF_COND_PARACHUTE_DEPLOYED);

	/// Intercepting a PASS Time pass, which shields the player as phasing does.
	#[doc(alias("TF_COND_PASSTIME_INTERCEPTION"))]
	pub const PASSTIME_INTERCEPTION: Self = Self(sys::ETFCond_TF_COND_PASSTIME_INTERCEPTION);

	/// Carrying PASS Time's ball with no teammates nearby.
	#[doc(alias("TF_COND_PASSTIME_PENALTY_DEBUFF"))]
	pub const PASSTIME_PENALTY_DEBUFF: Self = Self(sys::ETFCond_TF_COND_PASSTIME_PENALTY_DEBUFF);

	/// Phasing, as from Bonk! Atomic Punch: dodges damage, but cannot attack.
	#[doc(alias("TF_COND_PHASE"))]
	pub const PHASE: Self = Self(sys::ETFCond_TF_COND_PHASE);

	/// Infected by Mannpower's Plague.
	#[doc(alias("TF_COND_PLAGUE"))]
	pub const PLAGUE: Self = Self(sys::ETFCond_TF_COND_PLAGUE);

	/// Dominant in Mannpower, which weakens the player's powerup.
	#[doc(alias("TF_COND_POWERUPMODE_DOMINANT"))]
	pub const POWERUP_MODE_DOMINANT: Self = Self(sys::ETFCond_TF_COND_POWERUPMODE_DOMINANT);

	/// Survives one fatal hit with 1 health, which uses the condition up.
	#[doc(alias("TF_COND_PREVENT_DEATH"))]
	pub const PREVENT_DEATH: Self = Self(sys::ETFCond_TF_COND_PREVENT_DEATH);

	/// In the purgatory of Halloween's underworld.
	#[doc(alias("TF_COND_PURGATORY"))]
	pub const PURGATORY: Self = Self(sys::ETFCond_TF_COND_PURGATORY);

	/// Healing nearby teammates, as the Amputator's taunt does.
	#[doc(alias("TF_COND_RADIUSHEAL"))]
	pub const RADIUS_HEAL: Self = Self(sys::ETFCond_TF_COND_RADIUSHEAL);

	/// Healing nearby teammates, from a chance on dealing damage.
	#[doc(alias("TF_COND_RADIUSHEAL_ON_DAMAGE"))]
	pub const RADIUS_HEAL_ON_DAMAGE: Self = Self(sys::ETFCond_TF_COND_RADIUSHEAL_ON_DAMAGE);

	/// The Concheror: heals from damage dealt, and moves faster.
	#[doc(alias("TF_COND_REGENONDAMAGEBUFF"))]
	pub const REGEN_ON_DAMAGE_BUFF: Self = Self(sys::ETFCond_TF_COND_REGENONDAMAGEBUFF);

	/// A reprogrammed Mann vs. Machine robot; bots only.
	#[doc(alias("TF_COND_REPROGRAMMED"))]
	pub const REPROGRAMMED: Self = Self(sys::ETFCond_TF_COND_REPROGRAMMED);

	/// Flying with the Thermal Thruster.
	#[doc(alias("TF_COND_ROCKETPACK"))]
	pub const ROCKET_PACK: Self = Self(sys::ETFCond_TF_COND_ROCKETPACK);

	/// Mannpower's Agility powerup.
	#[doc(alias("TF_COND_RUNE_AGILITY"))]
	pub const RUNE_AGILITY: Self = Self(sys::ETFCond_TF_COND_RUNE_AGILITY);

	/// Mannpower's Haste powerup.
	#[doc(alias("TF_COND_RUNE_HASTE"))]
	pub const RUNE_HASTE: Self = Self(sys::ETFCond_TF_COND_RUNE_HASTE);

	/// Mannpower's Imbalance powerup.
	#[doc(alias("TF_COND_RUNE_IMBALANCE"))]
	pub const RUNE_IMBALANCE: Self = Self(sys::ETFCond_TF_COND_RUNE_IMBALANCE);

	/// Mannpower's King powerup.
	#[doc(alias("TF_COND_RUNE_KING"))]
	pub const RUNE_KING: Self = Self(sys::ETFCond_TF_COND_RUNE_KING);

	/// Mannpower's Knockout powerup.
	#[doc(alias("TF_COND_RUNE_KNOCKOUT"))]
	pub const RUNE_KNOCKOUT: Self = Self(sys::ETFCond_TF_COND_RUNE_KNOCKOUT);

	/// Mannpower's Plague powerup.
	#[doc(alias("TF_COND_RUNE_PLAGUE"))]
	pub const RUNE_PLAGUE: Self = Self(sys::ETFCond_TF_COND_RUNE_PLAGUE);

	/// Mannpower's Precision powerup.
	#[doc(alias("TF_COND_RUNE_PRECISION"))]
	pub const RUNE_PRECISION: Self = Self(sys::ETFCond_TF_COND_RUNE_PRECISION);

	/// Mannpower's Reflect powerup.
	#[doc(alias("TF_COND_RUNE_REFLECT"))]
	pub const RUNE_REFLECT: Self = Self(sys::ETFCond_TF_COND_RUNE_REFLECT);

	/// Mannpower's Regeneration powerup.
	#[doc(alias("TF_COND_RUNE_REGEN"))]
	pub const RUNE_REGEN: Self = Self(sys::ETFCond_TF_COND_RUNE_REGEN);

	/// Mannpower's Resistance powerup.
	#[doc(alias("TF_COND_RUNE_RESIST"))]
	pub const RUNE_RESIST: Self = Self(sys::ETFCond_TF_COND_RUNE_RESIST);

	/// Mannpower's Strength powerup.
	#[doc(alias("TF_COND_RUNE_STRENGTH"))]
	pub const RUNE_STRENGTH: Self = Self(sys::ETFCond_TF_COND_RUNE_STRENGTH);

	/// Mannpower's Supernova powerup.
	#[doc(alias("TF_COND_RUNE_SUPERNOVA"))]
	pub const RUNE_SUPERNOVA: Self = Self(sys::ETFCond_TF_COND_RUNE_SUPERNOVA);

	/// Mannpower's Vampire powerup.
	#[doc(alias("TF_COND_RUNE_VAMPIRE"))]
	pub const RUNE_VAMPIRE: Self = Self(sys::ETFCond_TF_COND_RUNE_VAMPIRE);

	/// A Mann vs. Machine robot with a sapper on it; bots only.
	#[doc(alias("TF_COND_SAPPED"))]
	pub const SAPPED: Self = Self(sys::ETFCond_TF_COND_SAPPED);

	/// On a teleporter entrance that is about to send the player.
	#[doc(alias("TF_COND_SELECTED_TO_TELEPORT"))]
	pub const SELECTED_TO_TELEPORT: Self = Self(sys::ETFCond_TF_COND_SELECTED_TO_TELEPORT);

	/// A Demoman charging with a shield.
	#[doc(alias("TF_COND_SHIELD_CHARGE"))]
	pub const SHIELD_CHARGE: Self = Self(sys::ETFCond_TF_COND_SHIELD_CHARGE);

	/// The Hitman's Heatmaker's focus: charges faster.
	#[doc(alias("TF_COND_SNIPERCHARGE_RAGE_BUFF"))]
	pub const SNIPER_CHARGE_RAGE_BUFF: Self = Self(sys::ETFCond_TF_COND_SNIPERCHARGE_RAGE_BUFF);

	/// The Soda Popper's hype: extra jumps in mid-air.
	#[doc(alias("TF_COND_SODAPOPPER_HYPE"))]
	pub const SODA_POPPER_HYPE: Self = Self(sys::ETFCond_TF_COND_SODAPOPPER_HYPE);

	/// A movement speed boost.
	#[doc(alias("TF_COND_SPEED_BOOST"))]
	pub const SPEED_BOOST: Self = Self(sys::ETFCond_TF_COND_SPEED_BOOST);

	/// A cloaked Spy.
	#[doc(alias("TF_COND_STEALTHED"))]
	pub const STEALTHED: Self = Self(sys::ETFCond_TF_COND_STEALTHED);

	/// A cloaked Spy flickering into view, as after bumping into an enemy.
	#[doc(alias("TF_COND_STEALTHED_BLINK"))]
	pub const STEALTHED_BLINK: Self = Self(sys::ETFCond_TF_COND_STEALTHED_BLINK);

	/// Invisibility, for any class, that items and scripts give.
	#[doc(alias("TF_COND_STEALTHED_USER_BUFF"))]
	pub const STEALTHED_USER_BUFF: Self = Self(sys::ETFCond_TF_COND_STEALTHED_USER_BUFF);

	/// Fading out of [`Self::STEALTHED_USER_BUFF`]'s invisibility.
	#[doc(alias("TF_COND_STEALTHED_USER_BUFF_FADING"))]
	pub const STEALTHED_USER_BUFF_FADING: Self =
		Self(sys::ETFCond_TF_COND_STEALTHED_USER_BUFF_FADING);

	/// Stunned in any way; the stun flags tell how.
	#[doc(alias("TF_COND_STUNNED"))]
	pub const STUNNED: Self = Self(sys::ETFCond_TF_COND_STUNNED);

	/// Swims through the air.
	#[doc(alias("TF_COND_SWIMMING_CURSE"))]
	pub const SWIMMING_CURSE: Self = Self(sys::ETFCond_TF_COND_SWIMMING_CURSE);

	/// Swims through the air, without the swimming effects.
	#[doc(alias("TF_COND_SWIMMING_NO_EFFECTS"))]
	pub const SWIMMING_NO_EFFECTS: Self = Self(sys::ETFCond_TF_COND_SWIMMING_NO_EFFECTS);

	/// Taunting.
	#[doc(alias("TF_COND_TAUNTING"))]
	pub const TAUNTING: Self = Self(sys::ETFCond_TF_COND_TAUNTING);

	/// Sees teammates' glows, as players briefly do after spawning.
	#[doc(alias("TF_COND_TEAM_GLOWS"))]
	pub const TEAM_GLOWS: Self = Self(sys::ETFCond_TF_COND_TEAM_GLOWS);

	/// Recently teleported, which leaves the teleporter's glow.
	#[doc(alias("TF_COND_TELEPORTED"))]
	pub const TELEPORTED: Self = Self(sys::ETFCond_TF_COND_TELEPORTED);

	/// A temporary damage bonus (`CTFPlayerShared::AddTmpDamageBonus`).
	#[doc(alias("TF_COND_TMPDAMAGEBONUS"))]
	pub const TEMPORARY_DAMAGE_BONUS: Self = Self(sys::ETFCond_TF_COND_TMPDAMAGEBONUS);

	/// Covered in Jarate: takes mini-crits.
	#[doc(alias("TF_COND_URINE"))]
	pub const URINE: Self = Self(sys::ETFCond_TF_COND_URINE);

	/// Zoomed in through a scope.
	#[doc(alias("TF_COND_ZOOMED"))]
	pub const ZOOMED: Self = Self(sys::ETFCond_TF_COND_ZOOMED);

	/// Every condition, in the game's order.
	pub fn all() -> impl ExactSizeIterator<Item = Self> + DoubleEndedIterator {
		(0..sys::ETFCond_TF_COND_LAST).map(Self)
	}

	/// Validates a raw identifier. Returns `None` for negative values and
	/// values at or above the `TF_COND_LAST` sentinel.
	pub const fn from_raw(raw: sys::ETFCond) -> Option<Self> {
		if raw >= 0 && raw < sys::ETFCond_TF_COND_LAST {
			Some(Self(raw))
		} else {
			None
		}
	}

	/// The raw `ETFCond` value.
	pub const fn to_raw(self) -> sys::ETFCond {
		self.0
	}
}

/// Condition lifetime. The game may keep a longer existing duration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConditionDuration(f32);

impl ConditionDuration {
	/// No expiry: TF2's `PERMANENT_CONDITION`, passed as -1 seconds.
	#[doc(alias("PERMANENT_CONDITION"))]
	pub const PERMANENT: Self = Self(raw::PERMANENT_CONDITION);

	/// A finite, nonnegative number of seconds. Zero expires on a subsequent
	/// game update; it does not mean permanent.
	pub fn seconds(seconds: f32) -> Option<Self> {
		(seconds.is_finite() && seconds >= 0.0).then_some(Self(seconds))
	}

	/// The duration passed to TF2: seconds, or -1 for [`Self::PERMANENT`].
	pub const fn as_raw(self) -> f32 {
		self.0
	}
}

/// A condition operation is unavailable for this game, entity, or binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConditionError {
	/// The server is not running TF2, or the entity's class name is not
	/// `player`.
	#[error("conditions require a TF2 player")]
	NotTfPlayer,

	/// The player's script class descriptors lack the native method, or its
	/// signature differs from the SDK's.
	#[error("the game does not expose the expected native condition method")]
	UnsupportedMethod,

	/// The native method's binding adapter reported failure.
	#[error("the native condition method rejected its arguments")]
	Rejected,

	/// The player's server class does not network its condition bits as
	/// `CTFPlayerShared` declares them, or the game DLL's interface or
	/// standard send proxies, through which they are found, are unavailable.
	#[error("the player's condition bits could not be located")]
	UnsupportedLayout,

	/// The provider's script instance, through which the game takes it, could
	/// not be found or made.
	#[error("the condition's provider has no script instance: {0}")]
	Provider(#[source] ScriptInstanceError),
}

impl From<BindingError> for ConditionError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// One player's conditions within the current engine callback.
///
/// Methods fail with [`ConditionError::UnsupportedMethod`] when the game
/// lacks the expected native method, and [`ConditionError::Rejected`] when
/// the method's binding reports failure.
#[derive(Debug, Clone, Copy)]
pub struct PlayerConditions<'s> {
	server: Server<'s>,
	player: Entity<'s>,
}

impl<'s> PlayerConditions<'s> {
	/// Wraps `player` for condition calls. Fails with
	/// [`ConditionError::NotTfPlayer`] unless the server runs TF2 and
	/// `player`'s class name is `player`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, ConditionError> {
		if !player_methods::is_tf_player(server, player) {
			return Err(ConditionError::NotTfPlayer);
		}

		Ok(Self { server, player })
	}

	/// Adds a condition without a provider, using TF2's normal duration rules.
	/// Returns whether it is active afterwards. TF2 can refuse additions, for
	/// example on dead players or outside the competitive match summary.
	#[doc(alias("AddCond", "AddCondEx"))]
	pub fn add(
		self,
		condition: Condition,
		duration: ConditionDuration,
	) -> Result<bool, ConditionError> {
		self.add_raw(condition, duration, std::ptr::null_mut())
	}

	/// Adds a condition through the native method, with `provider` a script
	/// instance or null for none, and returns whether it is active afterwards.
	fn add_raw(
		self,
		condition: Condition,
		duration: ConditionDuration,
		provider: sys::HSCRIPT,
	) -> Result<bool, ConditionError> {
		// SAFETY: A TF2 `player` is a CTFPlayer, live on the main thread for
		// this callback, whose game module stays loaded. The checked native
		// method only adds a validated condition; the provider is null, or a
		// script instance the VM registered, which the game resolves to its
		// entity and keeps by handle. Its condition effects respect the
		// callback's deferred entity-deletion contract.
		unsafe { raw::add_cond_ex(self.raw_player(), condition.0, duration.0, provider) }?;
		self.in_cond(condition)
	}

	/// Adds a condition as [`Self::add`] does, credited to `provider`, as a
	/// Medic's ÜberCharge is to the Medic.
	///
	/// The game keeps the provider, by handle, while the condition lasts, and
	/// credits a provider that is a player as it would the player who applied
	/// the condition: with an assist for a kill on a player it covered in
	/// Jarate, Mad Milk or gas, or marked for death, or for a kill by a player
	/// it buffed with a banner, and with the damage its invulnerability blocks.
	///
	/// The game takes the provider as its script instance, which is made for
	/// it, with a script scope, if it has none yet ([`ScriptInstance::of`]).
	/// Fails with [`ConditionError::Provider`] if that fails.
	#[doc(alias("AddCondEx", "GetConditionProvider"))]
	pub fn add_with_provider(
		self,
		condition: Condition,
		duration: ConditionDuration,
		provider: Entity<'s>,
	) -> Result<bool, ConditionError> {
		let provider =
			ScriptInstance::of(self.server, provider).map_err(ConditionError::Provider)?;

		self.add_raw(condition, duration, provider.as_raw())
	}

	/// The conditions the player has, all read at once from the bits the game
	/// keeps them in, without a call into the game for each, as
	/// [`Self::in_cond`] makes.
	///
	/// The bits are the `CTFPlayerShared` networked variables `m_nPlayerCond`,
	/// `m_nPlayerCondEx` through `m_nPlayerCondEx4`, and the condition list's
	/// `_condition_bits`, which `CTFPlayerShared::InCond` reads, found through
	/// the player's server class. Fails with
	/// [`ConditionError::UnsupportedLayout`] if they are not found as the game
	/// declares them.
	#[doc(alias("m_nPlayerCond", "m_nPlayerCondEx", "_condition_bits"))]
	pub fn bits(self) -> Result<ConditionBits, ConditionError> {
		let class = self
			.player
			.server_class()
			.ok_or(ConditionError::UnsupportedLayout)?;
		let layout = ConditionLayout::of(self.server, class)?;

		// SAFETY: The offsets were resolved in the player's own server class,
		// whose table describes where the player keeps these variables.
		Ok(unsafe { layout.read(self.player) })
	}

	/// Whether the player can be given debuffs, such as Jarate or bleeding:
	/// not while invulnerable ([`Self::is_invulnerable`]), phasing, or
	/// intercepting a PASS Time pass.
	#[doc(alias("CanBeDebuffed"))]
	pub fn can_be_debuffed(self) -> Result<bool, ConditionError> {
		self.predicate(c"CanBeDebuffed")
	}

	/// Removes each of `conditions` the player has, past any minimum duration,
	/// as a cleanse of [`ConditionBits::DEBUFFS`] would, and returns those
	/// removed.
	pub fn cleanse(self, conditions: ConditionBits) -> Result<ConditionBits, ConditionError> {
		let mut removed = ConditionBits::EMPTY;

		for condition in self.bits()?.intersection(conditions).iter() {
			if self.remove(condition, true)? {
				removed.insert(condition);
			}
		}

		Ok(removed)
	}

	/// The time left of a condition: [`ConditionDuration::PERMANENT`] for one
	/// that does not expire, and zero seconds for one the player does not have,
	/// as for one expiring on the next game update.
	///
	/// [`Condition::CRITBOOSTED`] keeps its time apart, in the game's condition
	/// list, so this does not tell it.
	#[doc(alias("GetCondDuration"))]
	pub fn duration(self, condition: Condition) -> Result<ConditionDuration, ConditionError> {
		// SAFETY: As for `add`. This is the native read-only query on a validated
		// player and condition.
		let seconds = unsafe { raw::get_cond_duration(self.raw_player(), condition.0) }?;

		// TF2 counts finite durations down to zero, and keeps -1 for permanent ones.
		if seconds < 0.0 {
			Ok(ConditionDuration::PERMANENT)
		} else {
			ConditionDuration::seconds(seconds).ok_or(ConditionError::Rejected)
		}
	}

	/// Checks both the object-backed condition list and all extended bitfields.
	#[doc(alias("InCond"))]
	pub fn in_cond(self, condition: Condition) -> Result<bool, ConditionError> {
		// SAFETY: As for `add`. This is the native read-only query on a
		// validated player and condition.
		Ok(unsafe { raw::in_cond(self.raw_player(), condition.0) }?)
	}

	/// Whether the player's controls are stunned: stunned with
	/// [`StunFlags::CONTROLS`](crate::tf2::effects::StunFlags::CONTROLS), which
	/// takes away the player's control.
	#[doc(alias("IsControlStunned"))]
	pub fn is_control_stunned(self) -> Result<bool, ConditionError> {
		self.predicate(c"IsControlStunned")
	}

	/// Whether the player is critically boosted: by any of the conditions that
	/// boost every weapon, by the Phlogistinator's while holding a primary
	/// weapon, or by the active weapon's attribute at low health
	/// (`CTFPlayerShared::IsCritBoosted`).
	#[doc(alias("IsCritBoosted"))]
	pub fn is_crit_boosted(self) -> Result<bool, ConditionError> {
		self.predicate(c"IsCritBoosted")
	}

	/// Whether the player is fully invisible: cloaked, and done fading out.
	#[doc(alias("IsFullyInvisible"))]
	pub fn is_fully_invisible(self) -> Result<bool, ConditionError> {
		self.predicate(c"IsFullyInvisible")
	}

	/// Whether the player is immune to knockback: from
	/// [`Condition::IMMUNE_TO_PUSHBACK`], the Quick-Fix's ÜberCharge, or a
	/// Heavy's spun-up minigun with that attribute.
	#[doc(alias("IsImmuneToPushback"))]
	pub fn is_immune_to_pushback(self) -> Result<bool, ConditionError> {
		self.predicate(c"IsImmuneToPushback")
	}

	/// Whether the player is invulnerable, by any of the
	/// [`Condition::INVULNERABLE`] conditions but the wearing off.
	#[doc(alias("IsInvulnerable"))]
	pub fn is_invulnerable(self) -> Result<bool, ConditionError> {
		self.predicate(c"IsInvulnerable")
	}

	/// Whether the player is stunned only in movement, with free controls.
	#[doc(alias("IsSnared"))]
	pub fn is_snared(self) -> Result<bool, ConditionError> {
		self.predicate(c"IsSnared")
	}

	/// Whether the player is cloaked or invisible, or fading out of an item's
	/// invisibility: [`Condition::STEALTHED`],
	/// [`Condition::STEALTHED_USER_BUFF`] or
	/// [`Condition::STEALTHED_USER_BUFF_FADING`].
	#[doc(alias("IsStealthed"))]
	pub fn is_stealthed(self) -> Result<bool, ConditionError> {
		self.predicate(c"IsStealthed")
	}

	/// The player whose conditions these are.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// Calls one of the player's native predicates, which take nothing.
	fn predicate(self, method: &CStr) -> Result<bool, ConditionError> {
		// SAFETY: As for `add`. The predicates only read the player's state.
		Ok(unsafe { player_methods::call_bool(self.player, method, &mut []) }?)
	}

	/// The player's pointer, for the native condition methods.
	fn raw_player(self) -> NonNull<sys::CBaseEntity> {
		// SAFETY: An entity's pointer is never null.
		unsafe { NonNull::new_unchecked(self.player.as_ptr()) }
	}

	/// Removes a condition. `ignore_duration` bypasses conditions' minimum
	/// duration (notably crit boost). Returns whether it is absent afterwards.
	#[doc(alias("RemoveCond", "RemoveCondEx"))]
	pub fn remove(
		self,
		condition: Condition,
		ignore_duration: bool,
	) -> Result<bool, ConditionError> {
		// SAFETY: As for `add`. This checked native method runs TF2's ordinary
		// removal path with a valid condition identifier.
		unsafe { raw::remove_cond_ex(self.raw_player(), condition.0, ignore_duration) }?;
		Ok(!self.in_cond(condition)?)
	}

	/// Runs TF2's full `RemoveAllCond` cleanup, including its object list.
	#[doc(alias("RemoveAllCond"))]
	pub fn remove_all(self) -> Result<(), ConditionError> {
		// SAFETY: As for `add`. The native method performs normal condition
		// cleanup without immediately deleting entities.
		unsafe { raw::remove_all_cond(self.raw_player()) }?;
		Ok(())
	}

	/// Sets the time left of a condition the player has, from which the game
	/// counts it down to its removal, or makes it permanent. Unlike adding the
	/// condition again, this can shorten it.
	///
	/// Returns whether the time was set: not for a condition the player lacks,
	/// whose time the game would keep for its next addition, nor for
	/// [`Condition::CRITBOOSTED`], whose time the game keeps apart, in its
	/// condition list.
	#[doc(alias("SetCondDuration"))]
	pub fn set_duration(
		self,
		condition: Condition,
		duration: ConditionDuration,
	) -> Result<bool, ConditionError> {
		if condition == Condition::CRITBOOSTED || !self.in_cond(condition)? {
			return Ok(false);
		}

		// SAFETY: As for `add`. The player has the condition, whose time the
		// native method writes, as the game's own condition code does.
		unsafe { raw::set_cond_duration(self.raw_player(), condition.0, duration.0) }?;
		Ok(true)
	}
}

thread_local! {
	/// The layout of the server class whose players' bits were last read.
	static LAYOUT: Cell<Option<ConditionLayout>> = const { Cell::new(None) };
}

/// A set of conditions, such as those a player has, which
/// [`PlayerConditions::bits`] reads.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConditionBits([u32; WORDS]);

impl ConditionBits {
	/// The debuffs that wear off faster while the player is cloaked
	/// (`g_aDebuffConditions`): burning, Jarate, bleeding, Mad Milk and gas.
	#[doc(alias("g_aDebuffConditions"))]
	pub const DEBUFFS: Self = Self::of(&[
		Condition::BURNING,
		Condition::URINE,
		Condition::BLEEDING,
		Condition::MAD_MILK,
		Condition::GAS,
	]);

	/// No conditions.
	pub const EMPTY: Self = Self([0; WORDS]);

	/// The set of `conditions`.
	pub const fn of(conditions: &[Condition]) -> Self {
		let mut bits = Self::EMPTY;
		let mut index = 0;

		while index < conditions.len() {
			bits.insert(conditions[index]);
			index += 1;
		}

		bits
	}

	/// The word and bit that hold `condition`, as `CConditionVars` places it.
	const fn position(condition: Condition) -> (usize, u32) {
		// A condition is validated to lie below `TF_COND_LAST`.
		let raw = condition.0 as usize;

		(raw / 32, 1 << (raw % 32))
	}

	/// The changes from `previous` to this set: the conditions added, and
	/// those removed.
	pub const fn changes_since(self, previous: Self) -> ConditionChanges {
		ConditionChanges {
			added: self.difference(previous),
			removed: previous.difference(self),
		}
	}

	/// Whether the set holds `condition`.
	pub const fn contains(self, condition: Condition) -> bool {
		let (word, bit) = Self::position(condition);

		self.0[word] & bit != 0
	}

	/// The conditions of this set that `other` lacks.
	pub const fn difference(self, other: Self) -> Self {
		let mut words = self.0;
		let mut word = 0;

		while word < WORDS {
			words[word] &= !other.0[word];
			word += 1;
		}

		Self(words)
	}

	/// Adds `condition`, and returns whether the set lacked it.
	pub const fn insert(&mut self, condition: Condition) -> bool {
		let (word, bit) = Self::position(condition);
		let lacked = self.0[word] & bit == 0;

		self.0[word] |= bit;
		lacked
	}

	/// The conditions both sets hold.
	pub const fn intersection(self, other: Self) -> Self {
		let mut words = self.0;
		let mut word = 0;

		while word < WORDS {
			words[word] &= other.0[word];
			word += 1;
		}

		Self(words)
	}

	/// Whether the set holds no conditions.
	pub const fn is_empty(self) -> bool {
		let mut word = 0;

		while word < WORDS {
			if self.0[word] != 0 {
				return false;
			}

			word += 1;
		}

		true
	}

	/// The conditions of the set, in the game's order.
	pub fn iter(self) -> impl Iterator<Item = Condition> {
		Condition::all().filter(move |&condition| self.contains(condition))
	}

	/// How many conditions the set holds.
	pub const fn len(self) -> usize {
		let mut count = 0;
		let mut word = 0;

		while word < WORDS {
			count += self.0[word].count_ones() as usize;
			word += 1;
		}

		count
	}

	/// Removes `condition`, and returns whether the set held it.
	pub const fn remove(&mut self, condition: Condition) -> bool {
		let (word, bit) = Self::position(condition);
		let held = self.0[word] & bit != 0;

		self.0[word] &= !bit;
		held
	}

	/// The conditions either set holds.
	pub const fn union(self, other: Self) -> Self {
		let mut words = self.0;
		let mut word = 0;

		while word < WORDS {
			words[word] |= other.0[word];
			word += 1;
		}

		Self(words)
	}
}

impl Extend<Condition> for ConditionBits {
	fn extend<I: IntoIterator<Item = Condition>>(&mut self, conditions: I) {
		for condition in conditions {
			self.insert(condition);
		}
	}
}

impl FromIterator<Condition> for ConditionBits {
	fn from_iter<I: IntoIterator<Item = Condition>>(conditions: I) -> Self {
		let mut bits = Self::EMPTY;

		bits.extend(conditions);
		bits
	}
}

/// The conditions added and removed between two reads of a player's
/// [`ConditionBits`], from [`ConditionBits::changes_since`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConditionChanges {
	/// The conditions the later read holds, and the earlier one lacks.
	pub added: ConditionBits,

	/// The conditions the earlier read holds, and the later one lacks.
	pub removed: ConditionBits,
}

impl ConditionChanges {
	/// Whether nothing changed.
	pub const fn is_empty(self) -> bool {
		self.added.is_empty() && self.removed.is_empty()
	}
}

/// Where the players of one server class keep their condition bits.
#[derive(Debug, Clone, Copy)]
struct ConditionLayout {
	/// The address of the server class the offsets were resolved in.
	class: usize,

	/// The offset of the condition list's `_condition_bits`, which hold the
	/// first word's conditions that the list keeps.
	list: usize,

	/// The offset of each word of [`WORD_NAMES`].
	words: [usize; WORDS],
}

impl ConditionLayout {
	/// The layout of `class`, resolved once for each class read in a row.
	fn of(server: Server<'_>, class: ServerClass<'_>) -> Result<Self, ConditionError> {
		if let Some(layout) = LAYOUT.get()
			&& layout.class == class.as_ptr().addr()
		{
			return Ok(layout);
		}

		let layout = Self::resolve(server, class)?;

		LAYOUT.set(Some(layout));
		Ok(layout)
	}

	/// Finds the variables in `class`'s send table.
	fn resolve(server: Server<'_>, class: ServerClass<'_>) -> Result<Self, ConditionError> {
		let dll = server
			.server_game_dll()
			.map_err(|_| ConditionError::UnsupportedLayout)?;

		let offset = |name: &CStr| match dll.net_prop(class, name) {
			Ok(variable) if variable.storage().is_compatible(Storage::U32) => Ok(variable.offset()),
			_ => Err(ConditionError::UnsupportedLayout),
		};

		let mut words = [0; WORDS];

		for (word, name) in words.iter_mut().zip(WORD_NAMES) {
			*word = offset(name)?;
		}

		Ok(Self {
			class: class.as_ptr().addr(),
			list: offset(c"_condition_bits")?,
			words,
		})
	}

	/// Reads the bits of `player`.
	///
	/// # Safety
	///
	/// `player` is live, and its server class is the one this layout was
	/// resolved in.
	unsafe fn read(self, player: Entity<'_>) -> ConditionBits {
		let read = |offset: usize| {
			// SAFETY: The offset is that of a 32-bit integer variable of the
			// player's class, as its send table declares, within the live player,
			// whose bytes are all initialized, since entities are zeroed when
			// allocated.
			unsafe {
				player
					.as_ptr()
					.cast::<u8>()
					.add(offset)
					.cast::<u32>()
					.read_unaligned()
			}
		};

		let mut words = self.words.map(read);

		// `CTFPlayerShared::InCond` checks the list's bits for the first word.
		words[0] |= read(self.list);
		ConditionBits(words)
	}
}

/// Reports how players' conditions change between reads, for a caller that
/// reads them regularly, such as every game frame: the added and removed
/// conditions SourceMod's `TF2_OnConditionAdded` and `TF2_OnConditionRemoved`
/// report.
///
/// A condition added and removed again between two reads goes unseen. Players
/// are told apart by handle, so one whose slot another player takes later is
/// not mistaken for them; [`Self::forget`] drops a player who left.
#[derive(Debug, Default, Clone)]
pub struct ConditionWatcher {
	previous: HashMap<EntityHandle, ConditionBits>,
}

impl ConditionWatcher {
	/// A watcher that has seen no players.
	pub fn new() -> Self {
		Self::default()
	}

	/// Forgets every player.
	pub fn clear(&mut self) {
		self.previous.clear();
	}

	/// Forgets `player`, as when they leave, so their next read reports every
	/// condition they have as added.
	pub fn forget(&mut self, player: Entity<'_>) {
		self.previous.remove(&player.handle());
	}

	/// Reads the player's conditions, and returns how they changed since the
	/// last read of this player, or since they had none, for a player read
	/// for the first time.
	pub fn update(
		&mut self,
		conditions: PlayerConditions<'_>,
	) -> Result<ConditionChanges, ConditionError> {
		let current = conditions.bits()?;
		let previous = self
			.previous
			.insert(conditions.player().handle(), current)
			.unwrap_or_default();

		Ok(current.changes_since(previous))
	}
}
