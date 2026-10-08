//! Tests of TF2 player meters and mediguns through the networked variables of
//! fake entities.

use super::*;
use crate::datatables::NetPropError;
use crate::test_support::server::{mock_server, null_server};
use crate::test_support::tf2::player::{FakePlayer, Var, VarKind, serve_interfaces};

/// The networked variables of fake mediguns.
const MEDIGUN_VARS: &[Var] = &[
	Var::new(c"m_bAttacking", VarKind::Bool),
	Var::new(c"m_bChargeRelease", VarKind::Bool),
	Var::new(c"m_bHealing", VarKind::Bool),
	Var::new(c"m_bHolstered", VarKind::Bool),
	Var::new(c"m_flChargeLevel", VarKind::Float),
	Var::new(c"m_hHealingTarget", VarKind::Handle),
	Var::new(c"m_hLastHealingTarget", VarKind::Handle),
	Var::new(c"m_nChargeResistType", VarKind::Int),
];

/// The meters' networked variables of fake players.
const VARS: &[Var] = &[
	Var::new(c"m_bRageDraining", VarKind::Bool),
	Var::new(c"m_flChargeMeter", VarKind::Float),
	Var::new(c"m_flCloakMeter", VarKind::Float),
	Var::new(c"m_flEnergyDrinkMeter", VarKind::Float),
	Var::new(c"m_flHypeMeter", VarKind::Float),
	Var::new(c"m_flItemChargeMeter", VarKind::Floats(3)),
	Var::new(c"m_flRageMeter", VarKind::Float),
	Var::new(c"m_iDecapitations", VarKind::Int),
	Var::new(c"m_iRevengeCrits", VarKind::Int),
	Var::new(c"m_nNumHealers", VarKind::Int),
];

#[test]
fn item_charges_are_elements_of_their_array() {
	let (fake, meters) = player();

	fake.set_float(c"m_flItemChargeMeter", 2, 30.0);
	meters.set_item_charge(1, 50.0).unwrap();
	assert!(fake.take_changed());
	assert_eq!(
		[0, 1, 2].map(|slot| fake.float(c"m_flItemChargeMeter", slot)),
		[0.0, 50.0, 30.0]
	);
	assert_eq!(meters.item_charge(2), Ok(30.0));
	assert!(matches!(
		meters.item_charge(3),
		Err(EffectError::NetProp(NetPropError::ElementOutOfRange {
			index: 3,
			len: 3,
			..
		}))
	));
	assert_eq!(
		meters.set_item_charge(0, 100.5),
		Err(EffectError::OutOfRange("charge"))
	);
	assert!(!fake.take_changed());
}

#[test]
fn mediguns_read_their_variables() {
	serve_interfaces();

	let scope = ();
	let server = mock_server(&scope);
	let weapon = FakePlayer::other(
		c"tf_weapon_medigun",
		c"CWeaponMedigun",
		c"DT_WeaponMedigun",
		MEDIGUN_VARS,
	);
	let (patient, _) = player();
	let medigun = Medigun::new(server, weapon.entity()).unwrap();

	assert_eq!(
		Medigun::new(server, patient.entity()).err(),
		Some(EffectError::NotTfPlayer)
	);
	assert_eq!(
		Medigun::new(null_server(Game::SourceSdk2013, &scope), weapon.entity()).err(),
		Some(EffectError::NotTfPlayer)
	);

	weapon.set(c"m_hHealingTarget", patient.handle().to_raw());
	weapon.set(c"m_hLastHealingTarget", EntityHandle::INVALID.to_raw());
	weapon.set(c"m_bHealing", 1);
	weapon.set(c"m_bChargeRelease", 1);
	assert_eq!(medigun.healing_target(), Ok(Some(patient.handle())));
	assert_eq!(medigun.last_healing_target(), Ok(None));
	assert_eq!(
		[
			medigun.is_attacking(),
			medigun.is_healing(),
			medigun.is_holstered(),
			medigun.is_releasing(),
		],
		[Ok(false), Ok(true), Ok(false), Ok(true)]
	);

	medigun.set_charge(0.75).unwrap();
	assert!(weapon.take_changed());
	assert_eq!(medigun.charge(), Ok(0.75));
	assert_eq!(
		medigun.set_charge(1.5),
		Err(EffectError::OutOfRange("charge"))
	);
	assert_eq!(medigun.charge(), Ok(0.75));

	for resistance in [
		MedigunResistance::Bullet,
		MedigunResistance::Blast,
		MedigunResistance::Fire,
	] {
		weapon.set(c"m_nChargeResistType", resistance.to_raw().cast_unsigned());
		assert_eq!(medigun.resistance(), Ok(Some(resistance)));
	}

	weapon.set(c"m_nChargeResistType", 3);
	assert_eq!(medigun.resistance(), Ok(None));
}

