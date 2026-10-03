//! Gameplay attributes vetted against the item schema TF2 ships, for the safe
//! setters of [`ItemAttributes`](super::ItemAttributes).
//!
//! Each definition was checked against `scripts/items/items_game.txt`: TF2's
//! legacy default type (no `attribute_type`), stored as a float (no
//! `stored_as_integer`), not hidden, and read by a gameplay hook on the
//! server. Attributes whose values index game tables (particles, killstreak
//! effects, item definitions, lookup tables, condition bit masks) are left
//! out, as are string and other blob types.
//!
//! Bounds are conservative policy, chosen for the attribute class and
//! description format the shipped schema gives each attribute, which
//! [`trust_shipped_schema`](super::trust_shipped_schema) has its caller vouch
//! the running schema keeps. A "bonus" or "increased" multiplier only raises
//! the hooked value, a "penalty", "decreased" or "reduced" one only lowers
//! it, matching how the game describes them to clients. Timing and
//! ammunition multipliers stay above zero, and amounts within a few hundred
//! points. Bounds apply to one item: the game combines the values of all the
//! items providing to a player.
//!
//! The game decides some effects only at certain moments. Movement speed
//! applies after the owner switches weapons or gains or loses a condition,
//! a new maximum health does not change current health, and clip and ammo
//! sizes apply to later reloads and pickups. [`PROVIDE_ON_ACTIVE`] takes
//! effect after [`ItemAttributes::reapply_provision`].
//!
//! [`ItemAttributes::reapply_provision`]: super::ItemAttributes::reapply_provision

use crate::tf2::attributes::{Amount, AttributeDef, DescriptionFormat, Flag, Multiplier, Seconds};

/// `airblast disabled` (356, `airblast_disabled`): the flame thrower cannot
/// airblast.
#[doc(alias("airblast disabled"))]
pub const AIRBLAST_DISABLED: AttributeDef<Flag> = AttributeDef::new(
	356,
	c"airblast disabled",
	c"airblast_disabled",
	DescriptionFormat::Additive,
	0.0,
	1.0,
);

/// `Blast radius decreased` (100, `mult_explosion_radius`): scales the
/// explosion radius of the weapon's projectiles, from 0.1 to 1.
#[doc(alias("Blast radius decreased"))]
pub const BLAST_RADIUS_DECREASED: AttributeDef<Multiplier> = AttributeDef::new(
	100,
	c"Blast radius decreased",
	c"mult_explosion_radius",
	DescriptionFormat::Percentage,
	0.1,
	1.0,
);

/// `Blast radius increased` (99, `mult_explosion_radius`): scales the
/// explosion radius of the weapon's projectiles, from 1 to 4.
#[doc(alias("Blast radius increased"))]
pub const BLAST_RADIUS_INCREASED: AttributeDef<Multiplier> = AttributeDef::new(
	99,
	c"Blast radius increased",
	c"mult_explosion_radius",
	DescriptionFormat::Percentage,
	1.0,
	4.0,
);

/// `bullets per shot bonus` (45, `mult_bullets_per_shot`): scales the
/// bullets or pellets each shot fires, from 1 to 5.
#[doc(alias("bullets per shot bonus"))]
pub const BULLETS_PER_SHOT_BONUS: AttributeDef<Multiplier> = AttributeDef::new(
	45,
	c"bullets per shot bonus",
	c"mult_bullets_per_shot",
	DescriptionFormat::Percentage,
	1.0,
	5.0,
);

/// `clip size bonus` (4, `mult_clipsize`): scales the clip size, from 1 to
/// 10. The current clip keeps its ammo until the next reload.
#[doc(alias("clip size bonus"))]
pub const CLIP_SIZE_BONUS: AttributeDef<Multiplier> = AttributeDef::new(
	4,
	c"clip size bonus",
	c"mult_clipsize",
	DescriptionFormat::Percentage,
	1.0,
	10.0,
);

