//! Tests of `crate::tf2::class_targets`: the classes of mock entities, by the
//! data description maps their classes chain.

use super::*;
use crate::test_support::entities::{MockEntity, base_entity_fields, set_datamap};
use crate::test_support::server::{mock_server, null_server};
use sdk_raw::test_support::entities::data_map;
use std::ptr::null_mut;

/// Has mock entities report the maps of a TF2 weapon's class.
fn chain_weapon_maps() {
	let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());
	let combat = data_map(c"CBaseCombatWeapon", vec![], base);

	set_datamap(data_map(c"CTFWeaponBase", vec![], combat));
}

#[test]
fn class_targets_need_tf2() {
	let scope = ();

	assert!(matches!(
		ClassTargets::load(null_server(Game::SourceSdk2013, &scope)),
		Err(ClassTargetError::WrongGame)
	));
}

#[test]
fn entities_have_the_classes_of_the_kinds_their_maps_chain() {
	let scope = ();
	let server = mock_server(&scope);
	let mut weapon = MockEntity::new(1);
	chain_weapon_maps();
	let entity = weapon.entity();
	let target = ClassTarget::<TfWeapon>::of(server, entity).unwrap();

	// SAFETY: A mock entity starts with its vtable pointer.
	let vtable = unsafe { vtable_pointer::<*mut c_void>(entity.as_ptr()) };

	assert_eq!(target.as_ptr().as_ptr(), vtable.cast_mut());
	assert!(target.is_class_of(entity));
	assert_eq!(
		ClassTarget::<CombatWeapon>::of(server, entity),
		Some(target.upcast())
	);
	assert_eq!(
		ClassTarget::<BaseEntity>::of(server, entity),
		Some(target.upcast())
	);

	// Maps not in the chain are other kinds'.
	assert_eq!(ClassTarget::<TfPlayer>::of(server, entity), None);
	assert_eq!(ClassTarget::<CombatCharacter>::of(server, entity), None);
	assert_eq!(ClassTarget::<TfMeleeWeapon>::of(server, entity), None);
}

#[test]
fn melee_weapons_are_weapons() {
	let scope = ();
	let server = mock_server(&scope);
	let mut weapon = MockEntity::new(1);
	let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());
	let combat = data_map(c"CBaseCombatWeapon", vec![], base);
	let tf = data_map(c"CTFWeaponBase", vec![], combat);

	set_datamap(data_map(c"CTFWeaponBaseMelee", vec![], tf));

	let entity = weapon.entity();
	let target = ClassTarget::<TfMeleeWeapon>::of(server, entity).unwrap();

	assert_eq!(
		ClassTarget::<TfWeapon>::of(server, entity),
		Some(target.upcast())
	);
	assert_eq!(
		ClassTarget::<CombatWeapon>::of(server, entity),
		Some(target.upcast())
	);
}

#[test]
fn entities_of_other_games_have_no_classes() {
	let scope = ();
	let server = null_server(Game::SourceSdk2013, &scope);
	let mut weapon = MockEntity::new(1);
	chain_weapon_maps();

	assert_eq!(ClassTarget::<BaseEntity>::of(server, weapon.entity()), None);
}

#[test]
fn objects_are_the_building_classes() {
	use crate::tf2::buildings::BuildingClass;

	assert_eq!(OBJECT_CLASSES, BuildingClass::ALL.map(BuildingClass::name));
}
