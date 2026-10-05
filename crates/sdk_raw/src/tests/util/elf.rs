//! Tests of resolving the symbols of a loaded ELF module, against its memory.
#![cfg(target_os = "linux")]

use super::*;

/// A variable of the test executable, in `.bss`, which the test finds by its
/// symbol.
#[unsafe(no_mangle)]
static mut SOURCE_SDK_RAW_LOADED_ELF_TEST_VARIABLE: [u64; 8] = [0; 8];

#[test]
fn loaded_function_requires_matching_file_and_memory() {
	let anchor = source_sdk_raw_loaded_elf_test_anchor as *const () as usize;
	// SAFETY: The test executable remains loaded throughout this test.
	let mut loaded = unsafe { LoadedElf::at(anchor) }.unwrap();
	let (address, body) = loaded
		.resolve(b"source_sdk_raw_loaded_elf_test_anchor")
		.unwrap();
	assert_eq!(address, anchor);
	assert_eq!(loaded.read(address, body.len()).unwrap(), body);
	assert!(!loaded.contains(address, 1, true, true));
	assert!(loaded.read(usize::MAX, 8).is_none());
	let (offset, _) = Elf::new(&loaded.bytes)
		.unwrap()
		.symbol(b"source_sdk_raw_loaded_elf_test_anchor")
		.unwrap();
	let elf = Elf::new(&loaded.bytes).unwrap();
	let (_, body) = elf
		.symbol(b"source_sdk_raw_loaded_elf_test_anchor")
		.unwrap();
	let file_offset = body.as_ptr() as usize - loaded.bytes.as_ptr() as usize;
	assert_eq!(loaded.module.base() + offset, anchor);
	loaded.bytes[file_offset] ^= 1;
	assert!(
		loaded
			.resolve(b"source_sdk_raw_loaded_elf_test_anchor")
			.is_none()
	);
}

#[test]
fn function_addresses_ignore_the_body_but_not_the_kind() {
	let anchor = source_sdk_raw_loaded_elf_test_anchor as *const () as usize;
	// SAFETY: The test executable remains loaded throughout this test.
	let mut loaded = unsafe { LoadedElf::at(anchor) }.unwrap();
	let name = b"source_sdk_raw_loaded_elf_test_anchor";
	assert_eq!(loaded.function_address(name), Some(anchor));

	// A body differing from the file, as a detour leaves it, still names the
	// function.
	let elf = Elf::new(&loaded.bytes).unwrap();
	let (_, body) = elf.symbol(name).unwrap();
	let file_offset = body.as_ptr() as usize - loaded.bytes.as_ptr() as usize;
	loaded.bytes[file_offset] ^= 1;
	assert!(loaded.resolve(name).is_none());
	assert_eq!(loaded.function_address(name), Some(anchor));

	assert!(
		loaded
			.function_address(b"SOURCE_SDK_RAW_LOADED_ELF_TEST_VARIABLE")
			.is_none()
	);
	assert!(
		loaded
			.function_address(b"source_sdk_raw_loaded_elf_test_missing")
			.is_none()
	);
}

#[test]
fn loaded_variables_require_a_writable_load_range() {
	let anchor = source_sdk_raw_loaded_elf_test_anchor as *const () as usize;
	// SAFETY: The test executable remains loaded throughout this test.
	let loaded = unsafe { LoadedElf::at(anchor) }.unwrap();
	let (address, size) = loaded
		.resolve_data(b"SOURCE_SDK_RAW_LOADED_ELF_TEST_VARIABLE")
		.unwrap();
	assert_eq!(
		address,
		(&raw const SOURCE_SDK_RAW_LOADED_ELF_TEST_VARIABLE).addr()
	);
	assert_eq!(size, 64);
	assert!(loaded.contains(address, size, false, true));
	assert!(
		loaded
			.resolve_data(b"source_sdk_raw_loaded_elf_test_anchor")
			.is_none()
	);
}

/// A function of the test executable, which the test finds by its symbol.
#[unsafe(no_mangle)]
#[inline(never)]
extern "C" fn source_sdk_raw_loaded_elf_test_anchor(value: u64) -> u64 {
	value.wrapping_add(7)
}