/// `clip size penalty` (3, `mult_clipsize`): scales the clip size, from 0.1
/// to 1.
#[doc(alias("clip size penalty"))]
pub const CLIP_SIZE_PENALTY: AttributeDef<Multiplier> = AttributeDef::new(
	3,
	c"clip size penalty",
	c"mult_clipsize",
	DescriptionFormat::Percentage,
	0.1,
	1.0,
);

/// `crit kill will gib` (309, `crit_kill_will_gib`): critical kills always
/// gib the victim.
#[doc(alias("crit kill will gib"))]
pub const CRIT_KILL_WILL_GIB: AttributeDef<Flag> = AttributeDef::new(
	309,
	c"crit kill will gib",
	c"crit_kill_will_gib",
	DescriptionFormat::Additive,
	0.0,
	1.0,
);

/// `critboost on kill` (31, `add_onkill_critboost_time`): seconds of
/// critical boost after each kill with the weapon, up to 60. The game rounds
/// them to whole seconds.
#[doc(alias("critboost on kill"))]
pub const CRITBOOST_ON_KILL: AttributeDef<Seconds> = AttributeDef::new(
	31,
	c"critboost on kill",
	c"add_onkill_critboost_time",
	DescriptionFormat::Additive,
	0.0,
	60.0,
);

/// `damage bonus` (2, `mult_dmg`): scales the weapon's damage, from 1 to 10.
#[doc(alias("damage bonus"))]
pub const DAMAGE_BONUS: AttributeDef<Multiplier> = AttributeDef::new(
	2,
	c"damage bonus",
	c"mult_dmg",
	DescriptionFormat::Percentage,
	1.0,
	10.0,
);

/// `damage penalty` (1, `mult_dmg`): scales the weapon's damage, from 0 to 1.
#[doc(alias("damage penalty"))]
pub const DAMAGE_PENALTY: AttributeDef<Multiplier> = AttributeDef::new(
	1,
	c"damage penalty",
	c"mult_dmg",
	DescriptionFormat::Percentage,
	0.0,
	1.0,
);

/// `dmg penalty vs players` (138, `mult_dmg_vs_players`): scales the
/// weapon's damage to players, from 0 to 1.
#[doc(alias("dmg penalty vs players"))]
pub const DAMAGE_PENALTY_VS_PLAYERS: AttributeDef<Multiplier> = AttributeDef::new(
	138,
	c"dmg penalty vs players",
	c"mult_dmg_vs_players",
	DescriptionFormat::Percentage,
	0.0,
	1.0,
);

/// `dmg taken from blast reduced` (64, `mult_dmgtaken_from_explosions`):
/// scales the blast damage the owner takes, from 0 to 1.
#[doc(alias("dmg taken from blast reduced"))]
pub const DAMAGE_TAKEN_FROM_BLAST_REDUCED: AttributeDef<Multiplier> = AttributeDef::new(
	64,
	c"dmg taken from blast reduced",
	c"mult_dmgtaken_from_explosions",
	DescriptionFormat::InvertedPercentage,
	0.0,
	1.0,
);

/// `dmg taken increased` (412, `mult_dmgtaken`): scales the damage the owner
/// takes, from 1 to 10.
#[doc(alias("dmg taken increased"))]
pub const DAMAGE_TAKEN_INCREASED: AttributeDef<Multiplier> = AttributeDef::new(
	412,
	c"dmg taken increased",
	c"mult_dmgtaken",
	DescriptionFormat::Percentage,
	1.0,
	10.0,
);

/// `deploy time decreased` (178, `mult_deploy_time`): scales the time to
/// switch to the weapon, from 0.1 to 1.
#[doc(alias("deploy time decreased"))]
pub const DEPLOY_TIME_DECREASED: AttributeDef<Multiplier> = AttributeDef::new(
	178,
	c"deploy time decreased",
	c"mult_deploy_time",
	DescriptionFormat::InvertedPercentage,
	0.1,
	1.0,
);

/// `deploy time increased` (177, `mult_deploy_time`): scales the time to
/// switch to the weapon, from 1 to 10.
#[doc(alias("deploy time increased"))]
pub const DEPLOY_TIME_INCREASED: AttributeDef<Multiplier> = AttributeDef::new(
	177,
	c"deploy time increased",
	c"mult_deploy_time",
	DescriptionFormat::Percentage,
	1.0,
	10.0,
);

