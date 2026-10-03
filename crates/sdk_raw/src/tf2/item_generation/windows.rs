//! Resolution in retail TF2's 64-bit Windows `server.dll`, through signatures
//! of three functions whose calls must agree with each other.
//!
//! Wildcards are relative call operands; the complete argument setup of
//! `SpawnItem` and its `Spawn` and `Activate` virtual calls are fixed.

use super::{Addresses, ITEM_GENERATION_GETTER, ITEM_GENERATION_GETTER_OPERAND, SINGLETON_LEN};
use crate::sig;
use crate::util::{Image, SignaturePattern, pattern, relative};
use std::mem::offset_of;
use std::num::NonZeroUsize;

// The tail calls `CBaseEntity::Spawn` and `Activate` through the generated
// vtable's slots, and `SpawnItem` passes `GetItemDefinition` the schema
// `SCHEMA_OFFSET` bytes into what `ItemSystem()` returns. Each call offset is
// that of a `call rel32` within its signature.
const _: () = {
	use sys::CBaseEntity__bindgen_vtable as Vtable;

	assert!(matches!(
		exact_u32(SPAWN_ITEM_TAIL, SPAWN_DISPLACEMENT),
		Some(offset) if offset as usize == offset_of!(Vtable, CBaseEntity_Spawn)
	));

	assert!(matches!(
		exact_u32(SPAWN_ITEM_TAIL, ACTIVATE_DISPLACEMENT),
		Some(offset) if offset as usize == offset_of!(Vtable, CBaseEntity_Activate)
	));

	assert!(is_exact(
		SPAWN_ITEM,
		SPAWN_ITEM_CALLS_GET_ITEM_DEFINITION - 1,
		SCHEMA_OFFSET as u8
	));
	assert!(is_exact(
		SPAWN_ITEM,
		SPAWN_ITEM_CALLS_GET_ITEM_DEFINITION,
		CALL
	));
	assert!(is_exact(SPAWN_ITEM, SPAWN_ITEM_CALLS_ITEM_SYSTEM, CALL));
	assert!(is_exact(
		GENERATE_RANDOM_ITEM,
		GENERATE_RANDOM_ITEM_CALLS_ITEM_SYSTEM,
		CALL
	));
};

/// Where the displacement of [`SPAWN_ITEM_TAIL`]'s call to `Activate` starts.
const ACTIVATE_DISPLACEMENT: usize = 20;

/// The opcode of `call rel32`.
const CALL: u8 = 0xe8;

/// The alignment of function entries in the module.
const FUNCTION_ALIGNMENT: usize = 16;

/// The prologue of `CItemGeneration::GenerateRandomItem`, up to its call to
/// `ItemSystem()`.
const GENERATE_RANDOM_ITEM: &[SignaturePattern] = &sig![0x48 0x89 0x5c 0x24 0x08 0x48 0x89 0x6c 0x24 0x10 0x48 0x89 0x74 0x24 0x18 0x57 0x48 0x83 0xec 0x50 0x49 0x8b 0xf9 0x49 0x8b 0xf0 0x48 0x8b 0xda 0x48 0x8b 0xe9 0xe8 ? ? ? ?];

/// Where [`GENERATE_RANDOM_ITEM`] calls `ItemSystem()`.
const GENERATE_RANDOM_ITEM_CALLS_ITEM_SYSTEM: usize = 0x20;

/// Where [`GENERATE_RANDOM_ITEM`] calls `SpawnItem`.
const GENERATE_RANDOM_ITEM_CALLS_SPAWN_ITEM: usize = 0x69;

/// The prologue of `CTFPlayer::GiveNamedItem`.
const GIVE_NAMED_ITEM: &[SignaturePattern] = &sig![0x48 0x89 0x5c 0x24 0x18 0x48 0x89 0x6c 0x24 0x20 0x56 0x57 0x41 0x57 0x48 0x81 0xec 0xb0 0 0 0 0x0f 0xb6 0x9c 0x24 0xf0 0 0 0 0x49 0x8b 0xe9 0x45 0x8b 0xf8 0x48 0x8b 0xfa 0x48 0x8b 0xf1];

/// Where [`GIVE_NAMED_ITEM`] calls `GenerateRandomItem`.
const GIVE_NAMED_ITEM_CALLS_GENERATE_RANDOM_ITEM: usize = 0x155;

/// Where [`GIVE_NAMED_ITEM`] calls `ItemGeneration()`, once for each way it
/// generates an item.
const GIVE_NAMED_ITEM_CALLS_ITEM_GENERATION: [usize; 2] = [0x72, 0x135];

/// How far into the object `ItemSystem()` returns, a `CEconItemSystem`, its
/// item schema lies.
pub(super) const SCHEMA_OFFSET: usize = 8;

/// Where the displacement of [`SPAWN_ITEM_TAIL`]'s call to `Spawn` starts.
const SPAWN_DISPLACEMENT: usize = 8;

