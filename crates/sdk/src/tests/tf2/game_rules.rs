//! Tests of `crate::tf2::game_rules`: finding fake game rules through a
//! `tf_gamerules` entity whose send tables nest them as TF2's do.

use super::*;
use crate::test_support::datatables::direct_table;
use crate::test_support::entities::{MockEntity, set_networking};
use crate::test_support::entities::{base_entity_fields, set_accepts, set_datamap, take_inputs};
use crate::test_support::sdk_core::change_tracking_engine;
use crate::test_support::server::{mock_server, null_server};

use crate::test_support::tf2::game_rules::{
	ENTITIES, GAME_TYPE, PROXY_CALLS, ROUND_STATE, RULES, RULES_SIZE, SETUP, SHADOWED, Tables,
	WAITING, World, no_rules_proxy, round_rules_proxy,
};

use crate::test_support::tf2::game_rules::{ROUND_FLAGS, ROUNDS_PLAYED, WINNING_TEAM};
use sdk_raw::edicts::{FL_EDICT_CHANGED, FL_FULL_EDICT_CHANGED};
use sdk_raw::entities::EFL_KILLME;
use sdk_raw::entities::datamap::FTYPEDESC_INPUT;
use sdk_raw::test_support::entities::data_map;
use sdk_raw::tf2::scoreboard::{TF_TEAM_BLUE, TF_TEAM_RED};
use std::ptr::null_mut;

#[test]
fn game_rules_are_read_from_the_objects_the_proxies_give() {
	let mut world = World::new(Some(round_rules_proxy));
	let scope = ();
	let server = mock_server(&scope);

	world.put_int(true, ROUND_STATE, GR_STATE_PREGAME);
	world.put_round_byte(WAITING, 1);
	world.put_int(false, GAME_TYPE, 4);
	world.put_int(false, SHADOWED, GR_STATE_BONUS);

	let rules = GameRules::get(server).unwrap();
	let entity = world.entity.as_ptr().cast::<c_void>().cast_const();

	// Each proxy is given the entity, its edict index, and every recipient.
	assert_eq!(
		PROXY_CALLS.take(),
		[(entity, entity, 7, true), (entity, entity, 7, true)]
	);
	assert_eq!(rules.as_ptr(), world.rules.cast());

	// A name in both tables is the round-based rules' variable.
	assert_eq!(rules.round_state(), Ok(RoundState::Pregame));
	assert_eq!(rules.is_waiting_for_players(), Ok(true));
	assert_eq!(rules.is_in_setup(), Ok(false));

	// The TF2 rules' own variables are read from the object their proxy gave.
	assert_eq!(rules.read::<i32>(c"m_nGameType"), Ok(4));

	world.put_int(true, ROUND_STATE, GR_STATE_STARTGAME);
	world.put_round_byte(SETUP, 1);
	assert_eq!(rules.round_state(), Ok(RoundState::StartGame));
	assert_eq!(rules.is_in_setup(), Ok(true));

	world.put_int(true, ROUND_STATE, GR_STATE_BETWEEN_RNDS + 1);
	assert_eq!(
		rules.round_state(),
		Err(GameRulesError::UnknownRoundState(GR_STATE_BETWEEN_RNDS + 1))
	);

	assert!(matches!(
		rules.read::<i32>(c"m_bMissing"),
		Err(GameRulesError::NetProp(NetPropError::NotFound { table, .. })) if table == "DT_TFGameRules"
	));
	assert!(matches!(
		rules.read::<i16>(c"m_iRoundState"),
		Err(GameRulesError::NetProp(NetPropError::TypeMismatch { .. }))
	));
}

