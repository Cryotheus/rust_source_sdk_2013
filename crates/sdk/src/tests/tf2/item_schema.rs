//! Tests of the item schema's numbers.

use super::*;

#[test]
fn levels_stay_within_what_clients_receive() {
	assert_eq!(ItemLevel::default(), ItemLevel::DEFAULT);
	assert_eq!(ItemLevel::DEFAULT.get(), 1);
	assert_eq!(ItemLevel::new(0).map(ItemLevel::get), Some(0));
	assert_eq!(ItemLevel::new(127), Some(ItemLevel::MAX));
	assert_eq!(ItemLevel::new(128), None);
	assert_eq!(ItemLevel::new(u8::MAX), None);
}

#[test]
fn qualities_keep_tf2s_numbers() {
	let numbers = [0, 1, 3, 5, 6, 7, 8, 9, 11, 13, 14, 15];

	for (quality, number) in ItemQuality::ALL.into_iter().zip(numbers) {
		assert_eq!(quality.to_raw(), number);
		assert_eq!(ItemQuality::from_raw(number), Some(quality));
	}

	// The unused qualities, the rarity grades, and the ones beyond what
	// clients receive are left out.
	for number in [-1, 2, 4, 10, 12, 16, 22, 23, 9999] {
		assert_eq!(ItemQuality::from_raw(number), None);
	}

	assert!(
		ItemQuality::ALL
			.iter()
			.all(|quality| (-16..16).contains(&quality.to_raw()))
	);
}

#[test]
fn loadout_positions_keep_tf2s_numbers() {
	let numbers = [
		sys::loadout_positions_t_LOADOUT_POSITION_PRIMARY,
		sys::loadout_positions_t_LOADOUT_POSITION_SECONDARY,
		sys::loadout_positions_t_LOADOUT_POSITION_MELEE,
		sys::loadout_positions_t_LOADOUT_POSITION_UTILITY,
		sys::loadout_positions_t_LOADOUT_POSITION_BUILDING,
		sys::loadout_positions_t_LOADOUT_POSITION_PDA,
		sys::loadout_positions_t_LOADOUT_POSITION_PDA2,
		sys::loadout_positions_t_LOADOUT_POSITION_HEAD,
		sys::loadout_positions_t_LOADOUT_POSITION_MISC,
		sys::loadout_positions_t_LOADOUT_POSITION_ACTION,
		sys::loadout_positions_t_LOADOUT_POSITION_MISC2,
		sys::loadout_positions_t_LOADOUT_POSITION_TAUNT,
		sys::loadout_positions_t_LOADOUT_POSITION_TAUNT2,
		sys::loadout_positions_t_LOADOUT_POSITION_TAUNT3,
		sys::loadout_positions_t_LOADOUT_POSITION_TAUNT4,
		sys::loadout_positions_t_LOADOUT_POSITION_TAUNT5,
		sys::loadout_positions_t_LOADOUT_POSITION_TAUNT6,
		sys::loadout_positions_t_LOADOUT_POSITION_TAUNT7,
		sys::loadout_positions_t_LOADOUT_POSITION_TAUNT8,
	];

	for (position, number) in LoadoutPosition::ALL.into_iter().zip(numbers) {
		assert_eq!(position.to_raw(), number);
		assert_eq!(LoadoutPosition::from_raw(number), Some(position));
	}

	assert_eq!(
		LoadoutPosition::ALL.len() as c_int,
		sys::loadout_positions_t_CLASS_LOADOUT_POSITION_COUNT
	);

	for number in [sys::loadout_positions_t_LOADOUT_POSITION_INVALID, 19] {
		assert_eq!(LoadoutPosition::from_raw(number), None);
	}
}

#[test]
fn shared_item_classes_are_translated_for_each_class() {
	let class = |item_class: &'static CStr, class| class_item_class(item_class, class);

	assert_eq!(
		class(c"tf_weapon_shotgun", PlayerClass::Soldier),
		Some(c"tf_weapon_shotgun_soldier")
	);
	assert_eq!(
		class(c"TF_WEAPON_SHOTGUN", PlayerClass::Engineer),
		Some(c"tf_weapon_shotgun_primary")
	);
	assert_eq!(class(c"tf_weapon_shotgun", PlayerClass::Scout), None);
	assert_eq!(
		class(c"saxxy", PlayerClass::Heavy),
		Some(c"tf_weapon_fireaxe")
	);
	assert_eq!(
		class(c"tf_weapon_parachute", PlayerClass::Demoman),
		Some(c"tf_weapon_parachute_primary")
	);
	assert_eq!(
		class(c"tf_weapon_revolver", PlayerClass::Engineer),
		Some(c"tf_weapon_revolver_secondary")
	);
	assert_eq!(
		class(c"tf_weapon_rocketlauncher", PlayerClass::Pyro),
		Some(c"tf_weapon_rocketlauncher")
	);

	// The table is lowercase, as the game's is, and names a weapon class for
	// at least one player class of each item class it translates.
	for (generic, classes) in CLASS_ITEM_CLASSES {
		assert!(
			generic
				.to_bytes()
				.iter()
				.all(|&byte| byte.is_ascii_lowercase() || byte == b'_')
		);
		assert!(classes.iter().any(Option::is_some));

		for translated in classes.into_iter().flatten() {
			assert!(translated.to_bytes().starts_with(b"tf_weapon_"));
		}
	}
}