/// `faster reload rate` (318, `fast_reload`): scales the reload time, from
/// 0.1 to 1.
#[doc(alias("faster reload rate"))]
pub const FASTER_RELOAD_RATE: AttributeDef<Multiplier> = AttributeDef::new(
	318,
	c"faster reload rate",
	c"fast_reload",
	DescriptionFormat::InvertedPercentage,
	0.1,
	1.0,
);

/// `fire rate bonus` (6, `mult_postfiredelay`): scales the delay between
/// shots, from 0.1 to 1.
#[doc(alias("fire rate bonus"))]
pub const FIRE_RATE_BONUS: AttributeDef<Multiplier> = AttributeDef::new(
	6,
	c"fire rate bonus",
	c"mult_postfiredelay",
	DescriptionFormat::InvertedPercentage,
	0.1,
	1.0,
);

/// `fire rate penalty` (5, `mult_postfiredelay`): scales the delay between
/// shots, from 1 to 10.
#[doc(alias("fire rate penalty"))]
pub const FIRE_RATE_PENALTY: AttributeDef<Multiplier> = AttributeDef::new(
	5,
	c"fire rate penalty",
	c"mult_postfiredelay",
	DescriptionFormat::InvertedPercentage,
	1.0,
	10.0,
);

/// `heal on hit for rapidfire` (16, `add_onhit_addhealth`): health the owner
/// gains per hit, up to 500.
#[doc(alias("heal on hit for rapidfire"))]
pub const HEAL_ON_HIT_RAPID_FIRE: AttributeDef<Amount> = AttributeDef::new(
	16,
	c"heal on hit for rapidfire",
	c"add_onhit_addhealth",
	DescriptionFormat::Additive,
	0.0,
	500.0,
);

/// `heal on hit for slowfire` (110, `add_onhit_addhealth`): health the owner
/// gains per hit, up to 500.
#[doc(alias("heal on hit for slowfire"))]
pub const HEAL_ON_HIT_SLOW_FIRE: AttributeDef<Amount> = AttributeDef::new(
	110,
	c"heal on hit for slowfire",
	c"add_onhit_addhealth",
	DescriptionFormat::Additive,
	0.0,
	500.0,
);

/// `heal on kill` (180, `heal_on_kill`): health the owner gains per kill with
/// the weapon, up to 500.
#[doc(alias("heal on kill"))]
pub const HEAL_ON_KILL: AttributeDef<Amount> = AttributeDef::new(
	180,
	c"heal on kill",
	c"heal_on_kill",
	DescriptionFormat::Additive,
	0.0,
	500.0,
);

/// `health regen` (57, `add_health_regen`): health the owner regenerates per
/// second, up to 100.
#[doc(alias("health regen"))]
pub const HEALTH_REGEN: AttributeDef<Amount> = AttributeDef::new(
	57,
	c"health regen",
	c"add_health_regen",
	DescriptionFormat::Additive,
	0.0,
	100.0,
);

/// `max health additive bonus` (26, `add_maxhealth`): health added to the
/// owner's maximum, up to 1000. Current health does not change.
#[doc(alias("max health additive bonus"))]
pub const MAX_HEALTH_ADDITIVE_BONUS: AttributeDef<Amount> = AttributeDef::new(
	26,
	c"max health additive bonus",
	c"add_maxhealth",
	DescriptionFormat::Additive,
	0.0,
	1000.0,
);

/// `max health additive penalty` (125, `add_maxhealth`): health removed from
/// the owner's maximum, as a negative amount down to -100, which on one item
/// leaves every class at least 25. The game adds up `add_maxhealth` from all
/// of the owner's items, so penalties on several items stack below that.
#[doc(alias("max health additive penalty"))]
pub const MAX_HEALTH_ADDITIVE_PENALTY: AttributeDef<Amount> = AttributeDef::new(
	125,
	c"max health additive penalty",
	c"add_maxhealth",
	DescriptionFormat::Additive,
	-100.0,
	0.0,
);

