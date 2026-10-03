//! Validation of the Windows signatures and call chain of item generation
//! against a retail `server.dll`.

use super::*;
use crate::tf2::item_generation::{ModuleKey, lookup};

#[test]
#[ignore = "set TF2_SERVER_IMAGE to an authorized retail server.dll for binary validation"]
fn retail_item_generation_call_chain() {
	let bytes =
		std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE")).unwrap();

	let mut image = crate::util::pe::from_file(&bytes).unwrap();

	let addresses =
		resolve_image(&image).expect("retail signatures and independent call references");

	assert!(image.executable(addresses.spawn_item));
	assert!(image.executable(addresses.schema_getter));
	assert!(image.executable(addresses.get_item_definition));

	// Resolving the same module twice through the cache inspects it once.
	// A file has no loader to find its factory by, so another of its
	// addresses stands in for it.
	let key = ModuleKey {
		base: image.base,
		factory: addresses.spawn_item,
	};

	assert_eq!(lookup(key, || resolve_image(&image)), Some(addresses));
	assert_eq!(
		lookup(key, || panic!("a cache hit inspected the module again")),
		Some(addresses)
	);

	// Matching prologues alone are insufficient: redirect the caller to
	// another executable function and require the cross-check to reject it.
	let call = image
		.unique(GENERATE_RANDOM_ITEM, FUNCTION_ALIGNMENT)
		.unwrap()
		+ GENERATE_RANDOM_ITEM_CALLS_SPAWN_ITEM;

	let displacement =
		i32::try_from(addresses.get_item_definition as isize - (call + 5) as isize).unwrap();

	let section = image
		.sections
		.iter_mut()
		.find(|section| (section.address..section.address + section.bytes.len()).contains(&call))
		.unwrap();

	let offset = call - section.address + 1;

	section.bytes[offset..offset + 4].copy_from_slice(&displacement.to_le_bytes());
	assert!(resolve_image(&image).is_none());
}