#[test]
fn game_rules_are_written_where_they_are_read() {
	let world = World::new(Some(round_rules_proxy));
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();

	// SAFETY: The fake game rules accept any value of their variables.
	unsafe { rules.write(c"m_bInWaitingForPlayers", true) }.unwrap();

	// The flag's byte alone changed, to the value the game stores for true.
	let mut expected = [0; RULES_SIZE];
	expected[WAITING] = 1;
	assert_eq!(world.round_bytes(), expected);
	assert_eq!(rules.is_waiting_for_players(), Ok(true));

	// A name in both tables is the round-based rules' variable, and the TF2
	// rules' own variables are written in the object their proxy gave.
	// SAFETY: As above.
	unsafe {
		rules.write(c"m_iRoundState", GR_STATE_BONUS).unwrap();
		rules.write(c"m_nGameType", 4).unwrap();
		rules.write(c"m_bInWaitingForPlayers", false).unwrap();
	}

	assert_eq!(world.int(true, ROUND_STATE), GR_STATE_BONUS);
	assert_eq!(world.int(false, SHADOWED), 0);
	assert_eq!(world.int(false, GAME_TYPE), 4);
	assert_eq!(rules.is_waiting_for_players(), Ok(false));

	// Variables stored as another type, or in neither table, are not written.
	assert!(matches!(
		// SAFETY: As above.
		unsafe { rules.write::<i16>(c"m_iRoundState", 1) },
		Err(GameRulesError::NetProp(NetPropError::TypeMismatch { .. }))
	));
	assert!(matches!(
		// SAFETY: As above.
		unsafe { rules.write(c"m_bMissing", true) },
		Err(GameRulesError::NetProp(NetPropError::NotFound { .. }))
	));
	assert_eq!(world.int(true, ROUND_STATE), GR_STATE_BONUS);
}

#[test]
fn game_rules_need_tf2_its_tables_and_a_live_proxy_entity() {
	let scope = ();

	assert_eq!(
		GameRules::get(null_server(Game::SourceSdk2013, &scope)),
		Err(GameRulesError::WrongGame)
	);
	assert!(matches!(
		GameRules::get(null_server(Game::TeamFortress2, &scope)),
		Err(GameRulesError::Interface(_))
	));

	// A proxy that keeps the entity's address would not reach the game rules,
	// a base class's table must be the entity's own, and the nested tables
	// must be the game rules'.
	let tf2 = Tables::TF2;

	for tables in [
		Tables {
			round_proxy: Some(direct_table),
			..tf2
		},
		Tables {
			base: (8, Some(direct_table)),
			..tf2
		},
		Tables {
			base: (0, Some(round_rules_proxy)),
			..tf2
		},
		Tables {
			rules_table: c"DT_GameRules",
			..tf2
		},
	] {
		let _world = World::with(tables);
		assert_eq!(
			GameRules::get(mock_server(&scope)),
			Err(GameRulesError::UnexpectedTables)
		);
	}

	// Without game rules, the proxies give nothing.
	let _world = World::new(Some(no_rules_proxy));
	assert_eq!(
		GameRules::get(mock_server(&scope)),
		Err(GameRulesError::NoGameRules)
	);

	let world = World::new(Some(round_rules_proxy));
	RULES.set((null_mut(), world.round_rules.cast()));
	assert_eq!(
		GameRules::get(mock_server(&scope)),
		Err(GameRulesError::NoGameRules)
	);

	// A proxy entity being removed, or of another class, is not the game's.
	let mut world = World::new(Some(round_rules_proxy));
	world.entity.set_eflags(EFL_KILLME);
	assert_eq!(
		GameRules::get(mock_server(&scope)),
		Err(GameRulesError::NoProxyEntity)
	);

	world.entity.set_eflags(0);
	set_networking(null_mut(), world.edict);
	assert_eq!(
		GameRules::get(mock_server(&scope)),
		Err(GameRulesError::NoProxyEntity)
	);

	set_networking(world.class, world.edict);
	assert!(GameRules::get(mock_server(&scope)).is_ok());

	ENTITIES.set(Vec::new());
	assert_eq!(
		GameRules::get(mock_server(&scope)),
		Err(GameRulesError::NoProxyEntity)
	);
}