/// `maxammo primary increased` (76, `mult_maxammo_primary`): scales the
/// owner's primary ammo capacity, from 1 to 10.
#[doc(alias("maxammo primary increased"))]
pub const MAXAMMO_PRIMARY_INCREASED: AttributeDef<Multiplier> = AttributeDef::new(
	76,
	c"maxammo primary increased",
	c"mult_maxammo_primary",
	DescriptionFormat::Percentage,
	1.0,
	10.0,
);

/// `maxammo primary reduced` (77, `mult_maxammo_primary`): scales the
/// owner's primary ammo capacity, from 0.1 to 1.
#[doc(alias("maxammo primary reduced"))]
pub const MAXAMMO_PRIMARY_REDUCED: AttributeDef<Multiplier> = AttributeDef::new(
	77,
	c"maxammo primary reduced",
	c"mult_maxammo_primary",
	DescriptionFormat::Percentage,
	0.1,
	1.0,
);

/// `maxammo secondary increased` (78, `mult_maxammo_secondary`): scales the
/// owner's secondary ammo capacity, from 1 to 10.
#[doc(alias("maxammo secondary increased"))]
pub const MAXAMMO_SECONDARY_INCREASED: AttributeDef<Multiplier> = AttributeDef::new(
	78,
	c"maxammo secondary increased",
	c"mult_maxammo_secondary",
	DescriptionFormat::Percentage,
	1.0,
	10.0,
);

/// `maxammo secondary reduced` (79, `mult_maxammo_secondary`): scales the
/// owner's secondary ammo capacity, from 0.1 to 1.
#[doc(alias("maxammo secondary reduced"))]
pub const MAXAMMO_SECONDARY_REDUCED: AttributeDef<Multiplier> = AttributeDef::new(
	79,
	c"maxammo secondary reduced",
	c"mult_maxammo_secondary",
	DescriptionFormat::Percentage,
	0.1,
	1.0,
);

/// `minicrit vs burning player` (209, `or_minicrit_vs_playercond_burning`):
/// hits on burning players are mini-crits.
#[doc(alias("minicrit vs burning player"))]
pub const MINICRIT_VS_BURNING_PLAYER: AttributeDef<Flag> = AttributeDef::new(
	209,
	c"minicrit vs burning player",
	c"or_minicrit_vs_playercond_burning",
	DescriptionFormat::Additive,
	0.0,
	1.0,
);

/// `minicrits become crits` (179, `minicrits_become_crits`): the weapon's
/// mini-crits are full critical hits.
#[doc(alias("minicrits become crits"))]
pub const MINICRITS_BECOME_CRITS: AttributeDef<Flag> = AttributeDef::new(
	179,
	c"minicrits become crits",
	c"minicrits_become_crits",
	DescriptionFormat::Additive,
	0.0,
	1.0,
);

/// `move speed bonus` (107, `mult_player_movespeed`): scales the owner's
/// movement speed, from 1 to 3. It applies when the game next recomputes the
/// speed, such as on a weapon switch.
#[doc(alias("move speed bonus"))]
pub const MOVE_SPEED_BONUS: AttributeDef<Multiplier> = AttributeDef::new(
	107,
	c"move speed bonus",
	c"mult_player_movespeed",
	DescriptionFormat::Percentage,
	1.0,
	3.0,
);

/// `move speed penalty` (54, `mult_player_movespeed`): scales the owner's
/// movement speed, from 0.1 to 1. It applies when the game next recomputes
/// the speed, such as on a weapon switch.
#[doc(alias("move speed penalty"))]
pub const MOVE_SPEED_PENALTY: AttributeDef<Multiplier> = AttributeDef::new(
	54,
	c"move speed penalty",
	c"mult_player_movespeed",
	DescriptionFormat::InvertedPercentage,
	0.1,
	1.0,
);

