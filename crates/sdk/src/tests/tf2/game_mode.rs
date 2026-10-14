//! Tests of `crate::tf2::game_mode`: the game modes of fake game rules, and
//! the raw values of the modes' types.

use super::*;
use crate::test_support::sdk_core::change_tracking_engine;
use crate::test_support::server::mock_server;

use crate::test_support::tf2::game_rules::{
	FLAG_NAMES, FLAGS, FORCE_ESCORT_PUSH, FORCE_UPGRADES, GAME_TYPE, HALLOWEEN_SCENARIO, HUD_TYPE,
	MAP_HOLIDAY, World, round_rules_proxy,
};

use sdk_raw::edicts::FL_FULL_EDICT_CHANGED;

/// The byte of the fake TF2 rules holding the flag named `name`.
fn flag(name: &CStr) -> usize {
	FLAGS + FLAG_NAMES.iter().position(|&flag| flag == name).unwrap()
}

#[test]
fn flags_are_read_from_their_variables() {
	let world = World::new(Some(round_rules_proxy));
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();

	// The first is cast to the pointer type of this scope's game rules, which
	// the others coerce to.
	let predicates = [
		(
			c"m_bPlayingKoth",
			GameRules::is_king_of_the_hill as fn(_) -> _,
		),
		(c"m_bPlayingMedieval", GameRules::is_medieval),
		(c"m_bPlayingHybrid_CTF_CP", GameRules::is_hybrid_ctf_cp),
		(
			c"m_bPlayingSpecialDeliveryMode",
			GameRules::is_special_delivery,
		),
		(
			c"m_bPlayingRobotDestructionMode",
			GameRules::is_robot_destruction,
		),
		(c"m_bPlayingMannVsMachine", GameRules::is_mann_vs_machine),
		(c"m_bPowerupMode", GameRules::is_mannpower),
		(c"m_bCompetitiveMode", GameRules::is_competitive),
		(c"m_bTruceActive", GameRules::is_truce_active),
		(c"m_bHelltowerPlayersInHell", GameRules::are_players_in_hell),
	];

	for (name, predicate) in predicates {
		assert_eq!(predicate(rules), Ok(false), "{name:?}");
		world.put_byte(false, flag(name), 1);

		// Each predicate reads its own flag alone.
		for (other, predicate) in predicates {
			assert_eq!(
				predicate(rules),
				Ok(other == name),
				"{other:?} after {name:?}"
			);
		}

		world.put_byte(false, flag(name), 0);
	}
}

#[test]
fn game_and_hud_types_are_read_and_unknown_ones_reported() {
	let world = World::new(Some(round_rules_proxy));
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();

	assert_eq!(rules.game_type(), Ok(GameType::Undefined));
	assert_eq!(rules.hud_type(), Ok(HudType::Undefined));

	world.put_int(false, GAME_TYPE, TF_GAMETYPE_ESCORT);
	world.put_int(false, HUD_TYPE, TF_HUDTYPE_TRAINING);
	assert_eq!(rules.game_type(), Ok(GameType::Payload));
	assert_eq!(rules.hud_type(), Ok(HudType::Training));

	world.put_int(false, GAME_TYPE, TF_GAMETYPE_PD + 1);
	world.put_int(false, HUD_TYPE, -1);
	assert_eq!(
		rules.game_type(),
		Err(GameRulesError::UnknownValue {
			variable: c"m_nGameType",
			value: TF_GAMETYPE_PD + 1,
		})
	);
	assert_eq!(
		rules.hud_type(),
		Err(GameRulesError::UnknownValue {
			variable: c"m_nHudType",
			value: -1,
		})
	);
}

#[test]
fn holidays_and_scenarios_are_none_unless_the_level_has_one() {
	let world = World::new(Some(round_rules_proxy));
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();

	assert_eq!(rules.map_holiday(), Ok(None));
	assert_eq!(rules.halloween_scenario(), Ok(None));

	world.put_int(false, MAP_HOLIDAY, HOLIDAY_HALLOWEEN);
	world.put_int(false, HALLOWEEN_SCENARIO, HALLOWEEN_SCENARIO_HIGHTOWER);
	assert_eq!(rules.map_holiday(), Ok(Some(Holiday::Halloween)));
	assert_eq!(
		rules.halloween_scenario(),
		Ok(Some(HalloweenScenario::Helltower))
	);

	world.put_int(false, MAP_HOLIDAY, HOLIDAY_SUMMER + 1);
	world.put_int(false, HALLOWEEN_SCENARIO, HALLOWEEN_SCENARIO_NONE - 1);
	assert_eq!(
		rules.map_holiday(),
		Err(GameRulesError::UnknownValue {
			variable: c"m_nMapHolidayType",
			value: HOLIDAY_SUMMER + 1,
		})
	);
	assert_eq!(
		rules.halloween_scenario(),
		Err(GameRulesError::UnknownValue {
			variable: c"m_halloweenScenario",
			value: HALLOWEEN_SCENARIO_NONE - 1,
		})
	);
}

#[test]
fn rule_overrides_and_the_underworld_are_set_for_clients() {
	let world = World::new(Some(round_rules_proxy));
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();
	let engine = change_tracking_engine();

	let overrides = [
		(
			FORCE_UPGRADES,
			GameRules::upgrades_override as fn(_) -> _,
			GameRules::set_upgrades_override as fn(_, _, _) -> _,
		),
		(
			FORCE_ESCORT_PUSH,
			GameRules::escort_push_override,
			GameRules::set_escort_push_override,
		),
	];

	for (offset, get, set) in overrides {
		assert_eq!(get(rules), Ok(RuleOverride::Default));

		set(rules, engine, RuleOverride::On).unwrap();
		assert_eq!(world.int(false, offset), 2);
		assert_eq!(get(rules), Ok(RuleOverride::On));

		world.put_int(false, offset, 3);
		assert!(matches!(
			get(rules),
			Err(GameRulesError::UnknownValue { value: 3, .. })
		));
	}

	assert_ne!(world.state_flags() & FL_FULL_EDICT_CHANGED, 0);

	let in_hell = flag(c"m_bHelltowerPlayersInHell");

	rules.set_players_in_hell(engine, true).unwrap();
	assert_eq!(world.byte(false, in_hell), 1);
	assert_eq!(rules.are_players_in_hell(), Ok(true));
}

#[test]
fn spells_are_turned_on_and_off_for_clients() {
	let world = World::new(Some(round_rules_proxy));
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();
	let engine = change_tracking_engine();
	let spells = flag(c"m_bIsUsingSpells");

	assert_eq!(rules.uses_spells(), Ok(false));

	world.put_byte(false, spells, 1);
	assert_eq!(rules.uses_spells(), Ok(true));

	rules.set_uses_spells(engine, false).unwrap();
	assert_eq!(world.byte(false, spells), 0);
	assert_eq!(rules.uses_spells(), Ok(false));
	assert_ne!(world.state_flags() & FL_FULL_EDICT_CHANGED, 0);

	rules.set_uses_spells(engine, true).unwrap();
	assert_eq!(world.byte(false, spells), 1);
}
