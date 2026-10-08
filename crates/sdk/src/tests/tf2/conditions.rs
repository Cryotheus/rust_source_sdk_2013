//! Tests of TF2 conditions through a fake player, whose native condition
//! methods keep its conditions in its networked condition bits, as the game's
//! do.

use super::*;
use crate::Game;
use crate::test_support::server::{mock_server, null_server};
use crate::test_support::tf2::player::{FakePlayer, Method, Value, Var, VarKind, serve_interfaces};
use sdk_raw::entities::EFL_KILLME;
use sdk_raw::tf2::script_binding::{BOOL, FLOAT, HANDLE, INT, VOID};
use std::ptr::null_mut;

/// The native condition methods of fake players.
const METHODS: &[Method] = &[
	Method::new(c"AddCondEx", VOID, &[INT, FLOAT, HANDLE]),
	Method::new(c"CanBeDebuffed", BOOL, &[]),
	Method::new(c"InCond", BOOL, &[INT]),
	Method::new(c"IsControlStunned", BOOL, &[]),
	Method::new(c"IsCritBoosted", BOOL, &[]),
	Method::new(c"IsFullyInvisible", BOOL, &[]),
	Method::new(c"IsImmuneToPushback", BOOL, &[]),
	Method::new(c"IsInvulnerable", BOOL, &[]),
	Method::new(c"IsSnared", BOOL, &[]),
	Method::new(c"IsStealthed", BOOL, &[]),
	Method::new(c"RemoveAllCond", VOID, &[]),
	Method::new(c"RemoveCondEx", VOID, &[INT, BOOL]),
	Method::new(c"SetCondDuration", VOID, &[INT, FLOAT]),
];

/// The condition bits of fake players, as `CTFPlayerShared` networks them.
const VARS: &[Var] = &[
	Var::new(c"m_nPlayerCond", VarKind::UInt),
	Var::new(c"_condition_bits", VarKind::UInt),
	Var::new(c"m_nPlayerCondEx", VarKind::UInt),
	Var::new(c"m_nPlayerCondEx2", VarKind::UInt),
	Var::new(c"m_nPlayerCondEx3", VarKind::UInt),
	Var::new(c"m_nPlayerCondEx4", VarKind::UInt),
];

#[test]
fn bits_hold_sets_of_conditions() {
	let last = Condition::from_raw(sys::ETFCond_TF_COND_LAST - 1).unwrap();
	let mut bits = ConditionBits::of(&[Condition::AIMING, Condition::URINE, last]);

	assert_eq!(bits.len(), 3);
	assert!(bits.contains(last) && !bits.contains(Condition::ZOOMED));
	assert!(bits.insert(Condition::ZOOMED));
	assert!(!bits.insert(Condition::ZOOMED));
	assert!(bits.remove(last));
	assert!(!bits.remove(last));
	assert_eq!(
		bits.iter().collect::<Vec<_>>(),
		[Condition::AIMING, Condition::ZOOMED, Condition::URINE]
	);
	assert_eq!(bits.iter().collect::<ConditionBits>(), bits);

	assert_eq!(ConditionBits::DEBUFFS.len(), 5);
	assert_eq!(
		bits.intersection(ConditionBits::DEBUFFS),
		ConditionBits::of(&[Condition::URINE])
	);
	assert_eq!(
		bits.difference(ConditionBits::DEBUFFS),
		ConditionBits::of(&[Condition::AIMING, Condition::ZOOMED])
	);
	assert_eq!(bits.union(ConditionBits::DEBUFFS).len(), 7);

	let mut extended = ConditionBits::default();

	assert!(extended.is_empty() && extended == ConditionBits::EMPTY);
	extended.extend([Condition::BLEEDING, last]);
	assert_eq!(extended.len(), 2);

	let changes = bits.changes_since(ConditionBits::of(&[Condition::AIMING, Condition::BURNING]));

	assert_eq!(
		changes,
		ConditionChanges {
			added: ConditionBits::of(&[Condition::ZOOMED, Condition::URINE]),
			removed: ConditionBits::of(&[Condition::BURNING]),
		}
	);
	assert!(!changes.is_empty());
	assert!(bits.changes_since(bits).is_empty());
}

