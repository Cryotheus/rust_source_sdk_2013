//! TF2's native item generation, `CItemGeneration::SpawnItem`, which creates
//! an economy item from its definition, initializes its item view, and spawns
//! it.
//!
//! The game exports none of the functions involved, so
//! [`ItemGeneration::resolve`] finds them in the game server module: on
//! Windows through signatures whose independent native callers must agree, and
//! on Linux through their mangled symbols, which need an unstripped
//! `server_srv.so`. Only retail's seven-argument `SpawnItem` is supported;
//! newer SDK headers add a class argument. All inspection uses owned
//! snapshots. Missing or ambiguous signatures, stripped ELF symbols, or a file
//! that differs from the loaded module fail closed before any native code is
//! called.
//!
//! The functions are C++ member functions, called as `extern "C"`: on both
//! supported x86-64 targets that is their calling convention, with `this`
//! passed as the first argument.
//!
//! # Caching
//!
//! Resolution inspects the whole module: on Windows it snapshots and scans its
//! image, and on Linux it reads and parses its file. [`ItemGeneration::cached`]
//! therefore keeps the last successful resolution for the rest of the process
//! in a [`ModuleCache`], keyed by the address of the factory and the base
//! address of the module containing it, which it looks up through the loader
//! on each call, and returns it again without inspecting the module. A call
//! for another factory or module base resolves anew and replaces it. Failures
//! are not kept, so the next call inspects the module again instead of
//! repeating an error that may have been transient, such as a failed read.
//!
//! The cache cannot tell a module from a different image later loaded at the
//! same base with its `CreateInterface` at the same address, and assumes, as
//! [`ModuleCache`] describes, that this does not happen.

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;

use crate::interfaces::CreateInterfaceFn;
use crate::sig;
use crate::util::{ModuleCache, ModuleKey, SignaturePattern};
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::transmute;
use std::num::NonZeroUsize;
use std::ptr::{self, NonNull};

/// `CEconItemSchema::GetItemDefinition(int)`.
type GetItemDefinitionFn = unsafe extern "C" fn(
	this: *mut sys::CEconItemSchema,
	index: c_int,
) -> *mut sys::CEconItemDefinition;

/// The getter of the item schema: `ItemSystem()` on Windows, whose
/// `CEconItemSystem` holds the schema `platform::SCHEMA_OFFSET` bytes in, and
/// `GetItemSchema()`, which returns the schema itself, on Linux.
type SchemaGetterFn = unsafe extern "C" fn() -> *mut c_void;

/// Retail's `CItemGeneration::SpawnItem(int, const Vector &, const QAngle &,
/// int, entityquality_t, const char *)`.
type SpawnItemFn = unsafe extern "C" fn(
	this: *mut sys::CItemGeneration,
	definition: c_int,
	origin: *const sys::Vector,
	angles: *const sys::QAngle,
	level: c_int,
	quality: sys::entityquality_t,
	classname: *const c_char,
) -> *mut sys::CBaseEntity;

// `ItemGeneration` is a plain bundle of addresses, which only the unsafe
// `spawn` uses, so it can be copied and shared between threads.
const _: () = {
	const fn assert_plain<T: Copy + Send + Sync>() {}

	assert_plain::<ItemGeneration>();
};

/// The body of `ItemGeneration()`, which returns the `CItemGeneration`
/// singleton: `lea rax, [rip + singleton]; ret`.
const ITEM_GENERATION_GETTER: &[SignaturePattern] = &sig![0x48 0x8d 0x05 ? ? ? ? 0xc3];

/// Where the `rip`-relative displacement of [`ITEM_GENERATION_GETTER`]'s
/// `lea` starts.
const ITEM_GENERATION_GETTER_OPERAND: usize = 3;

/// The level `CItemGeneration::GenerateItemFromDefIndex` gives the items it
/// creates.
const ITEM_LEVEL: c_int = 1;

/// The quality `CItemGeneration::GenerateItemFromDefIndex` gives the items it
/// creates: Unique.
const ITEM_QUALITY: sys::entityquality_t = sys::EEconItemQuality_AE_UNIQUE;

