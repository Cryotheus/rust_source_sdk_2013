//! Validation of the Linux symbol of `CTFGameRules::CleanUpMap` against a
//! retail `server_srv.so`.
#![cfg(target_os = "linux")]

use super::*;
use crate::util::elf::Elf;

#[test]
#[ignore = "set TF2_SERVER_IMAGE to an authorized unstripped retail server_srv.so for binary validation"]
fn retail_clean_up_map_symbol() {
	let bytes =
		std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE")).unwrap();
	let elf = Elf::new(&bytes).unwrap();

	assert!(elf.symbol(CLEAN_UP_MAP_SYMBOL).is_some());
}