#[test]
fn bits_need_the_variables_as_the_game_declares_them() {
	let scope = ();
	let server = mock_server(&scope);
	let fake = player(VARS);
	let bits =
		|fake: &'static FakePlayer| PlayerConditions::new(server, fake.entity()).unwrap().bits();

	// Without the game DLL, its standard proxies tell nothing of the storage.
	assert_eq!(bits(fake), Err(ConditionError::UnsupportedLayout));

	serve_interfaces();
	assert_eq!(
		bits(player(&VARS[..5])),
		Err(ConditionError::UnsupportedLayout)
	);

	for kind in [VarKind::Bool, VarKind::Float, VarKind::Handle] {
		let mut vars = VARS.to_vec();

		vars[3] = Var::new(c"m_nPlayerCondEx2", kind);
		assert_eq!(bits(player(&vars)), Err(ConditionError::UnsupportedLayout));
	}

	assert_eq!(bits(fake), Ok(ConditionBits::EMPTY));
}

#[test]
fn bits_read_every_word_and_the_condition_list() {
	serve_interfaces();

	let scope = ();
	let server = mock_server(&scope);
	let fake = player(VARS);
	let conditions = PlayerConditions::new(server, fake.entity()).unwrap();
	let held = [0, 1, 24, 40, 70, 100, sys::ETFCond_TF_COND_LAST - 1]
		.map(|raw| Condition::from_raw(raw).unwrap());

	assert_eq!(conditions.bits(), Ok(ConditionBits::EMPTY));

	for condition in held {
		give(fake, condition);
	}

	// The game keeps the crit boost in its condition list's bits.
	fake.set(c"_condition_bits", 1 << Condition::CRITBOOSTED.to_raw());

	let mut expected = ConditionBits::of(&held);

	expected.insert(Condition::CRITBOOSTED);
	assert_eq!(conditions.bits(), Ok(expected));
	assert!(fake.take_calls().is_empty());
}

#[test]
fn cleansing_removes_only_held_conditions() {
	serve_interfaces();

	let scope = ();
	let fake = player(VARS);
	let conditions = PlayerConditions::new(mock_server(&scope), fake.entity()).unwrap();

	for condition in [
		Condition::BURNING,
		Condition::URINE,
		Condition::ZOOMED,
		Condition::GAS,
	] {
		give(fake, condition);
	}

	// The game keeps the gas, as it might keep a condition for its minimum
	// duration.
	fake.answer(|player, method, arguments| {
		if method == c"RemoveCondEx" && arguments[0] == Value::Int(Condition::GAS.to_raw()) {
			return Some(Value::Void);
		}

		game(player, method, arguments)
	});

	assert_eq!(
		conditions.cleanse(ConditionBits::DEBUFFS),
		Ok(ConditionBits::of(&[Condition::BURNING, Condition::URINE]))
	);
	assert_eq!(
		conditions.bits(),
		Ok(ConditionBits::of(&[Condition::ZOOMED, Condition::GAS]))
	);

	let removals = fake
		.take_calls()
		.into_iter()
		.filter(|(method, _)| *method == c"RemoveCondEx")
		.map(|(_, arguments)| arguments)
		.collect::<Vec<_>>();

	assert_eq!(
		removals,
		[Condition::BURNING, Condition::URINE, Condition::GAS]
			.map(|condition| vec![Value::Int(condition.to_raw()), Value::Bool(true)])
	);

	conditions.remove_all().unwrap();
	assert_eq!(conditions.bits(), Ok(ConditionBits::EMPTY));
}

#[test]
fn conditions_cover_every_identifier_in_order() {
	assert_eq!(Condition::all().len(), sys::ETFCond_TF_COND_LAST as usize);

	for (raw, condition) in Condition::all().enumerate() {
		assert_eq!(condition.to_raw() as usize, raw);
		assert_eq!(Condition::from_raw(condition.to_raw()), Some(condition));
	}

	assert_eq!(
		Condition::all().next_back().map(Condition::to_raw),
		Some(sys::ETFCond_TF_COND_LAST - 1)
	);
	assert_eq!(Condition::from_raw(-1), None);
	assert_eq!(Condition::from_raw(sys::ETFCond_TF_COND_LAST), None);
}

#[test]
fn conditions_have_the_games_names() {
	assert_eq!(Condition::AIMING.name(), "TF_COND_AIMING");
	assert_eq!(Condition::URINE.name(), "TF_COND_URINE");
	assert_eq!(
		Condition::INVULNERABLE_HIDE_UNLESS_DAMAGED.name(),
		"TF_COND_INVULNERABLE_HIDE_UNLESS_DAMAGED"
	);

	let names: std::collections::HashSet<_> = Condition::all().map(Condition::name).collect();

	assert_eq!(names.len(), Condition::all().len());
	assert!(names.iter().all(|name| name.starts_with("TF_COND_")));

	for condition in Condition::all() {
		assert_eq!(Condition::from_name(condition.name()), Some(condition));
	}

	// As the game's lookup, case is ignored.
	assert_eq!(
		Condition::from_name("tf_cond_urine"),
		Some(Condition::URINE)
	);
	assert_eq!(Condition::from_name("TF_COND_LAST"), None);
	assert_eq!(Condition::from_name("URINE"), None);
}

