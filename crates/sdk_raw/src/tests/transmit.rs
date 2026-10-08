//! Tests of reading the sets of edicts a `CCheckTransmitInfo` marks.

use super::*;

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
