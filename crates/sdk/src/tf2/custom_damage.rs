//! TF2's custom damage kinds (`ETFDmgCustom`), which tell apart damage by
//! what dealt it, such as a headshot, afterburn or a taunt.
//!
//! [`CustomDamage`] names every kind TF2 numbers. Damage carries one as
//! [`DamageInfo::custom_kind`], a `tf_ragdoll` networks its death's as
//! `m_iDamageCustom`, and `player_death` events give it as `customkill`. The
//! game picks a kill's icon by its kind, and clients pick a ragdoll's effects
//! and death animation by it, which [`damage`](super::damage)'s
//! `CUSTOM_DAMAGE_*` constants describe for the kinds that change them. Some
//! of the game's checks read the kind too: a sapper takes any damage of
//! [`CustomDamage::WRENCH_FIX`], and a Spy's feigned death does not reduce a
//! [`CustomDamage::TELEFRAG`] (`tf_gamerules.cpp:7432`).
//!
//! [`DamageInfo::custom_kind`]: super::damage::DamageInfo::custom_kind

#[cfg(test)]
#[path = "../tests/tf2/custom_damage.rs"]
mod tests;

use std::fmt;

/// Defines [`CustomDamage`]'s kinds from the generated `ETFDmgCustom`
/// values, each with TF2's name for it, along with [`CustomDamage::ALL`] and
/// [`CustomDamage::name`].
macro_rules! kinds {
	($(
		$(#[doc = $doc:literal])*
		$kind:ident = $native:ident, $name:literal;
	)*) => {
		#[allow(
			clippy::unnecessary_cast,
			reason = "`ETFDmgCustom` is `c_int` on Windows but `c_uint` on Linux"
		)]
		impl CustomDamage {
			$(
				$(#[doc = $doc])*
				#[doc(alias = $name)]
				pub const $kind: Self = Self(sys::$native as i32);
			)*

			/// Every kind TF2 numbers, in its order, from [`Self::NONE`], 0.
			pub const ALL: [Self; [$($name),*].len()] = [$(Self::$kind),*];

			/// TF2's name for the kind, such as `TF_DMG_CUSTOM_HEADSHOT`, or
			/// `None` for a number it does not name.
			pub const fn name(self) -> Option<&'static str> {
				Some(match self {
					$(Self::$kind => $name,)*
					_ => return None,
				})
			}
		}

		/// The generated values' names, by kind, which the tests compare with
		/// the names the kinds give.
		#[cfg(test)]
		const NATIVE_NAMES: [&str; CustomDamage::ALL.len()] = [$(stringify!($native)),*];
	};
}

