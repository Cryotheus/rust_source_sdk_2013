#![cfg(target_os = "linux")]
//! Tests of reading Linux modules loaded in this process, and their memory.

use source_sdk_2013_raw::util::{
	Error, Image, MAX_IMAGE_BYTES, MemoryReader, Module, is_executable, loaded_symbol, pin_module,
};

#[test]
fn memory_reader_copies_owned_bytes_and_rejects_invalid_ranges() {
	let memory = MemoryReader::open().unwrap();
	let bytes = [1_u8, 2, 3, 4];
	assert_eq!(
		memory.copy(bytes.as_ptr() as usize, bytes.len()).unwrap(),
		bytes
	);
	assert!(matches!(
		memory.copy(usize::MAX, 2),
		Err(Error::InvalidImage)
	));
	assert!(matches!(
		memory.copy(1, MAX_IMAGE_BYTES + 1),
		Err(Error::InvalidImage)
	));
	assert!(memory.copy(0, 8).is_err());
}

#[test]
fn pins_the_module_containing_an_address() {
	let address = loaded_symbol(c"libc.so.6", c"getpid").unwrap().as_ptr() as usize;
	// SAFETY: libc stays loaded for the life of the test process.
	let module = unsafe { Module::at(address) }.unwrap();
	// SAFETY: As above.
	let pinned = unsafe { pin_module(address) }.unwrap();
	assert_eq!(pinned.base(), module.base());
	assert_eq!(pinned.path(), module.path());
}

#[test]
fn snapshots_code_and_data_with_load_permissions() {
	let anchor = snapshots_code_and_data_with_load_permissions as *const () as usize;
	// SAFETY: The test executable stays loaded throughout this test.
	let module = unsafe { Module::at(anchor) }.unwrap();
	assert!(module.path().is_file());
	assert!(is_executable(anchor));
	// SAFETY: The test executable stays loaded throughout this snapshot.
	let image = unsafe { Image::load(anchor) }.unwrap();
	assert_eq!(image.base, module.base());
	assert!(
		image
			.sections
			.iter()
			.any(|section| section.executable && !section.bytes.is_empty())
	);
	assert!(image.sections.iter().any(|section| section.writable));
	assert!(image.sections.iter().any(|section| !section.executable));
}

#[test]
fn symbols_are_found_only_in_loaded_libraries() {
	assert!(loaded_symbol(c"libc.so.6", c"getpid").is_some());
	assert!(loaded_symbol(c"libc.so.6", c"source_sdk_2013_raw_absent").is_none());
	assert!(loaded_symbol(c"libsource_sdk_2013_raw_absent.so", c"Msg").is_none());
}
