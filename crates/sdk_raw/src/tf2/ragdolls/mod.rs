//! TF2's server-side ragdolls: `CreateServerRagdoll`
//! (`game/server/physics_prop_ragdoll.cpp`), which turns an animating entity's
//! current pose into a new `prop_ragdoll` whose physics the server simulates,
//! the header values describing such ragdolls, and where a `prop_ragdoll`
//! keeps its physics objects ([`find_ragdoll_object_fields`]).
//!
//! TF2's players instead become ragdolls their clients simulate, through
//! `BecomeRagdollOnClient` and `CTFPlayer::CreateRagdollEntity`. The game
//! exports no `CreateServerRagdoll`, so [`ServerRagdolls::resolve`] finds it in
//! the game server module: on Windows through a signature whose independent
//! native callers must agree, and on Linux through its mangled symbol, which
//! needs an unstripped `server_srv.so`. A missing or ambiguous signature,
//! disagreeing callers, a stripped symbol, or a file that differs from the
//! loaded module fail closed before any native code is called.
//!
//! `CreateServerRagdoll` is a free C++ function, called as `extern "C"`, which
//! is its calling convention on both supported x86-64 targets.
//!
//! # Caching
//!
//! [`ServerRagdolls::cached`] keeps the last successful resolution for the rest
//! of the process in a [`ModuleCache`], keyed by the address of the factory and
//! the base address of the module containing it, as
//! [`ItemGeneration::cached`](crate::tf2::item_generation::ItemGeneration::cached)
//! does, under the same assumption: Source never unloads the game server
//! module while plugins are loaded, so a module at the same base with its
//! `CreateInterface` at the same address is the image that was inspected.
//! Failures are not kept.

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;

#[cfg(test)]
#[path = "../../tests/tf2/ragdolls/mod.rs"]
mod tests;

use crate::entities::datamap::DataMap;
use crate::interfaces::CreateInterfaceFn;
use crate::util::{ModuleCache, ModuleKey};
use std::ffi::{CStr, c_int};
use std::mem::transmute;
use std::ptr::{self, NonNull};

/// `CreateServerRagdoll(CBaseAnimating *, int, const CTakeDamageInfo &, int,
/// bool)`, which returns the new ragdoll.
#[doc(alias("CreateServerRagdoll"))]
pub type CreateServerRagdollFn = unsafe extern "C" fn(
	animating: *mut sys::CBaseAnimating,
	force_bone: c_int,
	info: *const sys::CTakeDamageInfo,
	collision_group: c_int,
	use_lru_retirement: bool,
) -> *mut sys::CBaseEntity;

// `ServerRagdolls` is a plain function address, which only the unsafe
// `create` uses, so it can be copied and shared between threads.
const _: () = {
	const fn assert_plain<T: Copy + Send + Sync>() {}

	assert_plain::<ServerRagdolls>();
};

/// The force bone [`ServerRagdolls::create`] takes for none, which applies
/// `CreateServerRagdoll`'s damage force to no single physics object. The game
/// (`RagdollCreate`) treats any index outside the ragdoll's elements the same.
pub const NO_FORCE_BONE: c_int = -1;

/// The most solids a ragdoll's collision model may have. The game creates no
/// physics objects for a model with more, nor for one without a collision
/// model, and a `prop_ragdoll` made from it has no physics.
///
/// This is `RAGDOLL_MAX_ELEMENTS` from `game/shared/ragdoll_shared.h`.
pub const RAGDOLL_MAX_ELEMENTS: c_int = 24;

/// How many of a ragdoll's physics objects its map declares: every element's
/// but the first.
const RAGDOLL_OBJECT_MEMBERS: usize = RAGDOLL_MAX_ELEMENTS as usize - 1;

/// The class of a `prop_ragdoll` (`CRagdollProp`), as its data description
/// map names it.
#[doc(alias("CRagdollProp", "prop_ragdoll"))]
pub const RAGDOLL_PROP_CLASS: &CStr = c"CRagdollProp";

/// The offset under which [`find_ragdoll_object_fields`] trusts a member of a
/// `prop_ragdoll` to lie, well past the size of the class.
pub const RAGDOLL_PROP_FIELD_OFFSET_LIMIT: usize = 16384;

/// The `prop_ragdoll` spawn flag that lets it be dissolved.
pub const SF_RAGDOLLPROP_ALLOW_DISSOLVE: c_int = 0x2000;