/// An input of the proxy entity's datamap, taking `field_type`.
fn input(name: &'static CStr, field_type: sys::fieldtype_t) -> sys::typedescription_t {
	// SAFETY: Zero is valid for every field of `typedescription_t`.
	let mut input: sys::typedescription_t = unsafe { std::mem::zeroed() };

	input.fieldType = field_type;
	input.externalName = name.as_ptr();
	input.flags = FTYPEDESC_INPUT;
	input
}

#[test]
fn inputs_are_sent_to_the_proxy_entity_from_itself() {
	use sys::{
		_fieldtypes_FIELD_FLOAT as FLOAT, _fieldtypes_FIELD_INTEGER as INTEGER,
		_fieldtypes_FIELD_VOID as VOID,
	};

	let mut world = World::new(Some(round_rules_proxy));
	let scope = ();
	let server = mock_server(&scope);
	let rules = GameRules::get(server).unwrap();
	let tools = server.server_tools().unwrap();
	let entity = world.entity.as_ptr();

	set_datamap(data_map(
		c"CTFGameRulesProxy",
		vec![
			input(c"SetBlueKothClockActive", VOID),
			input(c"AddRedTeamRespawnWaveTime", FLOAT),
			input(c"SetBlueTeamRespawnWaveTime", FLOAT),
			input(c"AddBlueTeamScore", INTEGER),
			input(c"SetRedTeamRole", INTEGER),
		],
		data_map(c"CBaseEntity", base_entity_fields().to_vec(), null_mut()),
	));
	set_accepts(true);
	take_inputs();

	rules.activate_koth_clock(tools, ScoringTeam::Blue).unwrap();
	rules
		.add_respawn_wave_time(tools, ScoringTeam::Red, -2.5)
		.unwrap();
	rules
		.set_respawn_wave_time(tools, ScoringTeam::Blue, 8.0)
		.unwrap();
	rules.add_team_score(tools, ScoringTeam::Blue, -1).unwrap();
	rules
		.set_team_role(tools, ScoringTeam::Red, TeamRole::Attackers)
		.unwrap();

	let sent = take_inputs()
		.into_iter()
		.map(|input| {
			assert_eq!(
				(input.target, input.activator, input.caller),
				(entity, entity, entity)
			);

			let value = <[u8; 4]>::try_from(&input.payload[..4]).unwrap();

			(input.name, input.field_type, value)
		})
		.collect::<Vec<_>>();

	assert_eq!(
		sent,
		[
			(c"SetBlueKothClockActive".to_owned(), VOID, sent[0].2),
			(
				c"AddRedTeamRespawnWaveTime".to_owned(),
				FLOAT,
				(-2.5f32).to_ne_bytes()
			),
			(
				c"SetBlueTeamRespawnWaveTime".to_owned(),
				FLOAT,
				8.0f32.to_ne_bytes()
			),
			(
				c"AddBlueTeamScore".to_owned(),
				INTEGER,
				(-1i32).to_ne_bytes()
			),
			(
				c"SetRedTeamRole".to_owned(),
				INTEGER,
				TEAM_ROLE_ATTACKERS.to_ne_bytes()
			),
		]
	);

	// The other team's inputs are not in the datamap, which `AcceptInput` would
	// not find either.
	assert_eq!(
		rules.set_team_role(tools, ScoringTeam::Blue, TeamRole::Defenders),
		Err(InputError::UnknownInput)
	);
	assert!(take_inputs().is_empty());
}

