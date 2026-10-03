use super::Targets;
use crate::util::{elf::LoadedElf, relative};

/// Resolves the item generation functions from the game module's symbols.
///
/// # Safety
///
/// The caller keeps the factory's game module loaded for all resolution and
/// subsequent native calls.
pub(super) unsafe fn resolve(factory: usize) -> Option<Targets> {
	// SAFETY: The caller guarantees the factory module remains loaded and its
	// image mappings remain valid throughout this snapshot and the calls.
	let elf = unsafe { LoadedElf::at(factory) }.ok()?;
	let (spawn, _) = elf.resolve(b"_ZN15CItemGeneration9SpawnItemEiRK6VectorRK6QAngleiiPKc")?;
	let (getter, body) = elf.resolve(b"_Z14ItemGenerationv")?;

	if body.len() != 8 || body[..3] != [0x48, 0x8d, 0x05] || body[7] != 0xc3 {
		return None;
	}

	let singleton = relative(getter, body, 3)?;

	if !elf.contains(singleton, 16, false, true) {
		return None;
	}

	elf.read(singleton, 16)?;

	let (schema, _) = elf.resolve(b"_Z13GetItemSchemav")?;
	let (definition, _) = elf.resolve(b"_ZN15CEconItemSchema17GetItemDefinitionEi")?;

	Some(Targets {
		spawn,
		singleton,
		schema,
		schema_offset: 0,
		definition,
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

		for name in [
			b"_ZN15CItemGeneration9SpawnItemEiRK6VectorRK6QAngleiiPKc".as_slice(),
			b"_Z13GetItemSchemav",
			b"_ZN15CEconItemSchema17GetItemDefinitionEi",
		] {
			assert!(elf.symbol(name).is_some());
		}

		let (getter, body) = elf.symbol(b"_Z14ItemGenerationv").unwrap();

		assert_eq!(body.len(), 8);
		assert_eq!(&body[..3], &[0x48, 0x8d, 0x05]);
		assert_eq!(body[7], 0xc3);
		assert!(relative(getter, body, 3).is_some());
		assert!(
			elf.symbol(b"_ZN15CItemGeneration9SpawnItemEiRK6VectorRK6QAngleiiPKci")
				.is_none(),
			"retail ABI must exclude the SDK's extra class argument"
		);
	}
}