/// The `prop_ragdoll` spawn flag that lets it stretch its constraints.
///
/// This is `SF_RAGDOLLPROP_ALLOW_STRETCH` from
/// `game/server/physics_prop_ragdoll.cpp`, as are the other `SF_RAGDOLLPROP_*`
/// values.
pub const SF_RAGDOLLPROP_ALLOW_STRETCH: c_int = 0x8000;

/// The `prop_ragdoll` spawn flag that spawns it as debris, which collides only
/// with the world and static props.
pub const SF_RAGDOLLPROP_DEBRIS: c_int = 0x0004;

/// The `prop_ragdoll` spawn flag that spawns it with motion disabled.
pub const SF_RAGDOLLPROP_MOTIONDISABLED: c_int = 0x4000;

/// The `prop_ragdoll` spawn flag that spawns it asleep.
pub const SF_RAGDOLLPROP_STARTASLEEP: c_int = 0x10000;

/// The `prop_ragdoll` spawn flag of the ragdolls the game retires when it
/// keeps too many, by fading them out. `CreateServerRagdoll` sets it when
/// asked to use this retirement.
pub const SF_RAGDOLLPROP_USE_LRU_RETIREMENT: c_int = 0x1000;

/// The last resolution [`ServerRagdolls::cached`] kept, with the module it
/// was resolved in.
static CACHE: ModuleCache<usize> = ModuleCache::new();

/// Where a `prop_ragdoll` keeps its physics objects, in its `m_ragdoll`, as
/// [`find_ragdoll_object_fields`] finds them.
///
/// The ragdoll has one physics object for each element of its list in use,
/// the first `m_ragdoll.listCount` of [`RAGDOLL_MAX_ELEMENTS`]. The ragdoll's
/// map declares only the objects of elements 1 on. Element 0's is taken to be
/// the entity's own `m_pPhysicsObject`, which [`find_physics_object_field`]
/// finds, as it is the ragdoll's first part, though nothing declares that.
///
/// [`find_physics_object_field`]: crate::entities::find_physics_object_field
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RagdollObjectFields {
	/// The offset of `m_ragdoll.listCount`, an `int`: how many elements of the
	/// list are in use.
	pub count: usize,

	/// The offsets of `m_ragdoll.list[1].pObject` to
	/// `m_ragdoll.list[23].pObject`, in order: each an `IPhysicsObject *`, null
	/// for none.
	pub objects: [usize; RAGDOLL_OBJECT_MEMBERS],
}

/// TF2's `CreateServerRagdoll`, resolved in a game server module.
///
/// It is a plain function address the resolver verified, so it is `Copy`,
/// `Send`, and `Sync`: only [`Self::create`], which must be called on the
/// server's main thread, uses it. Holding one does not keep that module
/// loaded: its function may only be called while the module it was resolved
/// in stays loaded.
#[doc(alias("CreateServerRagdoll"))]
#[derive(Debug, Clone, Copy)]
pub struct ServerRagdolls {
	create_server_ragdoll: CreateServerRagdollFn,
}

impl ServerRagdolls {
	/// Finds `CreateServerRagdoll` in the module whose `CreateInterface` export
	/// is `factory`, as [`Self::resolve`] does, but inspects each module only
	/// once, as the [module documentation](self#caching) describes.
	///
	/// Fails with [`ServerRagdollsError::Unresolved`] if no loaded module
	/// contains `factory`, or if resolution fails. A failure is not kept: the
	/// next call resolves again.
	///
	/// # Safety
	///
	/// - `factory` must be the `CreateInterface` export of a module that stays
	///   loaded, with its image mappings unchanged, for the whole call.
	/// - If an earlier call resolved a module mapped at the same base, with its
	///   `CreateInterface` at the same address, the module containing `factory`
	///   must be that same image, with the code that call found unchanged,
	///   since a cache hit returns that call's address without inspecting the
	///   module. This holds under the assumption [`ModuleCache`] describes.
	pub unsafe fn cached(factory: CreateInterfaceFn) -> Result<Self, ServerRagdollsError> {
		let factory = factory as usize;

		// SAFETY: The caller keeps the factory's module loaded for this call.
		let key = unsafe { ModuleKey::of(factory) }.map_err(|_| ServerRagdollsError::Unresolved)?;

		let address = CACHE.get_or_resolve(key, || {
			// SAFETY: The caller keeps the factory's module loaded, with its image
			// mappings unchanged, for this call, including the loader metadata the
			// resolver inspects.
			unsafe { platform::resolve(factory) }.ok_or(ServerRagdollsError::Unresolved)
		})?;

		// SAFETY: The resolver verified the address in this module: in this call,
		// or, on a cache hit, in an earlier call, for a module mapped at the same
		// base with the same factory, which the caller guarantees is this same,
		// unchanged image, still mapped for this call.
		Ok(unsafe { Self::from_address(address) })
	}

