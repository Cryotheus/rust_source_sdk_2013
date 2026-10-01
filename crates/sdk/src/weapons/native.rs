//! Retail TF2 item generation. Resolve only the supported seven-argument
//! CItemGeneration::SpawnItem ABI; newer SDK headers add a class argument.
//! All inspection uses owned snapshots. Missing/ambiguous signatures, stripped
//! ELF symbols, or a changed file fail closed before calling native code.

use super::WeaponError;
use crate::Server;
use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;

struct Targets {
	spawn: usize,
	singleton: usize,
	schema: usize,
	schema_offset: usize,
	definition: usize,
}

/// The caller supplies the same spawn/callback lifetime guarantees as `give`.
pub(super) unsafe fn spawn(
	server: Server<'_>,
	definition: u16,
	origin: sys::Vector,
	classname: Option<&CStr>,
) -> Result<NonNull<sys::CBaseEntity>, WeaponError> {
	// SAFETY: Server guarantees its game factory and module stay loaded for
	// this callback, including loader metadata inspected during resolution.
	let targets = unsafe { platform::resolve(server.game_server_factory().as_raw() as usize) }
		.ok_or(WeaponError::NativeUnavailable)?;
	type Schema = unsafe extern "C" fn() -> *mut c_void;
	type Definition = unsafe extern "C" fn(*mut c_void, i32) -> *mut c_void;
	type Spawn = unsafe extern "C" fn(
		*mut c_void,
		i32,
		*const sys::Vector,
		*const sys::QAngle,
		i32,
		i32,
		*const c_char,
	) -> *mut sys::CBaseEntity;
	// SAFETY: The resolver verifies these functions in the callback's game
	// module. Linux uses exact mangled signatures and identical live/file code;
	// Windows verifies the native call chain and the argument setup at its
	// schema lookup. No item-view or schema layout is manufactured by Rust.
	let get_schema: Schema = unsafe { std::mem::transmute(targets.schema) };
	let get_definition: Definition = unsafe { std::mem::transmute(targets.definition) };
	let generate: Spawn = unsafe { std::mem::transmute(targets.spawn) };
	let schema = unsafe { get_schema() };
	if schema.is_null() {
		return Err(WeaponError::NativeUnavailable);
	}
	// Windows' validated SpawnItem callsite adds eight bytes to ItemSystem's
	// result; Linux resolves GetItemSchema itself, whose adjustment is zero.
	let schema = unsafe { schema.byte_add(targets.schema_offset) };
	let fallback = unsafe { get_definition(schema, -1) };
	let item = unsafe { get_definition(schema, i32::from(definition)) };
	// Unknown indices return the default item, which could otherwise create
	// an unrelated entity. Schema loading excludes all negative indices.
	if item.is_null() || item == fallback {
		return Err(WeaponError::CreationFailed);
	}
	let angles = sys::QAngle {
		x: 0.0,
		y: 0.0,
		z: 0.0,
	};
	// SAFETY: Native generation initializes the embedded CEconItemView and
	// invokes Spawn/Activate. The caller guarantees those callbacks preserve
	// Server's lifetime contract. Level one / unique quality match the native
	// GenerateItemFromDefIndex wrapper. Inputs live through this call.
	let entity = unsafe {
		generate(
			targets.singleton as *mut c_void,
			i32::from(definition),
			&origin,
			&angles,
			1,
			6,
			classname.map_or(std::ptr::null(), CStr::as_ptr),
		)
	};
	NonNull::new(entity).ok_or(WeaponError::CreationFailed)
}

#[cfg(target_os = "windows")]
mod platform {
	use super::*;
	use sdk_raw::util::{Image, SignaturePattern, pattern, relative, sig};

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
				std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE"))
					.unwrap();

			let mut image = sdk_raw::util::pe::from_file(&bytes).unwrap();

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
}

#[cfg(target_os = "linux")]
mod platform {
	use super::Targets;
	use sdk_raw::util::{elf::LoadedElf, relative};

	/// The caller keeps the factory's game module loaded for all resolution and
	/// subsequent native calls, under the callback-scoped Server contract.
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
		use sdk_raw::util::elf::Elf;

		#[test]
		#[ignore = "set TF2_SERVER_IMAGE to an authorized unstripped retail server_srv.so for binary validation"]
		fn retail_item_generation_symbols() {
			let bytes =
				std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE"))
					.unwrap();
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
}