/// An index no item definition has, since the item schema rejects negative
/// indices when it loads. `GetItemDefinition` returns the schema's default
/// definition for it, as for every unknown index.
const NO_DEFINITION: c_int = -1;

/// How many bytes of the `CItemGeneration` singleton must lie in a writable
/// section of the module.
const SINGLETON_LEN: usize = 16;

/// The last resolution [`ItemGeneration::cached`] kept, with the module it was
/// resolved in.
static CACHE: ModuleCache<Addresses> = ModuleCache::new();

/// The addresses of the item generation functions and singleton in a module,
/// as the platform's resolver verified them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Addresses {
	/// `CEconItemSchema::GetItemDefinition`, a [`GetItemDefinitionFn`].
	get_item_definition: usize,

	/// The getter of the item schema, a [`SchemaGetterFn`].
	schema_getter: usize,

	/// The `CItemGeneration` singleton.
	singleton: NonZeroUsize,

	/// `CItemGeneration::SpawnItem`, a [`SpawnItemFn`].
	spawn_item: usize,
}

/// TF2's item generation functions, resolved in a game server module.
///
/// It is a plain bundle of the addresses the resolver verified, so it is
/// `Copy`, `Send`, and `Sync`: only [`Self::definition`] and [`Self::spawn`],
/// which must be called on the server's main thread, use them. Holding one
/// does not keep that module loaded: its functions may only be called while
/// the module it was resolved in stays loaded.
#[doc(alias("CItemGeneration"))]
#[derive(Debug, Clone, Copy)]
pub struct ItemGeneration {
	get_item_definition: GetItemDefinitionFn,
	schema_getter: SchemaGetterFn,
	singleton: NonZeroUsize,
	spawn_item: SpawnItemFn,
}

impl ItemGeneration {
	/// Finds TF2's item generation in the module whose `CreateInterface`
	/// export is `factory`, as [`Self::resolve`] does, but inspects each module
	/// only once.
	///
	/// If the last successful call was for the same factory, in a module at
	/// the same base address as the one containing `factory` now, this returns
	/// what that call resolved without inspecting the module again. Otherwise,
	/// it resolves the module and, on success, keeps the result in place of
	/// the previous one. Each call looks up the base of the module containing
	/// `factory` through the loader, as the
	/// [module documentation](crate::tf2::item_generation#caching) describes.
	///
	/// Fails with [`ItemGenerationError::Unresolved`] if no loaded module
	/// contains `factory`, or if resolution fails as for [`Self::resolve`]. A
	/// failure is not kept: the next call resolves again.
	///
	/// # Safety
	///
	/// - `factory` must be the `CreateInterface` export of a module that stays
	///   loaded, with its image mappings unchanged, for the whole call.
	/// - If an earlier call resolved a module mapped at the same base, with its
	///   `CreateInterface` at the same address, the module containing `factory`
	///   must be that same image, with the code and singleton that call found
	///   unchanged, since a cache hit returns that call's addresses without
	///   inspecting the module. This holds under the assumption
	///   [`ModuleCache`] describes.
	pub unsafe fn cached(factory: CreateInterfaceFn) -> Result<Self, ItemGenerationError> {
		let factory = factory as usize;

		// SAFETY: The caller keeps the factory's module loaded for this call.
		let key = unsafe { ModuleKey::of(factory) }.map_err(|_| ItemGenerationError::Unresolved)?;

		let addresses = CACHE.get_or_resolve(key, || {
			// SAFETY: The caller keeps the factory's module loaded, with its image
			// mappings unchanged, for this call, including the loader metadata the
			// resolver inspects.
			unsafe { platform::resolve(factory) }.ok_or(ItemGenerationError::Unresolved)
		})?;

		// SAFETY: The resolver verified the addresses in this module: in this
		// call, or, on a cache hit, in an earlier call, for a module mapped at
		// the same base with the same factory, which the caller guarantees is
		// this same, unchanged image, still mapped for this call.
		Ok(unsafe { Self::from_addresses(addresses) })
	}

