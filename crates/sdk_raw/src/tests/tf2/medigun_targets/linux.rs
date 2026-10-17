//! Validation of the Linux symbol of `AllowedToHealTarget` against a retail
//! `server_srv.so`.

use super::*;
use crate::util::elf::Elf;

#[test]
#[ignore = "set TF2_SERVER_IMAGE to an authorized unstripped retail server_srv.so for binary validation"]
fn retail_allowed_to_heal_target_symbol() {
	let bytes =
		std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE")).unwrap();
	let elf = Elf::new(&bytes).unwrap();

	assert!(elf.symbol(ALLOWED_TO_HEAL_TARGET).is_some());
}