kinds! {
	/// No custom kind, as most damage has.
	NONE = ETFDmgCustom_TF_DMG_CUSTOM_NONE, "TF_DMG_CUSTOM_NONE";

	/// A headshot, as a sniper rifle's or the Ambassador's. Clients' death
	/// animation for it: [`CUSTOM_DAMAGE_HEADSHOT`].
	///
	/// [`CUSTOM_DAMAGE_HEADSHOT`]: super::damage::CUSTOM_DAMAGE_HEADSHOT
	HEADSHOT = ETFDmgCustom_TF_DMG_CUSTOM_HEADSHOT, "TF_DMG_CUSTOM_HEADSHOT";

	/// A Spy's backstab. Clients' death animation for it:
	/// [`CUSTOM_DAMAGE_BACKSTAB`].
	///
	/// [`CUSTOM_DAMAGE_BACKSTAB`]: super::damage::CUSTOM_DAMAGE_BACKSTAB
	BACKSTAB = ETFDmgCustom_TF_DMG_CUSTOM_BACKSTAB, "TF_DMG_CUSTOM_BACKSTAB";

	/// Afterburn: the damage a burning player takes over time.
	BURNING = ETFDmgCustom_TF_DMG_CUSTOM_BURNING, "TF_DMG_CUSTOM_BURNING";

	/// A wrench's hit on a sapper, which a sapper takes whatever the wrench's
	/// attributes (`tf_obj.cpp:2757`, `tf_obj_sapper.cpp:564`).
	WRENCH_FIX = ETFDmgCustom_TF_DMG_WRENCH_FIX, "TF_DMG_WRENCH_FIX";

	/// A minigun's bullets, which sentries resist more as they level up
	/// (`tf_obj_sentrygun.cpp:1959`).
	MINIGUN = ETFDmgCustom_TF_DMG_CUSTOM_MINIGUN, "TF_DMG_CUSTOM_MINIGUN";

	/// A suicide, as the `kill` and `explode` commands make.
	SUICIDE = ETFDmgCustom_TF_DMG_CUSTOM_SUICIDE, "TF_DMG_CUSTOM_SUICIDE";

	/// The Pyro's Hadouken taunt kill.
	TAUNT_HADOUKEN = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_HADOUKEN,
		"TF_DMG_CUSTOM_TAUNTATK_HADOUKEN";

	/// A flare gun's flare, and the afterburn it causes.
	BURNING_FLARE = ETFDmgCustom_TF_DMG_CUSTOM_BURNING_FLARE, "TF_DMG_CUSTOM_BURNING_FLARE";

	/// The Heavy's High Noon taunt kill.
	TAUNT_HIGH_NOON = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_HIGH_NOON,
		"TF_DMG_CUSTOM_TAUNTATK_HIGH_NOON";

	/// The Scout's Home Run taunt kill, with the Sandman.
	TAUNT_GRAND_SLAM = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_GRAND_SLAM,
		"TF_DMG_CUSTOM_TAUNTATK_GRAND_SLAM";

	/// A sniper rifle's shot, which passes through the shooter's teammates.
	PENETRATE_MY_TEAM = ETFDmgCustom_TF_DMG_CUSTOM_PENETRATE_MY_TEAM,
		"TF_DMG_CUSTOM_PENETRATE_MY_TEAM";

	/// A shot that passes through every player, as a fully charged Machina's,
	/// or one from a weapon with the `projectile_penetration` attribute.
	PENETRATE_ALL_PLAYERS = ETFDmgCustom_TF_DMG_CUSTOM_PENETRATE_ALL_PLAYERS,
		"TF_DMG_CUSTOM_PENETRATE_ALL_PLAYERS";

	/// The Spy's Fencing taunt kill.
	TAUNT_FENCING = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_FENCING, "TF_DMG_CUSTOM_TAUNTATK_FENCING";

	/// The Sydney Sleeper's shot, which passes through the shooter's
	/// teammates unless they burn, which it puts out.
	PENETRATE_NONBURNING_TEAMMATE = ETFDmgCustom_TF_DMG_CUSTOM_PENETRATE_NONBURNING_TEAMMATE,
		"TF_DMG_CUSTOM_PENETRATE_NONBURNING_TEAMMATE";

	/// The Sniper's Skewer taunt kill, with the Huntsman.
	TAUNT_ARROW_STAB = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_ARROW_STAB,
		"TF_DMG_CUSTOM_TAUNTATK_ARROW_STAB";

	/// A telefrag: a player killed where a teleporter, or the Monoculus'
	/// vortex, sends an enemy.
	TELEFRAG = ETFDmgCustom_TF_DMG_CUSTOM_TELEFRAG, "TF_DMG_CUSTOM_TELEFRAG";

	/// Afterburn from a Huntsman's burning arrow.
	BURNING_ARROW = ETFDmgCustom_TF_DMG_CUSTOM_BURNING_ARROW, "TF_DMG_CUSTOM_BURNING_ARROW";

	/// A burning arrow's hit, which sets its target alight.
	FLYING_BURN = ETFDmgCustom_TF_DMG_CUSTOM_FLYINGBURN, "TF_DMG_CUSTOM_FLYINGBURN";

	/// A pumpkin bomb's explosion.
	PUMPKIN_BOMB = ETFDmgCustom_TF_DMG_CUSTOM_PUMPKIN_BOMB, "TF_DMG_CUSTOM_PUMPKIN_BOMB";

	/// A decapitating sword's kill, such as the Eyelander's. Clients'
	/// effects on the ragdoll: [`CUSTOM_DAMAGE_DECAPITATION`].
	///
	/// [`CUSTOM_DAMAGE_DECAPITATION`]: super::damage::CUSTOM_DAMAGE_DECAPITATION
	DECAPITATION = ETFDmgCustom_TF_DMG_CUSTOM_DECAPITATION, "TF_DMG_CUSTOM_DECAPITATION";

	/// The Soldier's Kamikaze taunt kill, with the Equalizer.
	TAUNT_GRENADE = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_GRENADE, "TF_DMG_CUSTOM_TAUNTATK_GRENADE";

	/// A ball from the Sandman or the Wrap Assassin.
	BASEBALL = ETFDmgCustom_TF_DMG_CUSTOM_BASEBALL, "TF_DMG_CUSTOM_BASEBALL";

	/// A Demoman's shield bash at the end of a charge.
	CHARGE_IMPACT = ETFDmgCustom_TF_DMG_CUSTOM_CHARGE_IMPACT, "TF_DMG_CUSTOM_CHARGE_IMPACT";

	/// The Demoman's Barbarian Swing taunt kill. Clients' effects on the
	/// ragdoll: [`CUSTOM_DAMAGE_TAUNT_BARBARIAN_SWING`].
	///
	/// [`CUSTOM_DAMAGE_TAUNT_BARBARIAN_SWING`]: super::damage::CUSTOM_DAMAGE_TAUNT_BARBARIAN_SWING
	TAUNT_BARBARIAN_SWING = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_BARBARIAN_SWING,
		"TF_DMG_CUSTOM_TAUNTATK_BARBARIAN_SWING";

	/// A sticky bomb detonated before it touched anything.
	AIR_STICKY_BURST = ETFDmgCustom_TF_DMG_CUSTOM_AIR_STICKY_BURST,
		"TF_DMG_CUSTOM_AIR_STICKY_BURST";

	/// A sticky bomb the game marks as defensive (`m_bDefensiveBomb`), once
	/// it has touched something.
	DEFENSIVE_STICKY = ETFDmgCustom_TF_DMG_CUSTOM_DEFENSIVE_STICKY,
		"TF_DMG_CUSTOM_DEFENSIVE_STICKY";

	/// A hit by a shovel that speeds up or hits harder at low health, as the
	/// Escape Plan and the Equalizer do.
	PICKAXE = ETFDmgCustom_TF_DMG_CUSTOM_PICKAXE, "TF_DMG_CUSTOM_PICKAXE";

	/// A rocket from the Direct Hit.
	ROCKET_DIRECT_HIT = ETFDmgCustom_TF_DMG_CUSTOM_ROCKET_DIRECTHIT,
		"TF_DMG_CUSTOM_ROCKET_DIRECTHIT";

	/// The Medic's Spinal Tap taunt kill, with the Ubersaw.
	TAUNT_UBERSLICE = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_UBERSLICE,
		"TF_DMG_CUSTOM_TAUNTATK_UBERSLICE";

	/// The bullets of a sentry its builder aims with the Wrangler.
	PLAYER_SENTRY = ETFDmgCustom_TF_DMG_CUSTOM_PLAYER_SENTRY, "TF_DMG_CUSTOM_PLAYER_SENTRY";

	/// A sticky bomb detonated after it touched something.
	STANDARD_STICKY = ETFDmgCustom_TF_DMG_CUSTOM_STANDARD_STICKY, "TF_DMG_CUSTOM_STANDARD_STICKY";

	/// A shot with a revenge critical hit, from the Frontier Justice or the
	/// Manmelter.
	SHOTGUN_REVENGE_CRIT = ETFDmgCustom_TF_DMG_CUSTOM_SHOTGUN_REVENGE_CRIT,
		"TF_DMG_CUSTOM_SHOTGUN_REVENGE_CRIT";

	/// The Engineer's Dischord taunt kill, with the Frontier Justice.
	TAUNT_ENGINEER_GUITAR_SMASH = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_ENGINEER_GUITAR_SMASH,
		"TF_DMG_CUSTOM_TAUNTATK_ENGINEER_GUITAR_SMASH";

	/// Bleeding: the damage a bleeding player takes over time.
	BLEEDING = ETFDmgCustom_TF_DMG_CUSTOM_BLEEDING, "TF_DMG_CUSTOM_BLEEDING";

	/// A Golden Wrench's kill. Clients turn the ragdoll of a player it kills
	/// to gold (`c_tf_player.cpp:730`).
	GOLD_WRENCH = ETFDmgCustom_TF_DMG_CUSTOM_GOLD_WRENCH, "TF_DMG_CUSTOM_GOLD_WRENCH";

	/// The building an Engineer carries, destroyed as they die.
	CARRIED_BUILDING = ETFDmgCustom_TF_DMG_CUSTOM_CARRIED_BUILDING,
		"TF_DMG_CUSTOM_CARRIED_BUILDING";

	/// The Gunslinger's third hit in a row, a critical hit.
	COMBO_PUNCH = ETFDmgCustom_TF_DMG_CUSTOM_COMBO_PUNCH, "TF_DMG_CUSTOM_COMBO_PUNCH";

	/// The Engineer's Organ Grinder taunt kill, with the Gunslinger.
	TAUNT_ENGINEER_ARM_KILL = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_ENGINEER_ARM_KILL,
		"TF_DMG_CUSTOM_TAUNTATK_ENGINEER_ARM_KILL";

	/// A killing hit with a fish, such as the Holy Mackerel.
	FISH_KILL = ETFDmgCustom_TF_DMG_CUSTOM_FISH_KILL, "TF_DMG_CUSTOM_FISH_KILL";

	/// A map's `trigger_hurt`.
	TRIGGER_HURT = ETFDmgCustom_TF_DMG_CUSTOM_TRIGGER_HURT, "TF_DMG_CUSTOM_TRIGGER_HURT";

	/// A Halloween boss's decapitating swing, such as the Horseless Headless
	/// Horsemann's. Clients' effects on the ragdoll:
	/// [`CUSTOM_DAMAGE_DECAPITATION_BOSS`].
	///
	/// [`CUSTOM_DAMAGE_DECAPITATION_BOSS`]: super::damage::CUSTOM_DAMAGE_DECAPITATION_BOSS
	DECAPITATION_BOSS = ETFDmgCustom_TF_DMG_CUSTOM_DECAPITATION_BOSS,
		"TF_DMG_CUSTOM_DECAPITATION_BOSS";

	/// The Ullapool Caber's explosion.
	STICKBOMB_EXPLOSION = ETFDmgCustom_TF_DMG_CUSTOM_STICKBOMB_EXPLOSION,
		"TF_DMG_CUSTOM_STICKBOMB_EXPLOSION";

	/// A kind TF2 declares, which the public TF2 source gives to no damage.
	AEGIS_ROUND = ETFDmgCustom_TF_DMG_CUSTOM_AEGIS_ROUND, "TF_DMG_CUSTOM_AEGIS_ROUND";

	/// The explosion of a flare from the Detonator.
	FLARE_EXPLOSION = ETFDmgCustom_TF_DMG_CUSTOM_FLARE_EXPLOSION, "TF_DMG_CUSTOM_FLARE_EXPLOSION";

	/// A stomp, by the Mantreads or a Thermal Thruster's landing.
	BOOTS_STOMP = ETFDmgCustom_TF_DMG_CUSTOM_BOOTS_STOMP, "TF_DMG_CUSTOM_BOOTS_STOMP";

	/// Plasma, as an uncharged Cow Mangler 5000 shot. Clients' effects on the
	/// ragdoll: [`CUSTOM_DAMAGE_PLASMA`].
	///
	/// [`CUSTOM_DAMAGE_PLASMA`]: super::damage::CUSTOM_DAMAGE_PLASMA
	PLASMA = ETFDmgCustom_TF_DMG_CUSTOM_PLASMA, "TF_DMG_CUSTOM_PLASMA";

	/// A charged Cow Mangler 5000 shot. Clients' effects on the ragdoll:
	/// [`CUSTOM_DAMAGE_PLASMA_CHARGED`].
	///
	/// [`CUSTOM_DAMAGE_PLASMA_CHARGED`]: super::damage::CUSTOM_DAMAGE_PLASMA_CHARGED
	PLASMA_CHARGED = ETFDmgCustom_TF_DMG_CUSTOM_PLASMA_CHARGED, "TF_DMG_CUSTOM_PLASMA_CHARGED";

	/// A kind TF2 declares, which the public TF2 source gives to no damage.
	PLASMA_GIB = ETFDmgCustom_TF_DMG_CUSTOM_PLASMA_GIB, "TF_DMG_CUSTOM_PLASMA_GIB";

	/// A sticky bomb from the Sticky Jumper.
	PRACTICE_STICKY = ETFDmgCustom_TF_DMG_CUSTOM_PRACTICE_STICKY, "TF_DMG_CUSTOM_PRACTICE_STICKY";

	/// A rocket of the Monoculus.
	EYEBALL_ROCKET = ETFDmgCustom_TF_DMG_CUSTOM_EYEBALL_ROCKET, "TF_DMG_CUSTOM_EYEBALL_ROCKET";

	/// A critical headshot by a weapon with the `decapitate_type` attribute.
	/// Clients' effects on the ragdoll:
	/// [`CUSTOM_DAMAGE_HEADSHOT_DECAPITATION`].
	///
	/// [`CUSTOM_DAMAGE_HEADSHOT_DECAPITATION`]: super::damage::CUSTOM_DAMAGE_HEADSHOT_DECAPITATION
	HEADSHOT_DECAPITATION = ETFDmgCustom_TF_DMG_CUSTOM_HEADSHOT_DECAPITATION,
		"TF_DMG_CUSTOM_HEADSHOT_DECAPITATION";

	/// The Pyro's Armageddon taunt kill, with the Rainblower.
	TAUNT_ARMAGEDDON = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_ARMAGEDDON,
		"TF_DMG_CUSTOM_TAUNTATK_ARMAGEDDON";

	/// A flare a Pyro fires while taunting, as with the Scorch Shot.
	FLARE_PELLET = ETFDmgCustom_TF_DMG_CUSTOM_FLARE_PELLET, "TF_DMG_CUSTOM_FLARE_PELLET";

	/// The Flying Guillotine.
	CLEAVER = ETFDmgCustom_TF_DMG_CUSTOM_CLEAVER, "TF_DMG_CUSTOM_CLEAVER";

	/// A kind TF2 declares, which the public TF2 source gives to no damage.
	CLEAVER_CRIT = ETFDmgCustom_TF_DMG_CUSTOM_CLEAVER_CRIT, "TF_DMG_CUSTOM_CLEAVER_CRIT";

	/// A building the Red-Tape Recorder sapper takes apart to nothing.
	SAPPER_RECORDER_DEATH = ETFDmgCustom_TF_DMG_CUSTOM_SAPPER_RECORDER_DEATH,
		"TF_DMG_CUSTOM_SAPPER_RECORDER_DEATH";

	/// The bomb Merasmus makes of a player (the `merasmus_player_bomb` kill
	/// icon).
	MERASMUS_PLAYER_BOMB = ETFDmgCustom_TF_DMG_CUSTOM_MERASMUS_PLAYER_BOMB,
		"TF_DMG_CUSTOM_MERASMUS_PLAYER_BOMB";

	/// Merasmus' bombs (the `merasmus_grenade` kill icon).
	MERASMUS_GRENADE = ETFDmgCustom_TF_DMG_CUSTOM_MERASMUS_GRENADE,
		"TF_DMG_CUSTOM_MERASMUS_GRENADE";

	/// Merasmus' zap (the `merasmus_zap` kill icon).
	MERASMUS_ZAP = ETFDmgCustom_TF_DMG_CUSTOM_MERASMUS_ZAP, "TF_DMG_CUSTOM_MERASMUS_ZAP";

	/// Merasmus' staff. Clients' effects on the ragdoll:
	/// [`CUSTOM_DAMAGE_MERASMUS_DECAPITATION`].
	///
	/// [`CUSTOM_DAMAGE_MERASMUS_DECAPITATION`]: super::damage::CUSTOM_DAMAGE_MERASMUS_DECAPITATION
	MERASMUS_DECAPITATION = ETFDmgCustom_TF_DMG_CUSTOM_MERASMUS_DECAPITATION,
		"TF_DMG_CUSTOM_MERASMUS_DECAPITATION";

	/// A cannonball from the Loose Cannon hitting a player.
	CANNONBALL_PUSH = ETFDmgCustom_TF_DMG_CUSTOM_CANNONBALL_PUSH, "TF_DMG_CUSTOM_CANNONBALL_PUSH";

	/// An all-class guitar taunt's kill (`TAUNTATK_ALLCLASS_GUITAR_RIFF`).
	TAUNT_ALLCLASS_GUITAR_RIFF = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_ALLCLASS_GUITAR_RIFF,
		"TF_DMG_CUSTOM_TAUNTATK_ALLCLASS_GUITAR_RIFF";

	/// A thrown item's hit, such as a water balloon's.
	THROWABLE = ETFDmgCustom_TF_DMG_CUSTOM_THROWABLE, "TF_DMG_CUSTOM_THROWABLE";

	/// A thrown item's kill.
	THROWABLE_KILL = ETFDmgCustom_TF_DMG_CUSTOM_THROWABLE_KILL, "TF_DMG_CUSTOM_THROWABLE_KILL";

	/// The Shadow Leap spell's teleport.
	SPELL_TELEPORT = ETFDmgCustom_TF_DMG_CUSTOM_SPELL_TELEPORT, "TF_DMG_CUSTOM_SPELL_TELEPORT";

	/// The skeletons of the Skeleton Horde spell.
	SPELL_SKELETON = ETFDmgCustom_TF_DMG_CUSTOM_SPELL_SKELETON, "TF_DMG_CUSTOM_SPELL_SKELETON";

	/// The pumpkins of the Pumpkin MIRV spell.
	SPELL_MIRV = ETFDmgCustom_TF_DMG_CUSTOM_SPELL_MIRV, "TF_DMG_CUSTOM_SPELL_MIRV";

	/// The Meteor Shower spell.
	SPELL_METEOR = ETFDmgCustom_TF_DMG_CUSTOM_SPELL_METEOR, "TF_DMG_CUSTOM_SPELL_METEOR";

	/// The Ball O' Lightning spell.
	SPELL_LIGHTNING = ETFDmgCustom_TF_DMG_CUSTOM_SPELL_LIGHTNING, "TF_DMG_CUSTOM_SPELL_LIGHTNING";

	/// The Fireball spell.
	SPELL_FIREBALL = ETFDmgCustom_TF_DMG_CUSTOM_SPELL_FIREBALL, "TF_DMG_CUSTOM_SPELL_FIREBALL";

	/// The Monoculus the MONOCULUS! spell summons.
	SPELL_MONOCULUS = ETFDmgCustom_TF_DMG_CUSTOM_SPELL_MONOCULUS, "TF_DMG_CUSTOM_SPELL_MONOCULUS";

	/// The Blast Jump spell.
	SPELL_BLAST_JUMP = ETFDmgCustom_TF_DMG_CUSTOM_SPELL_BLASTJUMP, "TF_DMG_CUSTOM_SPELL_BLASTJUMP";

	/// The Swarm of Bats spell.
	SPELL_BATS = ETFDmgCustom_TF_DMG_CUSTOM_SPELL_BATS, "TF_DMG_CUSTOM_SPELL_BATS";

	/// The Minify spell.
	SPELL_TINY = ETFDmgCustom_TF_DMG_CUSTOM_SPELL_TINY, "TF_DMG_CUSTOM_SPELL_TINY";

	/// A Halloween bumper car's hit.
	KART = ETFDmgCustom_TF_DMG_CUSTOM_KART, "TF_DMG_CUSTOM_KART";

	/// A melee hit in the Doomsday Halloween scenario, which the game makes
	/// critical.
	GIANT_HAMMER = ETFDmgCustom_TF_DMG_CUSTOM_GIANT_HAMMER, "TF_DMG_CUSTOM_GIANT_HAMMER";

	/// Damage Mannpower's Reflect powerup returns to the attacker.
	RUNE_REFLECT = ETFDmgCustom_TF_DMG_CUSTOM_RUNE_REFLECT, "TF_DMG_CUSTOM_RUNE_REFLECT";

	/// The Dragon's Fury's fireball.
	DRAGONS_FURY_IGNITE = ETFDmgCustom_TF_DMG_CUSTOM_DRAGONS_FURY_IGNITE,
		"TF_DMG_CUSTOM_DRAGONS_FURY_IGNITE";

	/// The Dragon's Fury's fireball on a burning player, which deals bonus
	/// damage.
	DRAGONS_FURY_BONUS_BURNING = ETFDmgCustom_TF_DMG_CUSTOM_DRAGONS_FURY_BONUS_BURNING,
		"TF_DMG_CUSTOM_DRAGONS_FURY_BONUS_BURNING";

	/// A killing slap with the Hot Hand.
	SLAP_KILL = ETFDmgCustom_TF_DMG_CUSTOM_SLAP_KILL, "TF_DMG_CUSTOM_SLAP_KILL";

	/// A map's crocodiles, sharks or piranhas (`func_croc`).
	CROC = ETFDmgCustom_TF_DMG_CUSTOM_CROC, "TF_DMG_CUSTOM_CROC";

	/// The Pyro's gas blast taunt kill (`TAUNTATK_PYRO_GASBLAST`).
	TAUNT_GAS_BLAST = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_GASBLAST,
		"TF_DMG_CUSTOM_TAUNTATK_GASBLAST";

	/// An Axtinguisher's hit on a burning player, which puts them out for
	/// bonus damage.
	AXTINGUISHER_BOOSTED = ETFDmgCustom_TF_DMG_CUSTOM_AXTINGUISHER_BOOSTED,
		"TF_DMG_CUSTOM_AXTINGUISHER_BOOSTED";

	/// Krampus' melee attack, on `koth_krampus`.
	KRAMPUS_MELEE = ETFDmgCustom_TF_DMG_CUSTOM_KRAMPUS_MELEE, "TF_DMG_CUSTOM_KRAMPUS_MELEE";

	/// Krampus' rockets, on `koth_krampus`.
	KRAMPUS_RANGED = ETFDmgCustom_TF_DMG_CUSTOM_KRAMPUS_RANGED, "TF_DMG_CUSTOM_KRAMPUS_RANGED";

	/// The Engineer's trick shot taunt kill (`TAUNTATK_ENGINEER_TRICKSHOT`).
	TAUNT_TRICKSHOT = ETFDmgCustom_TF_DMG_CUSTOM_TAUNTATK_TRICKSHOT,
		"TF_DMG_CUSTOM_TAUNTATK_TRICKSHOT";
}

/// One of TF2's custom damage kinds (`ETFDmgCustom`), as the
/// [module documentation](self) describes.
///
/// Any number is a kind, since the game keeps whatever number damage
/// carries, and its updates add kinds: [`Self::name`] tells TF2's own apart.
#[doc(alias("ETFDmgCustom", "m_iDamageCustom", "customkill"))]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CustomDamage(i32);

impl CustomDamage {
	/// The kind TF2 numbers `raw`, whether or not it names it.
	pub const fn from_raw(raw: i32) -> Self {
		Self(raw)
	}

	/// TF2's number for the kind.
	pub const fn to_raw(self) -> i32 {
		self.0
	}
}

impl fmt::Debug for CustomDamage {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self.name() {
			Some(name) => f.write_str(name),
			None => f.debug_tuple("CustomDamage").field(&self.0).finish(),
		}
	}
}
