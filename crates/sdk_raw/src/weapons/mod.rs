//! Retail TF2 item generation. Resolve only the supported seven-argument
//! CItemGeneration::SpawnItem ABI; newer SDK headers add a class argument.
//! All inspection uses owned snapshots. Missing/ambiguous signatures, stripped
//! ELF symbols, or a changed file fail closed before calling native code.

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::NonNull;

struct Targets {
	spawn: usize,
	singleton: usize,
	schema: usize,
	schema_offset: usize,
	definition: usize,
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("Failed to create weapon")]
pub struct WeaponCreationFailed(());

/// The caller supplies the same spawn/callback lifetime guarantees as `give`.
pub unsafe fn spawn(
	factory: unsafe extern "C" fn(name: *const c_char, return_code: *mut c_int) -> *mut c_void,
	definition: u16,
	origin: sys::Vector,
	classname: Option<&CStr>,
) -> Result<NonNull<sys::CBaseEntity>, WeaponCreationFailed> {
	// SAFETY: Server guarantees its game factory and module stay loaded for
	// this callback, including loader metadata inspected during resolution.
	let targets = unsafe { platform::resolve(factory as usize) }.expect("Failed to find ");

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

	assert!(!schema.is_null(), "");

	// Windows' validated SpawnItem callsite adds eight bytes to ItemSystem's
	// result; Linux resolves GetItemSchema itself, whose adjustment is zero.
	let schema = unsafe { schema.byte_add(targets.schema_offset) };
	let fallback = unsafe { get_definition(schema, -1) };
	let item = unsafe { get_definition(schema, i32::from(definition)) };

	// Unknown indices return the default item, which could otherwise create
	// an unrelated entity. Schema loading excludes all negative indices.
	if item.is_null() || item == fallback {
		return Err(WeaponCreationFailed(()));
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

	NonNull::new(entity).ok_or(WeaponCreationFailed(()))
}
