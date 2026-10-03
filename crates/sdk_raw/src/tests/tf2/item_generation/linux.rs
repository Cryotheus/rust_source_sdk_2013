//! Validation of the Linux symbols of item generation against a retail
//! `server_srv.so`.

use super::*;
use crate::util::elf::Elf;

#[test]
#[ignore = "set TF2_SERVER_IMAGE to an authorized unstripped retail server_srv.so for binary validation"]
fn retail_item_generation_symbols() {
	let bytes =
		std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE")).unwrap();
	let elf = Elf::new(&bytes).unwrap();

	for name in [SPAWN_ITEM, GET_ITEM_SCHEMA, GET_ITEM_DEFINITION] {
		assert!(elf.symbol(name).is_some());
	}

	let (getter, body) = elf.symbol(ITEM_GENERATION).unwrap();

	assert_eq!(body.len(), ITEM_GENERATION_GETTER.len());
	assert!(pattern(body, ITEM_GENERATION_GETTER));
	assert!(relative(getter, body, ITEM_GENERATION_GETTER_OPERAND).is_some());
	assert!(
		elf.symbol(b"_ZN15CItemGeneration9SpawnItemEiRK6VectorRK6QAngleiiPKci")
			.is_none(),
		"retail ABI must exclude the SDK's extra class argument"
	);
}
