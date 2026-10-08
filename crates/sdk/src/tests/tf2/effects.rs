//! Tests of TF2 player effects through a fake player's native methods and
//! networked variables.

use super::*;
use crate::Game;
use crate::test_support::server::{mock_server, null_server};
use crate::test_support::tf2::player::{FakePlayer, Method, Value, Var, VarKind, serve_interfaces};
use crate::tf2::conditions::ConditionBits;
use sdk_raw::entities::EFL_KILLME;
use sdk_raw::tf2::script_binding::{BOOL, FLOAT, HANDLE, INT, VOID};
use std::ptr::null_mut;

/// The native effect methods of fake players.
const METHODS: &[Method] = &[
	Method::new(c"BleedPlayerEx", VOID, &[FLOAT, INT, BOOL, INT]),
	Method::new(c"CanAirDash", BOOL, &[]),
	Method::new(c"ClearSpells", VOID, &[]),
	Method::new(c"DropRune", VOID, &[BOOL, INT]),
	Method::new(c"ExtinguishPlayerBurning", VOID, &[]),
	Method::new(c"GetDisguiseAmmoCount", INT, &[]),
	Method::new(c"IgnitePlayer", VOID, &[]),
	Method::new(c"InAirDueToExplosion", BOOL, &[]),
	Method::new(c"InAirDueToKnockback", BOOL, &[]),
	Method::new(c"InCond", BOOL, &[INT]),
	Method::new(c"IsAirDashing", BOOL, &[]),
	Method::new(c"IsCarryingRune", BOOL, &[]),
	Method::new(c"IsJumping", BOOL, &[]),
	Method::new(c"IsParachuteEquipped", BOOL, &[]),
	Method::new(c"RemoveDisguise", VOID, &[]),
	Method::new(c"RemoveInvisibility", VOID, &[]),
	Method::new(c"RollRareSpell", VOID, &[]),
	Method::new(c"SetDisguiseAmmoCount", VOID, &[INT]),
	Method::new(c"StunPlayer", VOID, &[FLOAT, FLOAT, INT, HANDLE]),
];

/// The effects' networked variables of fake players.
const VARS: &[Var] = &[
	Var::new(c"m_flMovementStunTime", VarKind::Float),
	Var::new(c"m_flRuneCharge", VarKind::Float),
	Var::new(c"m_hDisguiseTarget", VarKind::Handle),
	Var::new(c"m_hStunner", VarKind::Handle),
	Var::new(c"m_iDisguiseHealth", VarKind::Int),
	Var::new(c"m_iMovementStunAmount", VarKind::Int),
	Var::new(c"m_iStunFlags", VarKind::Int),
	Var::new(c"m_nDisguiseClass", VarKind::Int),
	Var::new(c"m_nDisguiseTeam", VarKind::Int),
];

#[test]
fn bleeds_and_disguise_ammo_are_checked_before_the_call() {
	let (fake, effects) = player();

	assert_eq!(
		effects.bleed(-1.0, 4, false),
		Err(EffectError::OutOfRange("duration"))
	);
	assert_eq!(
		effects.bleed(f32::INFINITY, 4, false),
		Err(EffectError::OutOfRange("duration"))
	);
	assert_eq!(
		effects.bleed(5.0, -1, false),
		Err(EffectError::OutOfRange("damage"))
	);
	assert_eq!(
		effects.set_disguise_ammo(-1),
		Err(EffectError::OutOfRange("ammo"))
	);
	assert!(fake.take_calls().is_empty());

	effects.bleed(5.0, 4, true).unwrap();
	effects.set_disguise_ammo(12).unwrap();
	assert_eq!(effects.disguise_ammo(), Ok(7));
	assert_eq!(
		fake.take_calls(),
		[
			(
				c"BleedPlayerEx",
				vec![
					Value::Float(5.0),
					Value::Int(4),
					Value::Bool(true),
					Value::Int(sys::ETFDmgCustom_TF_DMG_CUSTOM_BLEEDING as c_int),
				]
			),
			(c"SetDisguiseAmmoCount", vec![Value::Int(12)]),
			(c"GetDisguiseAmmoCount", vec![]),
		]
	);
}