/// The prologue of retail's seven-argument `CItemGeneration::SpawnItem`, up to
/// its call to `CEconItemSchema::GetItemDefinition`.
const SPAWN_ITEM: &[SignaturePattern] = &sig![0x48 0x89 0x5c 0x24 0x08 0x48 0x89 0x6c 0x24 0x10 0x48 0x89 0x74 0x24 0x18 0x48 0x89 0x7c 0x24 0x20 0x41 0x56 0x48 0x83 0xec 0x30 0x49 0x8b 0xe9 0x4d 0x8b 0xf0 0x8b 0xf2 0xe8 ? ? ? ? 0x0f 0xb7 0xd6 0x48 0x8d 0x48 0x08 0xe8 ? ? ? ?];

/// Where [`SPAWN_ITEM`] calls `CEconItemSchema::GetItemDefinition`, with the
/// schema `SCHEMA_OFFSET` bytes into what `ItemSystem()` returned.
const SPAWN_ITEM_CALLS_GET_ITEM_DEFINITION: usize = 0x2e;

/// Where [`SPAWN_ITEM`] calls `ItemSystem()`.
const SPAWN_ITEM_CALLS_ITEM_SYSTEM: usize = 0x22;

/// The tail of `SpawnItem`, which calls the new entity's `Spawn`, then its
/// `Activate`, through its vtable, and returns it.
const SPAWN_ITEM_TAIL: &[SignaturePattern] = &sig![0x48 0x8b 0x03 0x48 0x8b 0xcb 0xff 0x90 0xc0 0 0 0 0x48 0x8b 0x03 0x48 0x8b 0xcb 0xff 0x90 0x18 0x01 0 0 0x48 0x8b 0xc3];

/// Where [`SPAWN_ITEM_TAIL`] starts in `SpawnItem`.
const SPAWN_ITEM_TAIL_OFFSET: usize = 0xc1;

/// The little-endian `u32` that `signature` matches exactly at `at`, or `None`
/// if any of its bytes is a wildcard or past the end.
const fn exact_u32(signature: &[SignaturePattern], at: usize) -> Option<u32> {
	let mut bytes = [0; 4];
	let mut index = 0;

	while index < bytes.len() {
		match at.checked_add(index) {
			Some(position) if position < signature.len() => match signature[position] {
				SignaturePattern::Exact(byte) => bytes[index] = byte,
				SignaturePattern::Any => return None,
			},

			_ => return None,
		}

		index += 1;
	}

	Some(u32::from_le_bytes(bytes))
}

/// Whether `signature` matches exactly `byte` at `at`.
const fn is_exact(signature: &[SignaturePattern], at: usize, byte: u8) -> bool {
	at < signature.len() && matches!(signature[at], SignaturePattern::Exact(found) if found == byte)
}

/// Resolves the item generation functions in the module containing `address`.
///
/// # Safety
///
/// The module containing `address` stays loaded throughout this call.
pub(super) unsafe fn resolve(address: usize) -> Option<Addresses> {
	// SAFETY: The caller keeps the module loaded throughout this call.
	let image = unsafe { Image::load(address) }.ok()?;

	resolve_image(&image)
}

/// Resolves the item generation functions in a snapshot of the module.
fn resolve_image(image: &Image) -> Option<Addresses> {
	let spawn_item = image.unique(SPAWN_ITEM, FUNCTION_ALIGNMENT)?;
	let random = image.unique(GENERATE_RANDOM_ITEM, FUNCTION_ALIGNMENT)?;
	let give = image.unique(GIVE_NAMED_ITEM, FUNCTION_ALIGNMENT)?;

	// Independent native callers must agree on both item-generation targets.
	if image.call(random + GENERATE_RANDOM_ITEM_CALLS_SPAWN_ITEM)? != spawn_item
		|| image.call(give + GIVE_NAMED_ITEM_CALLS_GENERATE_RANDOM_ITEM)? != random
	{
		return None;
	}

	let [first, second] = GIVE_NAMED_ITEM_CALLS_ITEM_GENERATION;
	let getter = image.call(give + first)?;

	if image.call(give + second)? != getter {
		return None;
	}

	let body = image.read(getter, ITEM_GENERATION_GETTER.len())?;

	if !pattern(body, ITEM_GENERATION_GETTER) {
		return None;
	}

	let singleton = relative(getter, body, ITEM_GENERATION_GETTER_OPERAND)?;

	if !image.contains(singleton, SINGLETON_LEN, false, true) {
		return None;
	}

	let tail = image.read(spawn_item + SPAWN_ITEM_TAIL_OFFSET, SPAWN_ITEM_TAIL.len())?;

	if !pattern(tail, SPAWN_ITEM_TAIL) {
		return None;
	}

	let schema_getter = image.call(spawn_item + SPAWN_ITEM_CALLS_ITEM_SYSTEM)?;

	if image.call(random + GENERATE_RANDOM_ITEM_CALLS_ITEM_SYSTEM)? != schema_getter {
		return None;
	}

	let get_item_definition = image.call(spawn_item + SPAWN_ITEM_CALLS_GET_ITEM_DEFINITION)?;

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
			.find(|section| {
				(section.address..section.address + section.bytes.len()).contains(&call)
			})
			.unwrap();

		let offset = call - section.address + 1;

		section.bytes[offset..offset + 4].copy_from_slice(&displacement.to_le_bytes());
		assert!(resolve_image(&image).is_none());
	}
}