	/// Types the addresses a platform resolver verified.
	///
	/// # Safety
	///
	/// A platform resolver verified `addresses` in a module whose image is
	/// still mapped at the addresses it inspected.
	unsafe fn from_addresses(addresses: Addresses) -> Self {
		// SAFETY: The resolver verified each address as the entry of the function
		// its type describes, in the module's executable code: on Windows through
		// the native call chain and the argument setup of its calls, on Linux
		// through exact mangled symbols whose live code matches the file. None is
		// null, since each lies in a section of the module.
		unsafe {
			Self {
				get_item_definition: transmute::<*const (), GetItemDefinitionFn>(
					ptr::with_exposed_provenance(addresses.get_item_definition),
				),
				schema_getter: transmute::<*const (), SchemaGetterFn>(
					ptr::with_exposed_provenance(addresses.schema_getter),
				),
				singleton: addresses.singleton,
				spawn_item: transmute::<*const (), SpawnItemFn>(ptr::with_exposed_provenance(
					addresses.spawn_item,
				)),
			}
		}
	}

	/// Finds TF2's item generation in the module whose `CreateInterface`
	/// export is `factory`.
	///
	/// Fails with [`ItemGenerationError::Unresolved`] unless that module is a
	/// game server module whose functions match retail TF2's, as the
	/// [module documentation](crate::tf2::item_generation) describes. Nothing
	/// is cached: each call inspects the module again, while [`Self::cached`]
	/// keeps the result.
	///
	/// # Safety
	///
	/// `factory` must be the `CreateInterface` export of a module that stays
	/// loaded, with its image mappings unchanged, for the whole call.
	pub unsafe fn resolve(factory: CreateInterfaceFn) -> Result<Self, ItemGenerationError> {
		// SAFETY: The caller keeps the factory's module loaded, with its image
		// mappings unchanged, for this call, including the loader metadata the
		// resolver inspects.
		let addresses = unsafe { platform::resolve(factory as usize) }
			.ok_or(ItemGenerationError::Unresolved)?;

		// SAFETY: The resolver just verified the addresses in this module, which
		// the caller keeps mapped.
		Ok(unsafe { Self::from_addresses(addresses) })
	}

	/// The item schema's definition with the index, through
	/// `CEconItemSchema::GetItemDefinition`.
	///
	/// Fails with [`ItemGenerationError::NoSchema`] before the game has an item
	/// schema, and with [`ItemGenerationError::UnknownDefinition`] if the schema
	/// has no definition with that index, for which `GetItemDefinition`
	/// returns the schema's default definition instead.
	///
	/// The definition is the schema's own. The game replaces the schema, and
	/// frees its definitions, when the Game Coordinator sends a newer one,
	/// which it applies at a level change, so the pointer is only valid during
	/// the callback it was found in.
	///
	/// # Safety
	///
	/// - The module this was resolved in, by [`Self::resolve`] or
	///   [`Self::cached`], is still loaded, with its image mappings unchanged,
	///   for the whole call.
	/// - The call is made on the server's main thread.
	#[doc(alias("GetItemDefinition"))]
	pub unsafe fn definition(
		&self,
		definition: u16,
	) -> Result<NonNull<sys::CEconItemDefinition>, ItemGenerationError> {
		// SAFETY: The caller keeps the resolved module loaded and calls on the
		// main thread. The getter takes no arguments, and returns the game's
		// item system or schema, or null before the game created it.
		let system = unsafe { (self.schema_getter)() };

		if system.is_null() {
			return Err(ItemGenerationError::NoSchema);
		}

		// SAFETY: The schema lies `SCHEMA_OFFSET` bytes into the object the
		// getter returns, which on Windows is the item system holding it, as
		// `SpawnItem`'s own call to `GetItemDefinition` passes it.
		let schema = unsafe { system.byte_add(platform::SCHEMA_OFFSET) }.cast();

		// SAFETY: `schema` points to the game's live item schema.
		// `GetItemDefinition` only looks the index up, returning the default
		// definition for an index it does not have.
		let (fallback, item) = unsafe {
			(
				(self.get_item_definition)(schema, NO_DEFINITION),
				(self.get_item_definition)(schema, c_int::from(definition)),
			)
		};

		// An unknown index gives the default item.
		if item == fallback {
			return Err(ItemGenerationError::UnknownDefinition);
		}

		NonNull::new(item).ok_or(ItemGenerationError::UnknownDefinition)
	}