#[test]
fn disguises_are_read_while_disguised() {
	serve_interfaces();

	let (fake, effects) = player();
	let target = FakePlayer::new(&[], &[]);

	assert_eq!(effects.disguise(), Ok(None));

	fake.answer(game(ConditionBits::of(&[Condition::DISGUISED])));
	fake.set(
		c"m_nDisguiseClass",
		PlayerClass::Engineer.to_raw().cast_unsigned(),
	);
	fake.set(c"m_iDisguiseHealth", 125);
	fake.set(c"m_hDisguiseTarget", target.handle().to_raw());
	fake.set(c"m_nDisguiseTeam", 3);
	assert_eq!(
		effects.disguise(),
		Ok(Some(Disguise {
			class: Some(PlayerClass::Engineer),
			health: 125,
			target: Some(target.handle()),
			team: 3,
		}))
	);

	fake.set(c"m_nDisguiseClass", 12);
	fake.set(c"m_hDisguiseTarget", EntityHandle::INVALID.to_raw());
	assert_eq!(
		effects
			.disguise()
			.unwrap()
			.map(|disguise| (disguise.class, disguise.target)),
		Some((None, None))
	);
}

#[test]
fn effects_need_a_tf2_player_and_its_methods_and_variables() {
	let scope = ();
	let (fake, effects) = player();
	let medigun = FakePlayer::other(
		c"tf_weapon_medigun",
		c"CWeaponMedigun",
		c"DT_WeaponMedigun",
		&[],
	);

	assert_eq!(
		PlayerEffects::new(null_server(Game::SourceSdk2013, &scope), fake.entity()).err(),
		Some(EffectError::NotTfPlayer)
	);
	assert_eq!(
		PlayerEffects::new(mock_server(&scope), medigun.entity()).err(),
		Some(EffectError::NotTfPlayer)
	);

	// Without the game DLL, no variable can be found.
	assert!(matches!(
		effects.rune_charge(),
		Err(EffectError::Interface(_))
	));

	fake.answer(|_, _, _| None);
	assert_eq!(effects.ignite(), Err(EffectError::Rejected));

	let bare = FakePlayer::new(&[], &[]);
	let bare_effects = PlayerEffects::new(mock_server(&scope), bare.entity()).unwrap();

	assert_eq!(bare_effects.ignite(), Err(EffectError::UnsupportedMethod));

	serve_interfaces();
	assert!(matches!(
		bare_effects.rune_charge(),
		Err(EffectError::NetProp(NetPropError::NotFound { .. }))
	));
}

/// The game's methods for a player with the conditions `held`, who carries no
/// powerup, and whose disguise weapon shows 7 rounds. Other predicates answer
/// false.
fn game(held: ConditionBits) -> impl FnMut(&FakePlayer, &CStr, &[Value]) -> Option<Value> {
	move |_, method, arguments| {
		Some(match (method.to_bytes(), arguments) {
			(b"GetDisguiseAmmoCount", []) => Value::Int(7),

			(b"InCond", &[Value::Int(raw)]) => {
				Value::Bool(held.contains(Condition::from_raw(raw).unwrap()))
			}

			// The answers of methods that return nothing are dropped.
			_ => Value::Bool(false),
		})
	}
}

/// A fake player with no conditions, and its effects, on a mock server.
fn player() -> (&'static FakePlayer, PlayerEffects<'static>) {
	let fake = FakePlayer::new(METHODS, VARS);

	fake.answer(game(ConditionBits::EMPTY));
	(
		fake,
		PlayerEffects::new(mock_server(&()), fake.entity()).unwrap(),
	)
}

#[test]
fn runes_are_dropped_only_when_carried() {
	serve_interfaces();

	let (fake, effects) = player();

	assert_eq!(effects.drop_rune(), Ok(false));
	assert_eq!(fake.take_calls(), [(c"IsCarryingRune", vec![])]);

	fake.answer(|_, method, _| Some(Value::Bool(method == c"IsCarryingRune")));
	assert_eq!(effects.drop_rune(), Ok(true));
	assert_eq!(
		fake.take_calls(),
		[
			(c"IsCarryingRune", vec![]),
			(c"DropRune", vec![Value::Bool(true), Value::Int(-1)]),
		]
	);

	fake.set_float(c"m_flRuneCharge", 0, 62.5);
	assert_eq!(effects.rune_charge(), Ok(62.5));
}