#[test]
fn durations_are_set_only_for_held_conditions_but_crit_boosts() {
	let scope = ();
	let fake = player(VARS);
	let conditions = PlayerConditions::new(mock_server(&scope), fake.entity()).unwrap();
	let seconds = ConditionDuration::seconds(2.5).unwrap();
	let urine = Value::Int(Condition::URINE.to_raw());

	assert_eq!(
		conditions.set_duration(Condition::CRITBOOSTED, seconds),
		Ok(false)
	);
	assert!(fake.take_calls().is_empty());

	assert_eq!(
		conditions.set_duration(Condition::URINE, seconds),
		Ok(false)
	);
	assert_eq!(fake.take_calls(), [(c"InCond", vec![urine.clone()])]);

	give(fake, Condition::URINE);
	assert_eq!(
		conditions.set_duration(Condition::URINE, ConditionDuration::PERMANENT),
		Ok(true)
	);
	assert_eq!(
		fake.take_calls(),
		[
			(c"InCond", vec![urine.clone()]),
			(c"SetCondDuration", vec![urine, Value::Float(-1.0)]),
		]
	);
}

/// The game's condition methods, on the fake player's bits. Its predicates
/// answer false.
fn game(player: &FakePlayer, method: &CStr, arguments: &[Value]) -> Option<Value> {
	let condition = || match arguments.first() {
		Some(&Value::Int(raw)) => Condition::from_raw(raw).unwrap(),
		other => panic!("no condition: {other:?}"),
	};

	Some(match method.to_bytes() {
		b"AddCondEx" => {
			give(player, condition());
			Value::Void
		}

		b"InCond" => Value::Bool(has(player, condition())),

		b"RemoveAllCond" => {
			for name in WORD_NAMES {
				player.set(name, 0);
			}

			Value::Void
		}

		b"RemoveCondEx" => {
			take(player, condition());
			Value::Void
		}

		b"SetCondDuration" => Value::Void,
		_ => Value::Bool(false),
	})
}

/// Gives the fake player `condition`, as the game sets its bit.
fn give(player: &FakePlayer, condition: Condition) {
	let (name, bit) = word_of(condition);

	player.set(name, player.get(name) | bit);
}

/// Whether the fake player has `condition`, its bit set.
fn has(player: &FakePlayer, condition: Condition) -> bool {
	let (name, bit) = word_of(condition);

	player.get(name) & bit != 0
}

#[test]
fn only_tf2_players_are_wrapped() {
	let scope = ();
	let fake = player(VARS);
	let medigun = FakePlayer::other(
		c"tf_weapon_medigun",
		c"CWeaponMedigun",
		c"DT_WeaponMedigun",
		&[],
	);

	assert_eq!(
		PlayerConditions::new(null_server(Game::SourceSdk2013, &scope), fake.entity()).err(),
		Some(ConditionError::NotTfPlayer)
	);
	assert_eq!(
		PlayerConditions::new(mock_server(&scope), medigun.entity()).err(),
		Some(ConditionError::NotTfPlayer)
	);
}

/// A fake player whose condition methods act on its bits, which `vars`
/// declare.
fn player(vars: &[Var]) -> &'static FakePlayer {
	let player = FakePlayer::new(METHODS, vars);

	player.answer(game);
	player
}

