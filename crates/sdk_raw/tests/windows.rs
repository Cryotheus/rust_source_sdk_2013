#![cfg(target_os = "windows")]
//! Tests of finding Windows modules loaded in this process, and reading them
//! and their memory.

use source_sdk_2013_raw::util::{Error, Image, MemoryReader, Module, is_executable, loaded_symbol};
use std::ffi::c_void;
use std::io;

#[link(name = "kernel32")]
unsafe extern "system" {
	/// A function of kernel32, which every Windows process loads.
	fn GetCurrentProcess() -> *mut c_void;
}

#[test]
fn modules_are_found_by_name_only_when_loaded() {
	let address = GetCurrentProcess as *const () as usize;
	// SAFETY: This process's linked kernel32 module remains loaded.
	let by_address = unsafe { Module::at(address) }.unwrap();
	let by_name = Module::loaded("kernel32.dll").unwrap();
	assert_eq!(by_name.base(), by_address.base());
	let image = Image::from_module(&by_name).unwrap();
	assert_eq!(image.base, by_name.base());
	assert!(image.executable(address));
	assert!(Module::loaded("source_sdk_2013_raw_absent.dll").is_err());
	assert!(matches!(
		Module::loaded("kernel32.dll\0"),
		Err(Error::Io(error)) if error.kind() == io::ErrorKind::InvalidInput
	));
}

#[test]
fn snapshots_live_module_and_rejects_unreadable_memory() {
	let address = GetCurrentProcess as *const () as usize;
	// SAFETY: This process's linked kernel32 module remains loaded.
	let image = unsafe { Image::load(address) }.unwrap();
	assert!(image.executable(address));
	let reader = MemoryReader::open().unwrap();
	let bytes = [12_u8, 34, 56];
	assert_eq!(
		reader.copy(bytes.as_ptr() as usize, bytes.len()).unwrap(),
		bytes
	);
	assert!(reader.copy(1, 16).is_err());
	assert!(reader.copy(usize::MAX, 2).is_err());
	assert!(is_executable(address));
	assert!(!is_executable(bytes.as_ptr() as usize));
}

#[test]
fn symbols_are_found_only_in_loaded_libraries() {
	assert!(loaded_symbol(c"kernel32.dll", c"GetCurrentProcess").is_some());
	assert!(loaded_symbol(c"kernel32.dll", c"source_sdk_2013_raw_absent").is_none());
	assert!(loaded_symbol(c"source_sdk_2013_raw_absent.dll", c"Msg").is_none());
}
