//! Tests of TF2 taunts through a fake player's native methods and networked
//! variables.

use super::*;
use crate::Game;
use crate::test_support::server::{mock_server, null_server};
use crate::test_support::tf2::player::{FakePlayer, Method, Value, Var, VarKind, serve_interfaces};
use sdk_raw::tf2::script_binding::{BOOL, FLOAT, INT, VOID};
use std::cell::Cell;
use std::rc::Rc;

/// `TAUNT_LONG`, the kind of a press-and-hold taunt.
const LONG: c_int = sys::taunts_t_TAUNT_LONG as c_int;

/// The native taunt methods of fake players.
const METHODS: &[Method] = &[
	Method::new(c"CancelTaunt", VOID, &[]),
	Method::new(c"EndLongTaunt", VOID, &[]),
	Method::new(c"GetCurrentTauntMoveSpeed", FLOAT, &[]),
	Method::new(c"GetTauntRemoveTime", FLOAT, &[]),
	Method::new(c"HandleTauntCommand", VOID, &[INT]),
	Method::new(c"InCond", BOOL, &[INT]),
	Method::new(c"IsAllowedToRemoveTaunt", BOOL, &[]),
	Method::new(c"IsAllowedToTaunt", BOOL, &[]),
	Method::new(c"IsTaunting", BOOL, &[]),
	Method::new(c"SetCurrentTauntMoveSpeed", VOID, &[FLOAT]),
	Method::new(c"SetForcedTauntCam", VOID, &[INT]),
	Method::new(c"StopTaunt", VOID, &[BOOL]),
	Method::new(c"Taunt", VOID, &[INT, INT]),
];

/// The taunts' networked variables of fake players.
const VARS: &[Var] = &[
	Var::new(c"m_bAllowMoveDuringTaunt", VarKind::Bool),
	Var::new(c"m_flCurrentTauntMoveSpeed", VarKind::Float),
	Var::new(c"m_iTauntConcept", VarKind::Int),
	Var::new(c"m_iTauntIndex", VarKind::Int),
	Var::new(c"m_iTauntItemDefIndex", VarKind::Int),
];

/// The game's taunt methods, for a player who is taunting while `taunting`
/// holds, and starts taunting when told to if `starts`. Taunts move at 120
/// units a second, and end at 9.5 seconds.
fn game(
	taunting: Rc<Cell<bool>>,
	starts: bool,
) -> impl FnMut(&FakePlayer, &CStr, &[Value]) -> Option<Value> {
	move |_, method, arguments| {
		Some(match (method.to_bytes(), arguments) {
			(b"GetCurrentTauntMoveSpeed", []) => Value::Float(120.0),
			(b"GetTauntRemoveTime", []) => Value::Float(9.5),

			(b"HandleTauntCommand" | b"Taunt", _) => {
				taunting.set(starts);
				Value::Void
			}

			(b"InCond", &[Value::Int(raw)]) => {
				Value::Bool(raw == Condition::TAUNTING.to_raw() && taunting.get())
			}

			(b"IsTaunting", []) => Value::Bool(taunting.get()),

			// The answers of methods that return nothing are dropped.
			_ => Value::Bool(true),
		})
	}
}

#[test]
fn long_taunts_end_only_when_an_item_plays_one() {
	let (fake, taunts, taunting) = player();
	let special = sys::taunts_t_TAUNT_SPECIAL as c_int;

	assert_eq!(taunts.end_long_taunt(), Ok(false));

	taunting.set(true);

	for (kind, item) in [(LONG, -1_i32), (LONG, 0xFFFF), (special, 1196)] {
		fake.set(c"m_iTauntIndex", kind.cast_unsigned());
		fake.set(c"m_iTauntItemDefIndex", item.cast_unsigned());
		assert_eq!(taunts.end_long_taunt(), Ok(false));
	}

	assert!(
		fake.take_calls()
			.iter()
			.all(|(method, _)| *method == c"InCond")
	);

	fake.set(c"m_iTauntIndex", LONG.cast_unsigned());
	fake.set(c"m_iTauntItemDefIndex", 1196);
	assert_eq!(taunts.end_long_taunt(), Ok(true));
	assert_eq!(fake.take_calls().last(), Some(&(c"EndLongTaunt", vec![])));
}

/// A fake player with the taunts' methods and variables, who starts taunting
/// when told to, and its taunts, on a mock server that serves them.
fn player() -> (&'static FakePlayer, PlayerTaunts<'static>, Rc<Cell<bool>>) {
	serve_interfaces();

	let fake = FakePlayer::new(METHODS, VARS);
	let taunting = Rc::new(Cell::new(false));

	fake.answer(game(taunting.clone(), true));
	(
		fake,
		PlayerTaunts::new(mock_server(&()), fake.entity()).unwrap(),
		taunting,
	)
}