#[test]
fn predicates_ask_the_game() {
	let server = mock_server(&());
	let fake = player(VARS);
	let conditions = PlayerConditions::new(server, fake.entity()).unwrap();
	let others: [fn(PlayerConditions<'static>) -> Result<bool, ConditionError>; 7] = [
		PlayerConditions::can_be_debuffed,
		PlayerConditions::is_control_stunned,
		PlayerConditions::is_crit_boosted,
		PlayerConditions::is_fully_invisible,
		PlayerConditions::is_immune_to_pushback,
		PlayerConditions::is_invulnerable,
		PlayerConditions::is_stealthed,
	];

	fake.answer(|_, method, _| Some(Value::Bool(method == c"IsSnared")));
	assert_eq!(conditions.is_snared(), Ok(true));

	for predicate in others {
		assert_eq!(predicate(conditions), Ok(false));
	}

	assert_eq!(
		fake.take_calls()
			.into_iter()
			.map(|(method, arguments)| {
				assert!(arguments.is_empty());
				method
			})
			.collect::<Vec<_>>(),
		[
			c"IsSnared",
			c"CanBeDebuffed",
			c"IsControlStunned",
			c"IsCritBoosted",
			c"IsFullyInvisible",
			c"IsImmuneToPushback",
			c"IsInvulnerable",
			c"IsStealthed",
		]
	);

	fake.answer(|_, _, _| None);
	assert_eq!(conditions.is_snared(), Err(ConditionError::Rejected));

	let bare = FakePlayer::new(&[], VARS);

	assert_eq!(
		PlayerConditions::new(server, bare.entity())
			.unwrap()
			.is_snared(),
		Err(ConditionError::UnsupportedMethod)
	);
}

#[test]
fn providers_are_passed_as_their_script_instances() {
	let scope = ();
	let server = mock_server(&scope);
	let fake = player(VARS);
	let medic = player(VARS);
	let conditions = PlayerConditions::new(server, fake.entity()).unwrap();
	let seconds = ConditionDuration::seconds(3.0).unwrap();
	let urine = Value::Int(Condition::URINE.to_raw());

	assert_eq!(
		conditions.add_with_provider(Condition::URINE, seconds, medic.entity()),
		Ok(true)
	);
	assert!(!medic.instance().is_null());
	assert_eq!(medic.take_calls(), [(c"ValidateScriptScope", vec![])]);
	assert_eq!(
		fake.take_calls(),
		[
			(
				c"AddCondEx",
				vec![
					urine.clone(),
					Value::Float(3.0),
					Value::Handle(medic.instance())
				]
			),
			(c"InCond", vec![urine]),
		]
	);

	// Without a provider, the game is passed none.
	assert_eq!(
		conditions.add(Condition::ZOOMED, ConditionDuration::PERMANENT),
		Ok(true)
	);
	assert_eq!(
		fake.take_calls()[0],
		(
			c"AddCondEx",
			vec![
				Value::Int(Condition::ZOOMED.to_raw()),
				Value::Float(-1.0),
				Value::Handle(null_mut()),
			]
		)
	);

	// A provider marked for deletion has given its instance up.
	let leaving = player(VARS);

	leaving.set_flags(EFL_KILLME);
	assert_eq!(
		conditions.add_with_provider(Condition::URINE, seconds, leaving.entity()),
		Err(ConditionError::Provider(
			ScriptInstanceError::MarkedForDeletion
		))
	);
	assert!(fake.take_calls().is_empty());
}

/// Takes `condition` from the fake player, as the game clears its bit.
fn take(player: &FakePlayer, condition: Condition) {
	let (name, bit) = word_of(condition);

	player.set(name, player.get(name) & !bit);
}

#[test]
fn the_watcher_reports_changes_per_player() {
	serve_interfaces();

	let scope = ();
	let server = mock_server(&scope);
	let first = player(VARS);
	let second = player(VARS);
	let mut watcher = ConditionWatcher::new();
	let mut watch = |fake: &'static FakePlayer| {
		watcher
			.update(PlayerConditions::new(server, fake.entity()).unwrap())
			.unwrap()
	};

	give(first, Condition::ZOOMED);
	assert_eq!(
		watch(first),
		ConditionChanges {
			added: ConditionBits::of(&[Condition::ZOOMED]),
			removed: ConditionBits::EMPTY,
		}
	);
	assert!(watch(second).is_empty());
	assert!(watch(first).is_empty());

	take(first, Condition::ZOOMED);
	give(first, Condition::URINE);
	give(second, Condition::BURNING);
	assert_eq!(
		watch(first),
		ConditionChanges {
			added: ConditionBits::of(&[Condition::URINE]),
			removed: ConditionBits::of(&[Condition::ZOOMED]),
		}
	);
	assert_eq!(
		watch(second).added,
		ConditionBits::of(&[Condition::BURNING])
	);

	watcher.forget(first.entity());
	assert_eq!(watcher.previous.len(), 1);

	let mut watch = |fake: &'static FakePlayer| {
		watcher
			.update(PlayerConditions::new(server, fake.entity()).unwrap())
			.unwrap()
	};

	assert_eq!(watch(first).added, ConditionBits::of(&[Condition::URINE]));

	watcher.clear();
	assert!(watcher.previous.is_empty());
}

/// The variable and bit that hold `condition`.
fn word_of(condition: Condition) -> (&'static CStr, u32) {
	let raw = condition.to_raw() as usize;

	(WORD_NAMES[raw / 32], 1 << (raw % 32))
}
