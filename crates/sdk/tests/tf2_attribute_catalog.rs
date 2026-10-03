#![cfg(feature = "tf2")]

//! Tests of TF2's attribute catalog against the item schema the game ships.

use source_sdk_2013::tf2::attributes::catalog::*;

use source_sdk_2013::tf2::attributes::{
	AttributeDef, AttributeIndex, AttributeValue, DescriptionFormat,
};

use std::collections::HashMap;
use std::ffi::CStr;

/// What the shipped schema gives a catalog definition.
#[derive(Debug, Clone, Copy)]
struct Def {
	name: &'static CStr,
	index: AttributeIndex,
	class: &'static CStr,
	format: DescriptionFormat,
}

impl Def {
	fn of<V: AttributeValue>(def: &AttributeDef<V>) -> Self {
		Self {
			name: def.name(),
			index: def.index(),
			class: def.class(),
			format: def.format(),
		}
	}
}

#[derive(Debug, PartialEq)]
enum Token {
	Open,
	Close,
	String(String),
}

/// Every definition of the catalog.
fn all() -> [Def; 40] {
	[
		Def::of(&AIRBLAST_DISABLED),
		Def::of(&BLAST_RADIUS_DECREASED),
		Def::of(&BLAST_RADIUS_INCREASED),
		Def::of(&BULLETS_PER_SHOT_BONUS),
		Def::of(&CLIP_SIZE_BONUS),
		Def::of(&CLIP_SIZE_PENALTY),
		Def::of(&CRITBOOST_ON_KILL),
		Def::of(&CRIT_KILL_WILL_GIB),
		Def::of(&DAMAGE_BONUS),
		Def::of(&DAMAGE_PENALTY),
		Def::of(&DAMAGE_PENALTY_VS_PLAYERS),
		Def::of(&DAMAGE_TAKEN_FROM_BLAST_REDUCED),
		Def::of(&DAMAGE_TAKEN_INCREASED),
		Def::of(&DEPLOY_TIME_DECREASED),
		Def::of(&DEPLOY_TIME_INCREASED),
		Def::of(&FASTER_RELOAD_RATE),
		Def::of(&FIRE_RATE_BONUS),
		Def::of(&FIRE_RATE_PENALTY),
		Def::of(&HEALTH_REGEN),
		Def::of(&HEAL_ON_HIT_RAPID_FIRE),
		Def::of(&HEAL_ON_HIT_SLOW_FIRE),
		Def::of(&HEAL_ON_KILL),
		Def::of(&MAXAMMO_PRIMARY_INCREASED),
		Def::of(&MAXAMMO_PRIMARY_REDUCED),
		Def::of(&MAXAMMO_SECONDARY_INCREASED),
		Def::of(&MAXAMMO_SECONDARY_REDUCED),
		Def::of(&MAX_HEALTH_ADDITIVE_BONUS),
		Def::of(&MAX_HEALTH_ADDITIVE_PENALTY),
		Def::of(&MINICRITS_BECOME_CRITS),
		Def::of(&MINICRIT_VS_BURNING_PLAYER),
		Def::of(&MOVE_SPEED_BONUS),
		Def::of(&MOVE_SPEED_PENALTY),
		Def::of(&PROJECTILE_SPEED_DECREASED),
		Def::of(&PROJECTILE_SPEED_INCREASED),
		Def::of(&PROVIDE_ON_ACTIVE),
		Def::of(&RELOAD_TIME_DECREASED),
		Def::of(&RELOAD_TIME_INCREASED),
		Def::of(&SET_DAMAGE_TYPE_IGNITE),
		Def::of(&SPREAD_PENALTY),
		Def::of(&WEAPON_SPREAD_BONUS),
	]
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

	for def in all() {
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