#[test]
fn simple_effects_and_predicates_call_their_methods() {
	let (fake, effects) = player();
	let actions: [fn(PlayerEffects<'static>) -> Result<(), EffectError>; 6] = [
		PlayerEffects::clear_spells,
		PlayerEffects::extinguish,
		PlayerEffects::ignite,
		PlayerEffects::remove_disguise,
		PlayerEffects::remove_invisibility,
		PlayerEffects::roll_rare_spell,
	];
	let predicates: [fn(PlayerEffects<'static>) -> Result<bool, EffectError>; 7] = [
		PlayerEffects::can_air_dash,
		PlayerEffects::in_air_due_to_explosion,
		PlayerEffects::in_air_due_to_knockback,
		PlayerEffects::is_air_dashing,
		PlayerEffects::is_carrying_rune,
		PlayerEffects::is_jumping,
		PlayerEffects::is_parachute_equipped,
	];

	for action in actions {
		action(effects).unwrap();
	}

	fake.answer(|_, method, _| Some(Value::Bool(method == c"IsJumping")));

	assert_eq!(
		predicates.map(|predicate| predicate(effects).unwrap()),
		[false, false, false, false, false, true, false]
	);
	assert_eq!(
		fake.take_calls()
			.into_iter()
			.map(|(method, arguments)| {
				assert!(arguments.is_empty());
				method
			})
			.collect::<Vec<_>>(),
		[
			c"ClearSpells",
			c"ExtinguishPlayerBurning",
			c"IgnitePlayer",
			c"RemoveDisguise",
			c"RemoveInvisibility",
			c"RollRareSpell",
			c"CanAirDash",
			c"InAirDueToExplosion",
			c"InAirDueToKnockback",
			c"IsAirDashing",
			c"IsCarryingRune",
			c"IsJumping",
			c"IsParachuteEquipped",
		]
	);
}

#[test]
fn stun_flags_keep_the_games_bits() {
	let named = StunFlags::all()
		.iter_names()
		.fold(StunFlags::empty(), |all, (_, flag)| all | flag);

	assert_eq!(
		named.bits(),
		0b1_1111_1111,
		"the game's nine TF_STUN_* flags"
	);
	assert_eq!(StunFlags::BOTH, StunFlags::MOVEMENT | StunFlags::CONTROLS);
	assert_eq!(StunFlags::from_bits_retain(1 << 12).bits(), 1 << 12);
}

#[test]
fn stuns_are_read_while_stunned() {
	serve_interfaces();

	let (fake, effects) = player();
	let stunner = FakePlayer::new(&[], &[]);
	let flags = StunFlags::CONTROLS | StunFlags::from_bits_retain(1 << 12);

	assert_eq!(effects.active_stun(), Ok(None));

	fake.answer(game(ConditionBits::of(&[Condition::STUNNED])));
	fake.set(c"m_iStunFlags", flags.bits().cast_unsigned());
	fake.set(c"m_iMovementStunAmount", 51);
	fake.set_float(c"m_flMovementStunTime", 0, 3.0);
	fake.set(c"m_hStunner", stunner.handle().to_raw());
	assert_eq!(
		effects.active_stun(),
		Ok(Some(ActiveStun {
			duration: 3.0,
			flags,
			slowdown: 0.2,
			stunner: Some(stunner.handle()),
		}))
	);

	fake.set(c"m_hStunner", EntityHandle::INVALID.to_raw());
	assert_eq!(
		effects.active_stun().unwrap().and_then(|stun| stun.stunner),
		None
	);
}

#[test]
fn stuns_pass_their_attacker_as_a_script_instance() {
	let (fake, effects) = player();
	let attacker = FakePlayer::new(&[], &[]);
	let flags = StunFlags::BOTH | StunFlags::SOUND;

	for (duration, slowdown, refused) in [
		(-1.0, 0.5, "duration"),
		(f32::NAN, 0.5, "duration"),
		(2.0, 1.5, "slowdown"),
		(2.0, -0.1, "slowdown"),
		(2.0, f32::NAN, "slowdown"),
	] {
		assert_eq!(
			effects.stun(duration, slowdown, flags, Some(attacker.entity())),
			Err(EffectError::OutOfRange(refused))
		);
	}

	assert!(fake.take_calls().is_empty());
	assert!(attacker.take_calls().is_empty());

	effects
		.stun(2.0, 0.5, flags, Some(attacker.entity()))
		.unwrap();
	effects.stun(0.0, 1.0, StunFlags::empty(), None).unwrap();
	assert!(!attacker.instance().is_null());
	assert_eq!(
		fake.take_calls(),
		[
			(
				c"StunPlayer",
				vec![
					Value::Float(2.0),
					Value::Float(0.5),
					Value::Int(flags.bits()),
					Value::Handle(attacker.instance()),
				]
			),
			(
				c"StunPlayer",
				vec![
					Value::Float(0.0),
					Value::Float(1.0),
					Value::Int(0),
					Value::Handle(null_mut()),
				]
			),
		]
	);

	// An attacker marked for deletion has given its instance up.
	let leaving = FakePlayer::new(&[], &[]);

	leaving.set_flags(EFL_KILLME);
	assert_eq!(
		effects.stun(2.0, 0.5, flags, Some(leaving.entity())),
		Err(EffectError::ScriptInstance(
			ScriptInstanceError::MarkedForDeletion
		))
	);
	assert!(fake.take_calls().is_empty());
}