#[test]
fn meters_read_and_write_the_players_variables() {
	type Setter = fn(PlayerMeters<'static>, f32) -> Result<(), EffectError>;

	let (fake, meters) = player();
	let setters: [(Setter, &CStr); 5] = [
		(PlayerMeters::set_cloak, c"m_flCloakMeter"),
		(PlayerMeters::set_energy_drink, c"m_flEnergyDrinkMeter"),
		(PlayerMeters::set_hype, c"m_flHypeMeter"),
		(PlayerMeters::set_rage, c"m_flRageMeter"),
		(PlayerMeters::set_shield_charge, c"m_flChargeMeter"),
	];

	for (index, (set, name)) in setters.into_iter().enumerate() {
		let value = 10.0 * index as f32 + 0.5;

		set(meters, value).unwrap();
		assert!(fake.take_changed());
		assert_eq!(fake.float(name, 0), value);
	}

	assert_eq!(
		[
			meters.cloak(),
			meters.energy_drink(),
			meters.hype(),
			meters.rage(),
			meters.shield_charge(),
		],
		[Ok(0.5), Ok(10.5), Ok(20.5), Ok(30.5), Ok(40.5)]
	);

	meters.set_decapitations(5).unwrap();
	assert_eq!(fake.get(c"m_iDecapitations"), 5);
	assert_eq!(meters.decapitations(), Ok(5));

	fake.set(c"m_nNumHealers", 2);
	fake.set(c"m_iRevengeCrits", 3);
	fake.set(c"m_bRageDraining", 1);
	assert_eq!(meters.healers(), Ok(2));
	assert_eq!(meters.revenge_crits(), Ok(3));
	assert_eq!(meters.is_rage_draining(), Ok(true));
}

#[test]
fn meters_refuse_values_outside_their_range() {
	let (fake, meters) = player();

	for value in [-1.0, 100.5, f32::NAN, f32::INFINITY] {
		assert_eq!(
			meters.set_cloak(value),
			Err(EffectError::OutOfRange("value"))
		);
	}

	for heads in [-1, 256] {
		assert_eq!(
			meters.set_decapitations(heads),
			Err(EffectError::OutOfRange("value"))
		);
	}

	assert!(!fake.take_changed());
	meters.set_cloak(100.0).unwrap();
	meters.set_decapitations(255).unwrap();
	assert_eq!(
		(meters.cloak(), meters.decapitations()),
		(Ok(100.0), Ok(255))
	);

	let medigun = FakePlayer::other(
		c"tf_weapon_medigun",
		c"CWeaponMedigun",
		c"DT_WeaponMedigun",
		&[],
	);

	assert_eq!(
		PlayerMeters::new(mock_server(&()), medigun.entity()).err(),
		Some(EffectError::NotTfPlayer)
	);
}

/// A fake player with the meters' variables, and its meters, on a mock
/// server that serves them.
fn player() -> (&'static FakePlayer, PlayerMeters<'static>) {
	serve_interfaces();

	let fake = FakePlayer::new(&[], VARS);

	(
		fake,
		PlayerMeters::new(mock_server(&()), fake.entity()).unwrap(),
	)
}
