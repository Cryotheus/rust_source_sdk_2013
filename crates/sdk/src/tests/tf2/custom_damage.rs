//! Tests of TF2's custom damage kinds.

use super::*;

use crate::tf2::damage::{
	CUSTOM_DAMAGE_BACKSTAB, CUSTOM_DAMAGE_DECAPITATION, CUSTOM_DAMAGE_DECAPITATION_BOSS,
	CUSTOM_DAMAGE_HEADSHOT, CUSTOM_DAMAGE_HEADSHOT_DECAPITATION,
	CUSTOM_DAMAGE_MERASMUS_DECAPITATION, CUSTOM_DAMAGE_PLASMA, CUSTOM_DAMAGE_PLASMA_CHARGED,
	CUSTOM_DAMAGE_TAUNT_BARBARIAN_SWING, DamageInfo, DamageType,
};

#[test]
fn damage_carries_its_kind() {
	let mut info = DamageInfo::new(10.0, DamageType::empty());

	assert_eq!(info.custom_kind(), CustomDamage::NONE);

	info.set_custom_kind(CustomDamage::HEADSHOT);
	assert_eq!(info.custom_kind(), CustomDamage::HEADSHOT);
	assert_eq!(info.custom_damage(), CustomDamage::HEADSHOT.to_raw());

	info.set_custom_damage(1000);
	assert_eq!(info.custom_kind(), CustomDamage::from_raw(1000));
}

#[test]
fn damage_constants_are_the_same_kinds() {
	let pairs = [
		(CUSTOM_DAMAGE_BACKSTAB, CustomDamage::BACKSTAB),
		(CUSTOM_DAMAGE_DECAPITATION, CustomDamage::DECAPITATION),
		(
			CUSTOM_DAMAGE_DECAPITATION_BOSS,
			CustomDamage::DECAPITATION_BOSS,
		),
		(CUSTOM_DAMAGE_HEADSHOT, CustomDamage::HEADSHOT),
		(
			CUSTOM_DAMAGE_HEADSHOT_DECAPITATION,
			CustomDamage::HEADSHOT_DECAPITATION,
		),
		(
			CUSTOM_DAMAGE_MERASMUS_DECAPITATION,
			CustomDamage::MERASMUS_DECAPITATION,
		),
		(CUSTOM_DAMAGE_PLASMA, CustomDamage::PLASMA),
		(CUSTOM_DAMAGE_PLASMA_CHARGED, CustomDamage::PLASMA_CHARGED),
		(
			CUSTOM_DAMAGE_TAUNT_BARBARIAN_SWING,
			CustomDamage::TAUNT_BARBARIAN_SWING,
		),
	];

	for (constant, kind) in pairs {
		assert_eq!(CustomDamage::from_raw(constant), kind);
	}
}

#[test]
fn kinds_keep_tf2s_numbers_and_names() {
	assert_eq!(
		CustomDamage::ALL.len(),
		sys::ETFDmgCustom_TF_DMG_CUSTOM_END as usize
	);
	assert_eq!(CustomDamage::default(), CustomDamage::NONE);

	for (number, kind) in (0..).zip(CustomDamage::ALL) {
		let name = kind.name().unwrap();

		assert_eq!(kind.to_raw(), number, "{name}");
		assert_eq!(CustomDamage::from_raw(number), kind, "{name}");
		assert_eq!(
			NATIVE_NAMES[number as usize],
			format!("ETFDmgCustom_{name}")
		);
		assert_eq!(format!("{kind:?}"), name);
	}

	// Numbers TF2 does not name, such as its count of kinds, are kept as
	// they are.
	for number in [-1, CustomDamage::ALL.len() as i32, 1000] {
		let kind = CustomDamage::from_raw(number);

		assert_eq!(kind.to_raw(), number);
		assert_eq!(kind.name(), None);
		assert_eq!(format!("{kind:?}"), format!("CustomDamage({number})"));
	}
}