	/// Types the address a platform resolver verified.
	///
	/// # Safety
	///
	/// A platform resolver verified `address` in a module whose image is still
	/// mapped at the addresses it inspected.
	unsafe fn from_address(address: usize) -> Self {
		// SAFETY: The resolver verified the address as the entry of
		// `CreateServerRagdoll` in the module's executable code: on Windows
		// through its prologue and two native callers, on Linux through its exact
		// mangled symbol, whose live code matches the file. It is not null, since
		// it lies in a section of the module.
		unsafe {
			Self {
				create_server_ragdoll: transmute::<*const (), CreateServerRagdollFn>(
					ptr::with_exposed_provenance(address),
				),
			}
		}
	}

	/// Finds `CreateServerRagdoll` in the module whose `CreateInterface` export
	/// is `factory`.
	///
	/// Fails with [`ServerRagdollsError::Unresolved`] unless that module is a
	/// game server module whose function matches retail TF2's, as the
	/// [module documentation](self) describes. Nothing is cached: each call
	/// inspects the module again, while [`Self::cached`] keeps the result.
	///
	/// # Safety
	///
	/// `factory` must be the `CreateInterface` export of a module that stays
	/// loaded, with its image mappings unchanged, for the whole call.
	pub unsafe fn resolve(factory: CreateInterfaceFn) -> Result<Self, ServerRagdollsError> {
		// SAFETY: The caller keeps the factory's module loaded, with its image
		// mappings unchanged, for this call, including the loader metadata the
		// resolver inspects.
		let address = unsafe { platform::resolve(factory as usize) }
			.ok_or(ServerRagdollsError::Unresolved)?;

		// SAFETY: The resolver just verified the address in this module, which
		// the caller keeps mapped.
		Ok(unsafe { Self::from_address(address) })
	}

	/// Calls `CreateServerRagdoll`, which creates a `prop_ragdoll` without
	/// spawning it, in `animating`'s current pose and with its velocity, and
	/// applies `info`'s damage force to the physics object of `force_bone`, or
	/// to none for [`NO_FORCE_BONE`], and to the others from `info`'s damage
	/// position unless it is the origin. The ragdoll is put in
	/// `collision_group`, a `COLLISION_GROUP_*` value, and owned by
	/// `animating`.
	///
	/// With `use_lru_retirement`, the game adds the ragdoll to the ones it
	/// retires by fading them out once it keeps too many. Without it, the
	/// ragdoll stays until it is removed, as the round's cleanup does.
	///
	/// The ragdoll copies `animating`'s model, skin, body groups, sequence,
	/// cycle, and effects, such as `EF_NODRAW`, which keeps the engine from
	/// sending it to clients. Returns `None` if the game created no ragdoll.
	///
	/// The engine ends the process with a fatal error if it has no edict left
	/// for the new entity, as for every entity created.
	///
	/// # Safety
	///
	/// - The module this was resolved in, by [`Self::resolve`] or
	///   [`Self::cached`], is still loaded, with its image mappings unchanged,
	///   for the whole call.
	/// - The call is made on the server's main thread, from a callback in which
	///   the game may create entities, and outside of VPhysics' simulation and
	///   its callbacks, since the call creates physics objects.
	/// - `animating` points to a live `CBaseAnimating` of that module, not
	///   marked for deletion, whose model is a studio model with a collision
	///   model of 1 to [`RAGDOLL_MAX_ELEMENTS`] solids.
	/// - `info` points to a `CTakeDamageInfo` that stays live and unchanged for
	///   the whole call.
	/// - Everything the call runs, including the entity factories it calls and
	///   whatever their constructors reach, frees entities only through the
	///   engine's deferred deletion.
	#[doc(alias("CreateServerRagdoll"))]
	pub unsafe fn create(
		&self,
		animating: NonNull<sys::CBaseAnimating>,
		force_bone: c_int,
		info: NonNull<sys::CTakeDamageInfo>,
		collision_group: c_int,
		use_lru_retirement: bool,
	) -> Option<NonNull<sys::CBaseEntity>> {
		// SAFETY: The caller keeps the module loaded, calls on the main thread,
		// and vouches for the entity, the damage record, and the game code the
		// call runs. The function reads the damage record and creates and
		// initializes a new entity.
		NonNull::new(unsafe {
			(self.create_server_ragdoll)(
				animating.as_ptr(),
				force_bone,
				info.as_ptr(),
				collision_group,
				use_lru_retirement,
			)
		})
	}
}