#[test]
fn taunt_settings_and_reads_are_passed_on() {
	let (fake, taunts, _) = player();

	for speed in [-1.0, f32::NAN, f32::INFINITY] {
		assert_eq!(
			taunts.set_move_speed(speed),
			Err(EffectError::OutOfRange("speed"))
		);
	}

	assert!(fake.take_calls().is_empty());

	taunts.set_forced_third_person(true).unwrap();
	taunts.set_forced_third_person(false).unwrap();
	taunts.set_move_speed(250.0).unwrap();
	taunts.stop(true).unwrap();
	taunts.cancel().unwrap();
	assert_eq!(taunts.move_speed(), Ok(120.0));
	assert_eq!(taunts.remove_time(), Ok(9.5));
	assert_eq!(taunts.is_allowed_to_taunt(), Ok(true));
	assert_eq!(taunts.is_allowed_to_remove_taunt(), Ok(true));
	assert_eq!(
		fake.take_calls(),
		[
			(c"SetForcedTauntCam", vec![Value::Int(1)]),
			(c"SetForcedTauntCam", vec![Value::Int(0)]),
			(c"SetCurrentTauntMoveSpeed", vec![Value::Float(250.0)]),
			(c"StopTaunt", vec![Value::Bool(true)]),
			(c"CancelTaunt", vec![]),
			(c"GetCurrentTauntMoveSpeed", vec![]),
			(c"GetTauntRemoveTime", vec![]),
			(c"IsAllowedToTaunt", vec![]),
			(c"IsAllowedToRemoveTaunt", vec![]),
		]
	);

	assert_eq!(
		PlayerTaunts::new(null_server(Game::SourceSdk2013, &()), fake.entity()).err(),
		Some(EffectError::NotTfPlayer)
	);
}

#[test]
fn taunt_state_is_read_while_taunting() {
	let (fake, taunts, taunting) = player();

	fake.set(c"m_iTauntItemDefIndex", 1196);
	assert_eq!(taunts.state(), Ok(None));

	taunting.set(true);
	fake.set(c"m_bAllowMoveDuringTaunt", 1);
	fake.set(c"m_iTauntConcept", 5);
	fake.set(c"m_iTauntIndex", LONG.cast_unsigned());
	fake.set_float(c"m_flCurrentTauntMoveSpeed", 0, 120.0);

	let state = taunts.state().unwrap().unwrap();

	assert_eq!(
		state,
		TauntState {
			can_move: true,
			concept: 5,
			item: ItemDefinitionIndex::new(1196),
			kind: LONG,
			move_speed: 120.0,
		}
	);
	assert!(state.is_long());

	// A weapon's taunt plays no item.
	fake.set(c"m_iTauntItemDefIndex", (-1_i32).cast_unsigned());
	fake.set(
		c"m_iTauntIndex",
		TauntKind::BaseWeapon.to_raw().cast_unsigned(),
	);

	let state = taunts.state().unwrap().unwrap();

	assert_eq!((state.item, state.is_long()), (None, false));
}

#[test]
fn taunts_start_by_kind_or_loadout_slot() {
	let (fake, taunts, taunting) = player();

	assert_eq!(
		taunts.taunt_slot(TAUNT_SLOTS + 1),
		Err(EffectError::OutOfRange("slot"))
	);
	assert!(fake.take_calls().is_empty());

	assert_eq!(taunts.taunt(TauntKind::ShowItem), Ok(true));
	assert_eq!(taunts.taunt_slot(TAUNT_SLOTS), Ok(true));
	assert_eq!(
		fake.take_calls(),
		[
			(
				c"Taunt",
				vec![
					Value::Int(sys::taunts_t_TAUNT_SHOW_ITEM as c_int),
					Value::Int(0),
				]
			),
			(c"IsTaunting", vec![]),
			(c"HandleTauntCommand", vec![Value::Int(8)]),
			(c"IsTaunting", vec![]),
		]
	);

	// The game may refuse, as while the player is stunned.
	taunting.set(false);
	fake.answer(game(taunting, false));
	assert_eq!(taunts.taunt(TauntKind::BaseWeapon), Ok(false));
	assert_eq!(taunts.taunt_slot(0), Ok(false));
	assert_eq!(
		fake.take_calls()[0],
		(
			c"Taunt",
			vec![
				Value::Int(sys::taunts_t_TAUNT_BASE_WEAPON as c_int),
				Value::Int(0),
			]
		)
	);
}