/// `Projectile speed decreased` (104, `mult_projectile_speed`): scales the
/// launch speed of the weapon's projectiles, from 0.1 to 1.
#[doc(alias("Projectile speed decreased"))]
pub const PROJECTILE_SPEED_DECREASED: AttributeDef<Multiplier> = AttributeDef::new(
	104,
	c"Projectile speed decreased",
	c"mult_projectile_speed",
	DescriptionFormat::Percentage,
	0.1,
	1.0,
);

/// `Projectile speed increased` (103, `mult_projectile_speed`): scales the
/// launch speed of the weapon's projectiles, from 1 to 4.
#[doc(alias("Projectile speed increased"))]
pub const PROJECTILE_SPEED_INCREASED: AttributeDef<Multiplier> = AttributeDef::new(
	103,
	c"Projectile speed increased",
	c"mult_projectile_speed",
	DescriptionFormat::Percentage,
	1.0,
	4.0,
);

/// `provide on active` (128, `provide_on_active`): the weapon provides its
/// attributes to its owner only while it is the active weapon. Takes effect
/// after [`ItemAttributes::reapply_provision`].
///
/// [`ItemAttributes::reapply_provision`]: super::ItemAttributes::reapply_provision
#[doc(alias("provide on active"))]
pub const PROVIDE_ON_ACTIVE: AttributeDef<Flag> = AttributeDef::new(
	128,
	c"provide on active",
	c"provide_on_active",
	DescriptionFormat::Additive,
	0.0,
	1.0,
);

/// `Reload time decreased` (97, `mult_reload_time`): scales the reload time,
/// from 0.1 to 1.
#[doc(alias("Reload time decreased"))]
pub const RELOAD_TIME_DECREASED: AttributeDef<Multiplier> = AttributeDef::new(
	97,
	c"Reload time decreased",
	c"mult_reload_time",
	DescriptionFormat::InvertedPercentage,
	0.1,
	1.0,
);

/// `Reload time increased` (96, `mult_reload_time`): scales the reload time,
/// from 1 to 10.
#[doc(alias("Reload time increased"))]
pub const RELOAD_TIME_INCREASED: AttributeDef<Multiplier> = AttributeDef::new(
	96,
	c"Reload time increased",
	c"mult_reload_time",
	DescriptionFormat::Percentage,
	1.0,
	10.0,
);

/// `Set DamageType Ignite` (208, `set_dmgtype_ignite`): the weapon's hits set
/// players on fire.
#[doc(alias("Set DamageType Ignite"))]
pub const SET_DAMAGE_TYPE_IGNITE: AttributeDef<Flag> = AttributeDef::new(
	208,
	c"Set DamageType Ignite",
	c"set_dmgtype_ignite",
	DescriptionFormat::Additive,
	0.0,
	1.0,
);

/// `spread penalty` (36, `mult_spread_scale`): scales the weapon's bullet
/// spread, from 1 to 10.
#[doc(alias("spread penalty"))]
pub const SPREAD_PENALTY: AttributeDef<Multiplier> = AttributeDef::new(
	36,
	c"spread penalty",
	c"mult_spread_scale",
	DescriptionFormat::Percentage,
	1.0,
	10.0,
);

/// `weapon spread bonus` (106, `mult_spread_scale`): scales the weapon's
/// bullet spread, from 0 to 1.
#[doc(alias("weapon spread bonus"))]
pub const WEAPON_SPREAD_BONUS: AttributeDef<Multiplier> = AttributeDef::new(
	106,
	c"weapon spread bonus",
	c"mult_spread_scale",
	DescriptionFormat::InvertedPercentage,
	0.0,
	1.0,
);

#[cfg(test)]
mod tests {
	use super::*;
	use crate::tf2::attributes::definition::RawDef;
	use std::collections::{BTreeSet, HashMap};