/// Why TF2's `CreateServerRagdoll` could not be found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum ServerRagdollsError {
	/// The game server module's `CreateServerRagdoll` was not found, or did
	/// not match retail TF2's, as the [module documentation](self) describes.
	#[error("the game's CreateServerRagdoll could not be found")]
	Unresolved,
}

/// Finds where a `prop_ragdoll` keeps its physics objects, as `map`, the own
/// map of its class ([`RAGDOLL_PROP_CLASS`]), declares them:
/// `m_ragdoll.listCount` as one `FIELD_INTEGER` of 4 bytes, and each
/// `m_ragdoll.list[i].pObject`, for `i` from 1 to 23, as one `FIELD_CUSTOM`
/// member (`DEFINE_PHYSPTR`), whose size the map leaves out.
///
/// Fails closed, returning `None` unless the map declares the count and every
/// one of these objects exactly once, each under
/// [`RAGDOLL_PROP_FIELD_OFFSET_LIMIT`] and aligned for its type, with the
/// objects an element's size apart, as members of an array are, and the count
/// clear of them. A map that declares the object of element 0, or of an
/// element past the most a ragdoll has, is another layout, and gives `None`
/// too.
#[doc(alias("m_ragdoll", "listCount", "pObject"))]
pub fn find_ragdoll_object_fields(map: DataMap<'_>) -> Option<RagdollObjectFields> {
	let pointer = align_of::<*const ()>();
	let mut count = None;
	let mut objects = [None; RAGDOLL_OBJECT_MEMBERS];

	for field in map.fields() {
		let Some(name) = field.name() else {
			continue;
		};

		if name == c"m_ragdoll.listCount" {
			let offset = field.offset().filter(|&offset| {
				field.fieldType == sys::_fieldtypes_FIELD_INTEGER
					&& field.fieldSize == 1
					&& usize::try_from(field.fieldSizeInBytes) == Ok(size_of::<c_int>())
					&& offset < RAGDOLL_PROP_FIELD_OFFSET_LIMIT
					&& offset.is_multiple_of(align_of::<c_int>())
			})?;

			if count.replace(offset).is_some() {
				return None;
			}
		} else if let Some(element) = ragdoll_object_element(name.to_bytes()) {
			let offset = field.offset().filter(|&offset| {
				field.fieldType == sys::_fieldtypes_FIELD_CUSTOM
					&& field.fieldSize == 1
					&& offset < RAGDOLL_PROP_FIELD_OFFSET_LIMIT
					&& offset.is_multiple_of(pointer)
			})?;

			// Element 0's is the entity's own object, and those past the last
			// element are another layout's.
			let slot = objects.get_mut(element.checked_sub(1)?)?;

			if slot.replace(offset).is_some() {
				return None;
			}
		}
	}

	let count = count?;
	let mut found = [0; RAGDOLL_OBJECT_MEMBERS];

	for (found, object) in found.iter_mut().zip(objects) {
		*found = object?;
	}

	let first = found[0];
	let stride = found[1].checked_sub(first)?;

	// An element holds at least its object, and the array keeps the objects
	// aligned, so the elements lie a whole number of pointers apart.
	let in_array = stride >= pointer
		&& stride.is_multiple_of(pointer)
		&& (0..).zip(found).all(|(element, object)| {
			stride
				.checked_mul(element)
				.and_then(|distance| first.checked_add(distance))
				== Some(object)
		});

	let count_clear = found
		.iter()
		.all(|&object| count + size_of::<c_int>() <= object || object + pointer <= count);

	(in_array && count_clear).then_some(RagdollObjectFields {
		count,
		objects: found,
	})
}

/// The element `i` of a member named `m_ragdoll.list[i].pObject`, with `i` in
/// decimal without leading zeros, or `None` for any other name.
fn ragdoll_object_element(name: &[u8]) -> Option<usize> {
	let digits = name
		.strip_prefix(b"m_ragdoll.list[")?
		.strip_suffix(b"].pObject")?;

	if digits.is_empty()
		|| (digits.len() > 1 && digits[0] == b'0')
		|| !digits.iter().all(u8::is_ascii_digit)
	{
		return None;
	}

	digits.iter().try_fold(0_usize, |element, &digit| {
		element
			.checked_mul(10)?
			.checked_add(usize::from(digit - b'0'))
	})
}
