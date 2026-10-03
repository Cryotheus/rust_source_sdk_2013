use super::*;
use crate::sig;
use crate::util::{Image, SignaturePattern, pattern, relative};

const GIVE: &[SignaturePattern] = &sig![0x48 0x89 0x5c 0x24 0x18 0x48 0x89 0x6c 0x24 0x20 0x56 0x57 0x41 0x57 0x48 0x81 0xec 0xb0 0 0 0 0x0f 0xb6 0x9c 0x24 0xf0 0 0 0 0x49 0x8b 0xe9 0x45 0x8b 0xf8 0x48 0x8b 0xfa 0x48 0x8b 0xf1];
const RANDOM: &[SignaturePattern] = &sig![0x48 0x89 0x5c 0x24 0x08 0x48 0x89 0x6c 0x24 0x10 0x48 0x89 0x74 0x24 0x18 0x57 0x48 0x83 0xec 0x50 0x49 0x8b 0xf9 0x49 0x8b 0xf0 0x48 0x8b 0xda 0x48 0x8b 0xe9 0xe8 ? ? ? ?];

// Bravo retail Windows x64. Wildcards are relative call/jump operands;
// the complete argument setup and Spawn/Activate virtual calls are fixed.
const SPAWN: &[SignaturePattern] = &sig![0x48 0x89 0x5c 0x24 0x08 0x48 0x89 0x6c 0x24 0x10 0x48 0x89 0x74 0x24 0x18 0x48 0x89 0x7c 0x24 0x20 0x41 0x56 0x48 0x83 0xec 0x30 0x49 0x8b 0xe9 0x4d 0x8b 0xf0 0x8b 0xf2 0xe8 ? ? ? ? 0x0f 0xb7 0xd6 0x48 0x8d 0x48 0x08 0xe8 ? ? ? ?];

pub(super) unsafe fn resolve(factory: usize) -> Option<Targets> {
	// SAFETY: The caller keeps the game module loaded for this callback.
	let image = unsafe { Image::load(factory) }.ok()?;

	resolve_image(&image)
}

fn resolve_image(image: &Image) -> Option<Targets> {
	let spawn = image.unique(SPAWN, 16)?;
	let random = image.unique(RANDOM, 16)?;
	let give = image.unique(GIVE, 16)?;

	// Independent native callers must agree on both item-generation targets.
	if image.call(random + 0x69)? != spawn || image.call(give + 0x155)? != random {
		return None;
	}

	let getter = image.call(give + 0x135)?;

	if image.call(give + 0x72)? != getter {
		return None;
	}

	let bytes = image.read(getter, 8)?;

	if !pattern(bytes, &sig![0x48 0x8d 0x05 ? ? ? ? 0xc3]) {
		return None;
	}

	let singleton = relative(getter, bytes, 3)?;

	if !image.contains(singleton, 16, false, true) {
		return None;
	}

	let tail = image.read(spawn + 0xc1, 30)?;

	if !pattern(
		tail,
		&sig![0x48 0x8b 0x03 0x48 0x8b 0xcb 0xff 0x90 0xc0 0 0 0 0x48 0x8b 0x03 0x48 0x8b 0xcb 0xff 0x90 0x18 0x01 0 0 0x48 0x8b 0xc3],
	) {
		return None;
	}

	let schema = image.call(spawn + 0x22)?;

	if image.call(random + 0x20)? != schema {
		return None;
	}

	let definition = image.call(spawn + 0x2e)?;

	Some(Targets {
		spawn,
		singleton,
		schema,
		schema_offset: 8,
		definition,
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	#[ignore = "set TF2_SERVER_IMAGE to an authorized retail server.dll for binary validation"]
	fn retail_item_generation_call_chain() {
		let bytes =
			std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE")).unwrap();

		let mut image = crate::util::pe::from_file(&bytes).unwrap();

		let targets =
			resolve_image(&image).expect("retail signatures and independent call references");

		assert!(image.executable(targets.spawn));
		assert!(image.executable(targets.schema));
		assert!(image.executable(targets.definition));
		assert_eq!(targets.schema_offset, 8);

		// Matching prologues alone are insufficient: redirect the caller to
		// another executable function and require the cross-check to reject it.
		let call = image.unique(RANDOM, 16).unwrap() + 0x69;
		let displacement =
			i32::try_from(targets.definition as isize - (call + 5) as isize).unwrap();

		let section = image
			.sections
			.iter_mut()
			.find(|section| {
				(section.address..section.address + section.bytes.len()).contains(&call)
			})
			.unwrap();

		let offset = call - section.address + 1;

		section.bytes[offset..offset + 4].copy_from_slice(&displacement.to_le_bytes());
		assert!(resolve_image(&image).is_none());
	}
}