	/// Creates the economy item `definition` at `origin` through
	/// `CItemGeneration::SpawnItem`, as `GenerateItemFromDefIndex` does: at
	/// level 1, with Unique quality, and without rotation. With `classname`,
	/// the item is created as that entity class instead of the definition's
	/// own, unless no entity factory has that name, in which case `SpawnItem`
	/// falls back to the definition's class.
	///
	/// The entity returned is newly created, spawned, and activated. It must
	/// not be spawned again.
	///
	/// Fails with [`ItemGenerationError::NoSchema`] before the game has an item
	/// schema, with [`ItemGenerationError::UnknownDefinition`] if the schema
	/// has no definition with that index, for which `SpawnItem` would create
	/// the schema's default item instead, and with
	/// [`ItemGenerationError::NotCreated`] if `SpawnItem` creates no entity.
	///
	/// # Safety
	///
	/// - The module this was resolved in, by [`Self::resolve`] or
	///   [`Self::cached`], is still loaded, with its image mappings unchanged,
	///   for the whole call.
	/// - The call is made on the server's main thread, from a callback in which
	///   the game may create and spawn entities.
	/// - The game code the call runs, such as the item's constructor, `Spawn`,
	///   and `Activate` and everything they reach, frees entities only through
	///   the engine's deferred deletion.
	/// - `classname`, if given, names an entity class compatible with
	///   `definition`, since `SpawnItem` initializes the item of whatever entity
	///   it creates from `definition` without checking.
	#[doc(alias("SpawnItem", "GenerateItemFromDefIndex"))]
	pub unsafe fn spawn(
		&self,
		definition: u16,
		origin: sys::Vector,
		classname: Option<&CStr>,
	) -> Result<NonNull<sys::CBaseEntity>, ItemGenerationError> {
		// An unknown index gives the default item, which could otherwise create
		// an unrelated entity.
		//
		// SAFETY: The caller keeps the resolved module loaded and calls on the
		// main thread.
		unsafe { self.definition(definition) }?;

		let angles = sys::QAngle {
			x: 0.0,
			y: 0.0,
			z: 0.0,
		};

		// SAFETY: The singleton is the game's live `CItemGeneration`. Native
		// generation creates the entity, initializes its item view, and runs its
		// `Spawn` and `Activate`, whose game code the caller vouches for, as for
		// the classname. The definition exists, and the arguments live through
		// this call.
		let entity = unsafe {
			(self.spawn_item)(
				ptr::with_exposed_provenance_mut(self.singleton.get()),
				c_int::from(definition),
				&origin,
				&angles,
				ITEM_LEVEL,
				ITEM_QUALITY,
				classname.map_or(ptr::null(), CStr::as_ptr),
			)
		};

		NonNull::new(entity).ok_or(ItemGenerationError::NotCreated)
	}
}

/// Why TF2's item generation could not create an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum ItemGenerationError {
	/// `CItemGeneration::SpawnItem` created no entity, such as for a
	/// definition without an entity class.
	#[error("the game's item generation created no entity")]
	NotCreated,

	/// The game has no item schema yet.
	#[error("the game's item schema is not available")]
	NoSchema,

	/// The item schema has no definition with the index.
	#[error("the item schema has no such item definition")]
	UnknownDefinition,

	/// The game server module's item generation functions were not found, or
	/// did not match retail TF2's, as the
	/// [module documentation](crate::tf2::item_generation) describes.
	#[error("the game's item generation functions could not be found")]
	Unresolved,
}