	/// Every definition of the catalog.
	const ALL: [RawDef; 40] = [
		AIRBLAST_DISABLED.raw(),
		BLAST_RADIUS_DECREASED.raw(),
		BLAST_RADIUS_INCREASED.raw(),
		BULLETS_PER_SHOT_BONUS.raw(),
		CLIP_SIZE_BONUS.raw(),
		CLIP_SIZE_PENALTY.raw(),
		CRITBOOST_ON_KILL.raw(),
		CRIT_KILL_WILL_GIB.raw(),
		DAMAGE_BONUS.raw(),
		DAMAGE_PENALTY.raw(),
		DAMAGE_PENALTY_VS_PLAYERS.raw(),
		DAMAGE_TAKEN_FROM_BLAST_REDUCED.raw(),
		DAMAGE_TAKEN_INCREASED.raw(),
		DEPLOY_TIME_DECREASED.raw(),
		DEPLOY_TIME_INCREASED.raw(),
		FASTER_RELOAD_RATE.raw(),
		FIRE_RATE_BONUS.raw(),
		FIRE_RATE_PENALTY.raw(),
		HEALTH_REGEN.raw(),
		HEAL_ON_HIT_RAPID_FIRE.raw(),
		HEAL_ON_HIT_SLOW_FIRE.raw(),
		HEAL_ON_KILL.raw(),
		MAXAMMO_PRIMARY_INCREASED.raw(),
		MAXAMMO_PRIMARY_REDUCED.raw(),
		MAXAMMO_SECONDARY_INCREASED.raw(),
		MAXAMMO_SECONDARY_REDUCED.raw(),
		MAX_HEALTH_ADDITIVE_BONUS.raw(),
		MAX_HEALTH_ADDITIVE_PENALTY.raw(),
		MINICRITS_BECOME_CRITS.raw(),
		MINICRIT_VS_BURNING_PLAYER.raw(),
		MOVE_SPEED_BONUS.raw(),
		MOVE_SPEED_PENALTY.raw(),
		PROJECTILE_SPEED_DECREASED.raw(),
		PROJECTILE_SPEED_INCREASED.raw(),
		PROVIDE_ON_ACTIVE.raw(),
		RELOAD_TIME_DECREASED.raw(),
		RELOAD_TIME_INCREASED.raw(),
		SET_DAMAGE_TYPE_IGNITE.raw(),
		SPREAD_PENALTY.raw(),
		WEAPON_SPREAD_BONUS.raw(),
	];

	#[derive(Debug, PartialEq)]
	enum Token {
		Open,
		Close,
		String(String),
	}

	#[test]
	fn definitions_are_distinct_and_bounded_within_their_value_types() {
		let indices: BTreeSet<_> = ALL.iter().map(|def| def.index).collect();
		let names: BTreeSet<_> = ALL.iter().map(|def| def.name).collect();

		assert_eq!(indices.len(), ALL.len(), "duplicate definition index");
		assert_eq!(names.len(), ALL.len(), "duplicate schema name");

		for def in ALL {
			let name = def.name.to_string_lossy();

			assert!(def.min.is_finite() && def.max.is_finite(), "{name}");
			assert!(def.min < def.max, "{name}");
			assert!(!def.class.is_empty(), "{name}");
			// Index-like formats are excluded from the catalog.
			assert_ne!(def.format, DescriptionFormat::Or, "{name}");

			// A multiplier only ever moves the hooked value one way.
			if def.format.combine() == crate::tf2::attributes::Combine::Multiply {
				assert!(def.min >= 0.0, "{name}");
				assert!(def.max <= 1.0 || def.min >= 1.0, "{name}");
			}
		}

		for flag in [
			AIRBLAST_DISABLED,
			CRIT_KILL_WILL_GIB,
			MINICRIT_VS_BURNING_PLAYER,
			MINICRITS_BECOME_CRITS,
			PROVIDE_ON_ACTIVE,
			SET_DAMAGE_TYPE_IGNITE,
		] {
			assert_eq!((flag.min(), flag.max()), (0.0, 1.0));
		}
	}

