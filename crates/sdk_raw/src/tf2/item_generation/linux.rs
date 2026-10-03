//! Resolution in retail TF2's 64-bit Linux `server_srv.so`, through the
//! mangled symbols of an unstripped build.

use super::{Addresses, ITEM_GENERATION_GETTER, ITEM_GENERATION_GETTER_OPERAND, SINGLETON_LEN};
use crate::util::elf::LoadedElf;
use crate::util::{pattern, relative};
use std::num::NonZeroUsize;

/// `CEconItemSchema::GetItemDefinition(int)`.
const GET_ITEM_DEFINITION: &[u8] = b"_ZN15CEconItemSchema17GetItemDefinitionEi";

/// `GetItemSchema()`, which returns the item schema itself.
const GET_ITEM_SCHEMA: &[u8] = b"_Z13GetItemSchemav";

/// `ItemGeneration()`, which returns the `CItemGeneration` singleton.
const ITEM_GENERATION: &[u8] = b"_Z14ItemGenerationv";

/// How far into the object `GetItemSchema()` returns, the schema itself, the
/// item schema lies.
pub(super) const SCHEMA_OFFSET: usize = 0;

/// Retail's seven-argument `CItemGeneration::SpawnItem`.
const SPAWN_ITEM: &[u8] = b"_ZN15CItemGeneration9SpawnItemEiRK6VectorRK6QAngleiiPKc";

/// Resolves the item generation functions from the symbols of the module
/// containing `address`.
///
/// # Safety
///
/// The module containing `address` stays loaded, with its image mappings
/// unchanged, throughout this call.
pub(super) unsafe fn resolve(address: usize) -> Option<Addresses> {
	// SAFETY: The caller guarantees the module remains loaded and its image
	// mappings remain valid throughout this snapshot.
	let elf = unsafe { LoadedElf::at(address) }.ok()?;
	let (spawn_item, _) = elf.resolve(SPAWN_ITEM)?;
	let (getter, body) = elf.resolve(ITEM_GENERATION)?;

	if body.len() != ITEM_GENERATION_GETTER.len() || !pattern(body, ITEM_GENERATION_GETTER) {
		return None;
	}

	let singleton = relative(getter, body, ITEM_GENERATION_GETTER_OPERAND)?;

	if !elf.contains(singleton, SINGLETON_LEN, false, true) {
		return None;
	}

	// The singleton must also be readable now, not only mapped by the file.
	elf.read(singleton, SINGLETON_LEN)?;

	let (schema_getter, _) = elf.resolve(GET_ITEM_SCHEMA)?;
	let (get_item_definition, _) = elf.resolve(GET_ITEM_DEFINITION)?;

	Some(Addresses {
		get_item_definition,
		schema_getter,
		singleton: NonZeroUsize::new(singleton)?,
		spawn_item,
	})
}

#[cfg(test)]
mod tests {
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
}
