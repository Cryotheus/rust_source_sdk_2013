//! Tests of the item schema's numbers.

use super::*;

#[test]
fn copied_equip_masks_keep_high_bits() {
	let high = EquipRegions::from_masks(1 << 31, 1 << 31);
	let low = EquipRegions::from_masks(1, 1);

	assert!(high.is_compatible_with(low));
	assert!(!high.is_compatible_with(high));
	assert_eq!(high.regions(), 1 << 31);
	assert_eq!(high.conflicts(), 1 << 31);
}

#[test]
fn equip_regions_check_both_directions_and_all_occupied_regions() {
	let hat = EquipRegions::from_masks(0b0010, 0b0011);
	let whole_head = EquipRegions::from_masks(0b0001, 0b0001);
	let shirt = EquipRegions::from_masks(0b0100, 0b0100);
	let hat_and_shirt = EquipRegions::from_masks(0b0110, 0b0111);

	assert!(!hat.is_compatible_with(whole_head));
	assert!(!whole_head.is_compatible_with(hat));
	assert!(hat.is_compatible_with(shirt));
	assert!(shirt.is_compatible_with(hat));
	assert!(!hat_and_shirt.is_compatible_with(shirt));
	assert!(!shirt.is_compatible_with(hat_and_shirt));
	assert!(!hat.is_compatible_with(hat));
}

#[test]
fn item_type_tokens_preserve_native_null_empty_and_untranslated_values() {
	let mut raw = std::mem::MaybeUninit::<sys::CEconItemDefinition>::uninit();
	let definition = ItemDefinition::new(NonNull::new(raw.as_mut_ptr()).unwrap());

	for token in [
		None,
		Some(c""),
		Some(c"#TF_Wearable_TournamentMedal"),
		Some(c"#TF_Wearable_Badge"),
	] {
		// SAFETY: This fixture only reads the item-type pointer, which is
		// initialized here. Every non-null pointer names a static C string,
		// and the definition's allocation lives through each read.
		unsafe {
			(&raw mut (*raw.as_mut_ptr()).m_pszItemTypeName)
				.write(token.map_or(std::ptr::null(), CStr::as_ptr));
		}
		assert_eq!(
			checked_item_type(Some(true), || unsafe { definition.read_item_type() }),
			Ok(token),
		);
	}
}

#[test]
fn schema_items_with_zero_masks_have_no_region_restrictions() {
	let unrestricted = EquipRegions::from_masks(0, 0);
	let restricted = EquipRegions::from_masks(u32::MAX, u32::MAX);

	assert_eq!(unrestricted.regions(), 0);
	assert_eq!(unrestricted.conflicts(), 0);
	assert!(unrestricted.is_compatible_with(restricted));
	assert!(restricted.is_compatible_with(unrestricted));
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

#[test]
fn unavailable_equip_mask_layout_fails_without_reading_native_memory() {
	for checked in [None, Some(false)] {
		assert_eq!(
			checked_equip_regions(checked, || panic!("an unchecked layout must not be read")),
			Err(ItemSchemaError::UnsupportedLayout),
		);
	}

	assert_eq!(
		checked_equip_regions(Some(true), || EquipRegions::from_masks(0, 0)),
		Ok(EquipRegions::from_masks(0, 0)),
	);
}

#[test]
fn unavailable_item_type_layout_fails_without_reading_native_memory() {
	for checked in [None, Some(false)] {
		assert_eq!(
			checked_item_type(checked, || panic!(
				"an unchecked item-type layout must not be read"
			)),
			Err(ItemSchemaError::UnsupportedLayout),
		);
	}
}