	/// Checks every definition against an `items_game.txt` named by the
	/// `TF2_ITEMS_GAME` environment variable, such as a server's
	/// `tf/scripts/items/items_game.txt`. Run it with `--ignored`.
	#[test]
	#[ignore = "set TF2_ITEMS_GAME to a server's items_game.txt"]
	fn definitions_match_the_shipped_item_schema() {
		let path = std::env::var_os("TF2_ITEMS_GAME")
			.expect("TF2_ITEMS_GAME must name a server's items_game.txt");

		let text = std::fs::read_to_string(&path).expect("reading TF2_ITEMS_GAME");
		let schema = schema_attributes(&text);

		for def in ALL {
			let name = def.name.to_str().unwrap();
			let entry = schema
				.get(&def.index.get())
				.unwrap_or_else(|| panic!("{name}: index {} is not in the schema", def.index));
			let key = |key: &str| entry.get(key).map(String::as_str);

			assert_eq!(key("name"), Some(name), "name of {}", def.index);
			assert_eq!(
				key("attribute_class"),
				def.class.to_str().ok(),
				"class of {name}"
			);
			assert_eq!(
				key("description_format"),
				Some(def.format.keyword()),
				"format of {name}"
			);
			assert_eq!(key("attribute_type"), None, "type of {name}");
			assert!(
				matches!(key("stored_as_integer"), None | Some("0")),
				"storage of {name}"
			);
			assert!(
				matches!(key("hidden"), None | Some("0")),
				"visibility of {name}"
			);
		}
	}

	/// The entries of the top-level `attributes` section of `items_game.txt`,
	/// by definition index, as their keys and string values.
	fn schema_attributes(text: &str) -> HashMap<u16, HashMap<String, String>> {
		let mut tokens = tokens(text).into_iter().peekable();
		let mut depth = 0;
		let mut attributes = HashMap::new();

		// Find `"attributes" {` directly inside the root `"items_game" {`.
		while let Some(token) = tokens.next() {
			match token {
				Token::Open => depth += 1,
				Token::Close => depth -= 1,

				Token::String(key) if depth == 1 && key == "attributes" => {
					assert_eq!(tokens.next(), Some(Token::Open));
					break;
				}

				Token::String(_) => {}
			}
		}

		// Each entry is `"<index>" { "<key>" "<value>" ... }`, possibly with
		// nested sections, which are skipped.
		while let Some(token) = tokens.next() {
			let Token::String(index) = token else {
				assert_eq!(token, Token::Close, "malformed attributes section");
				break;
			};

			assert_eq!(tokens.next(), Some(Token::Open));
			let mut entry = HashMap::new();
			let mut nested = 0;

			loop {
				match tokens.next().expect("unterminated attribute") {
					Token::Close if nested == 0 => break,
					Token::Close => nested -= 1,
					Token::Open => nested += 1,

					// A key followed by a string is a value; otherwise a section.
					Token::String(key) => {
						if let Some(Token::String(_)) = tokens.peek()
							&& let Some(Token::String(value)) = tokens.next()
							&& nested == 0
						{
							entry.insert(key, value);
						}
					}
				}
			}

			if let Ok(index) = index.parse() {
				attributes.insert(index, entry);
			}
		}

		attributes
	}

	/// Splits KeyValues text into braces and quoted or bare strings, dropping
	/// `//` comments and conditionals such as `[$WIN32]`.
	fn tokens(text: &str) -> Vec<Token> {
		let mut tokens = Vec::new();
		let mut chars = text.chars().peekable();

		while let Some(c) = chars.next() {
			match c {
				'{' => tokens.push(Token::Open),
				'}' => tokens.push(Token::Close),

				'/' if chars.peek() == Some(&'/') => {
					for c in chars.by_ref() {
						if c == '\n' {
							break;
						}
					}
				}

				'[' => {
					for c in chars.by_ref() {
						if c == ']' {
							break;
						}
					}
				}

				'"' => {
					let mut string = String::new();

					while let Some(c) = chars.next() {
						match c {
							'"' => break,
							'\\' => string.extend(chars.next()),
							c => string.push(c),
						}
					}

					tokens.push(Token::String(string));
				}

				c if c.is_whitespace() => {}

				c => {
					let mut string = String::from(c);

					while let Some(&c) = chars.peek() {
						if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
							break;
						}

						string.push(c);
						chars.next();
					}

					tokens.push(Token::String(string));
				}
			}
		}

		tokens
	}
}
