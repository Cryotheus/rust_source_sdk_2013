//! Tests of reading the sets of edicts a `CCheckTransmitInfo` marks.

use super::*;

#[test]
fn clearing_edict_bits_preserves_neighbors_and_bounds() {
	let mut words = [u32::MAX; MAX_EDICTS as usize / 32];
	let bits = words.as_mut_ptr().cast::<u8>();
	// SAFETY: The local words hold a writable `MAX_EDICTS` set.
	for index in [0, 31, 32, MAX_EDICTS - 1] {
		assert!(unsafe { clear_edict_bit(bits, index) });
		assert!(!unsafe { has_edict_bit(bits, index) });
		assert!(!unsafe { clear_edict_bit(bits, index) });
	}
	assert_eq!(words[0], u32::MAX & !1 & !(1 << 31));
	assert_eq!(words[1], u32::MAX & !1);
	assert_eq!(words[2], u32::MAX);
	assert_eq!(words[words.len() - 1], u32::MAX & !(1 << 31));
	let before = words;
	// SAFETY: The same local set, with checked invalid indices, and null.
	assert!(!unsafe { clear_edict_bit(bits, -1) });
	assert!(!unsafe { clear_edict_bit(bits, MAX_EDICTS) });
	assert!(!unsafe { clear_edict_bit(std::ptr::null_mut(), 1) });
	assert_eq!(words, before);
}

#[test]
fn edict_bits_are_read_from_32_bit_words() {
	let mut words = [0_u32; MAX_EDICTS as usize / 32];

	words[0] = 1 << 3;
	words[1] = 1 << 31;
	words[63] = 1 << 31;

	let bits = words.as_ptr().cast::<u8>();

	// SAFETY: The set is a local of `MAX_EDICTS` bits.
	let set = |index| unsafe { has_edict_bit(bits, index) };

	assert!(set(3));
	assert!(set(63));
	assert!(set(MAX_EDICTS - 1));
	assert!(!set(0));
	assert!(!set(32));
	assert!(!set(-1));
	assert!(!set(MAX_EDICTS));

	// SAFETY: A null set holds no bit.
	assert!(!unsafe { has_edict_bit(std::ptr::null(), 3) });
}
