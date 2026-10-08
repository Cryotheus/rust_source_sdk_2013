//! Tests of reading the program headers of loaded Linux modules.

use super::*;

#[test]
fn rejects_overflowing_or_excessive_load_segments() {
	let mut program = [0_u8; 56];
	program[..4].copy_from_slice(&1_u32.to_le_bytes());
	program[4..8].copy_from_slice(&5_u32.to_le_bytes());
	program[16..24].copy_from_slice(&usize::MAX.to_le_bytes());
	program[40..48].copy_from_slice(&1_usize.to_le_bytes());
	assert!(load_segments(1, &program).is_err());
	program[16..24].copy_from_slice(&0_usize.to_le_bytes());
	program[40..48].copy_from_slice(&(MAX_IMAGE_BYTES + 1).to_le_bytes());
	assert!(load_segments(0, &program).is_err());
}
