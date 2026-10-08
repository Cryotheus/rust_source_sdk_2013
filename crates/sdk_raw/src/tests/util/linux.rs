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

#[test]
fn modules_are_found_by_an_address_in_them() {
	let strlen = loaded_symbol(c"libc.so.6", c"strlen").expect("libc exports strlen");
	let address = strlen.as_ptr() as usize;

	// SAFETY: libc stays loaded.
	unsafe {
		assert_eq!(module_symbol(address, c"strlen"), Some(strlen));
		assert!(module_symbol(address, c"memcpy").is_some());
		assert_eq!(
			module_symbol(address, c"source_sdk_2013_no_such_export"),
			None
		);
	}

	// No module holds the heap.
	let heap = Box::new(0_u64);

	// SAFETY: The address lies in no module.
	let found = unsafe { module_symbol(&raw const *heap as usize, c"strlen") };

	assert_eq!(found, None);
}