#[test]
fn round_progress_is_read_from_the_round_based_rules() {
	let world = World::new(Some(round_rules_proxy));
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();

	let flags = [
		GameRules::is_in_overtime as fn(_) -> _,
		GameRules::is_stopwatch,
		GameRules::has_multiple_trains,
		GameRules::switched_teams_this_round,
	];

	for (offset, flag) in (ROUND_FLAGS..).zip(flags) {
		assert_eq!(flag(rules), Ok(false));
		world.put_round_byte(offset, 1);
		assert_eq!(flag(rules), Ok(true));
	}

	world.put_int(true, ROUNDS_PLAYED, 3);
	assert_eq!(rules.rounds_played(), Ok(3));

	// No team won yet, or after a stalemate.
	assert_eq!(rules.winning_team(), Ok(None));
	world.put_int(true, WINNING_TEAM, TF_TEAM_BLUE);
	assert_eq!(rules.winning_team(), Ok(Some(ScoringTeam::Blue)));
	world.put_int(true, WINNING_TEAM, TF_TEAM_RED);
	assert_eq!(rules.winning_team(), Ok(Some(ScoringTeam::Red)));
	world.put_int(true, WINNING_TEAM, TF_TEAM_RED - 1);
	assert_eq!(rules.winning_team(), Ok(None));
}

#[test]
fn round_states_round_trip_through_their_raw_values() {
	for (state, raw) in RoundState::ALL.into_iter().zip(GR_STATE_INIT..) {
		assert_eq!(state.to_raw(), raw);
		assert_eq!(RoundState::from_raw(raw), Some(state));
	}

	assert_eq!(
		RoundState::ALL.last().unwrap().to_raw(),
		GR_STATE_BETWEEN_RNDS
	);
	assert_eq!(RoundState::from_raw(GR_STATE_INIT - 1), None);
	assert_eq!(RoundState::from_raw(GR_STATE_BETWEEN_RNDS + 1), None);
}

#[test]
fn set_variables_are_recorded_for_clients_on_the_proxy_entity() {
	let mut world = World::new(Some(round_rules_proxy));
	let scope = ();
	let rules = GameRules::get(mock_server(&scope)).unwrap();
	let engine = change_tracking_engine();
	let changed = FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED;

	assert_eq!(rules.proxy().as_ptr(), world.entity.as_ptr());
	assert_eq!(world.state_flags() & changed, 0);

	// SAFETY: The fake game rules accept any value of their variables.
	unsafe { rules.set(engine, c"m_nGameType", 2) }.unwrap();
	assert_eq!(world.int(false, GAME_TYPE), 2);
	assert_eq!(world.state_flags() & changed, changed);

	// A variable that is not written records no change.
	// SAFETY: The edict is leaked, and only the tests and the engine's change
	// tracking write it.
	unsafe { (*world.edict)._base.m_fStateFlags = 0 };
	assert!(matches!(
		// SAFETY: As above.
		unsafe { rules.set(engine, c"m_bMissing", true) },
		Err(GameRulesError::NetProp(NetPropError::NotFound { .. }))
	));
	assert_eq!(world.state_flags(), 0);

	rules.network_state_changed(engine);
	assert_eq!(world.state_flags() & changed, changed);
}

#[test]
fn the_game_rules_vtable_is_only_searched_for_on_tf2() {
	let scope = ();

	assert!(matches!(
		game_rules_vtable(null_server(Game::SourceSdk2013, &scope)),
		Err(GameRulesVtableError::WrongGame)
	));

	// The tests' own executable has no `CTFGameRules`.
	assert!(matches!(
		game_rules_vtable(mock_server(&scope)),
		Err(GameRulesVtableError::NotFound)
	));
}

#[test]
fn the_proxy_entity_a_maps_own_replaces_is_skipped() {
	let scope = ();
	let mut world = World::new(Some(round_rules_proxy));
	let mut replaced = MockEntity::new(2);

	// Building another mock entity resets what they report.
	replaced.set_eflags(EFL_KILLME);
	set_networking(world.class, world.edict);
	ENTITIES.set(vec![replaced.as_ptr(), world.entity.as_ptr()]);

	let rules = GameRules::get(mock_server(&scope)).unwrap();
	let entity = world.entity.as_ptr().cast::<c_void>().cast_const();

	assert_eq!(rules.as_ptr(), world.rules.cast());
	assert_eq!(
		PROXY_CALLS.take(),
		[(entity, entity, 7, true), (entity, entity, 7, true)]
	);
}
