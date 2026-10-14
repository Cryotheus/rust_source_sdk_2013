//! Tests of TF2's custom damage kinds.

use super::*;

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
