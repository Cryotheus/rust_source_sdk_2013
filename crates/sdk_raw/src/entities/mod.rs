//! Hand-written ABI of `CBaseEntity`: the vtable slots of methods that the
//! generated bindings do not give for every game, calls through them, and
//! header values describing entities and their handles. Data description maps
//! are in [`datamap`].
//!
//! The generated `CBaseEntity` vtable is TF2's. Slots are numbered from the
//! primary vtable's first entry, counting each destructor slot, under the
//! target's C++ ABI.

pub mod datamap;

use crate::edicts::MAX_EDICT_BITS;
use crate::util::vtable::vtable_pointer;
use crate::{vcall, vtable_slot};
use datamap::DataMaps;
use std::ffi::{CStr, c_int};
use std::mem::transmute;

/// `CBaseEntity::Teleport`, which moves an entity and sets each of its origin,
/// angles, and velocity that is not null.
#[doc(alias("Teleport"))]
pub type TeleportFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	origin: *const sys::Vector,
	angles: *const sys::QAngle,
	velocity: *const sys::Vector,
);

// TF2's slots of these methods match the generated vtable on each ABI.
const _: () = {
	use sys::CBaseEntity__bindgen_vtable as Vtable;

	assert!(ACCEPT_INPUT_SLOT == vtable_slot!(Vtable, CBaseEntity_AcceptInput));
	assert!(GET_DATA_DESC_MAP_SLOT == vtable_slot!(Vtable, CBaseEntity_GetDataDescMap));
};

// The generated `Teleport` has the hand-written signature.
const _: fn(&sys::CBaseEntity__bindgen_vtable) -> TeleportFn = |vtable| vtable.CBaseEntity_Teleport;

/// `CBaseEntity::AcceptInput` in the primary vtable. TF2 declares its own
/// virtual methods after it, so the slot is the same for every game.
/// Derived from `game/server/baseentity.h` with the MSVC ABI model on Windows
/// and the Itanium ABI model on Linux, and verified against SourceMod's
/// `sdktools.games/game.tf.txt` gamedata.
#[doc(alias("AcceptInput"))]
pub const ACCEPT_INPUT_SLOT: usize = cfg_select! {
	target_os = "windows" => 38,
	target_os = "linux" => 39,
};

/// An exclusive bound on the offsets of `CBaseEntity`'s own fields, past which
/// an offset its datamap gives is not trusted.
pub const BASE_ENTITY_FIELD_OFFSET_LIMIT: usize = 8192;

/// `EFL_KILLME` from `game/shared/shareddefs.h`: the bit of `m_iEFlags` set
/// while the entity is marked for deferred deletion.
pub const EFL_KILLME: c_int = 1 << 0;

/// `ENT_ENTRY_MASK` from `public/const.h`: the bits of a `CBaseHandle`'s raw
/// value holding the entity's slot in the entity list.
pub const ENT_ENTRY_MASK: u32 = (1 << NUM_SERIAL_NUM_BITS) - 1;

/// `CBaseEntity::GetDataDescMap` in the Source SDK 2013 primary vtable.
/// Derived from `game/server/cbase.h` with the MSVC ABI model on Windows and
/// the Itanium ABI model on Linux.
#[doc(alias("GetDataDescMap"))]
pub const GET_DATA_DESC_MAP_SLOT: usize = cfg_select! {
	target_os = "windows" => 11,
	target_os = "linux" => 12,
};

/// `INVALID_EHANDLE_INDEX` from `public/const.h`: the raw value of a
/// `CBaseHandle` that refers to no entity.
pub const INVALID_EHANDLE_INDEX: u32 = 0xFFFF_FFFF;

/// `INVALID_NETWORKED_EHANDLE_VALUE` from `public/const.h`: the value
/// `SendProxy_EHandleToInt` networks for a handle that refers to no entity.
pub const INVALID_NETWORKED_EHANDLE_VALUE: u32 = (1 << NUM_NETWORKED_EHANDLE_BITS) - 1;

/// `NUM_ENT_ENTRIES` from `public/const.h`: the number of slots in the entity
/// list, networked or not.
pub const NUM_ENT_ENTRIES: usize = 1 << NUM_ENT_ENTRY_BITS;

/// `NUM_ENT_ENTRY_BITS` from `public/const.h`: the bits that index the entity
/// list.
pub const NUM_ENT_ENTRY_BITS: u32 = MAX_EDICT_BITS + 2;

/// `NUM_NETWORKED_EHANDLE_BITS` from `public/const.h`: the bits of a networked
/// handle, its edict index in the low [`MAX_EDICT_BITS`] and its serial number
/// above them.
pub const NUM_NETWORKED_EHANDLE_BITS: u32 =
	MAX_EDICT_BITS + NUM_NETWORKED_EHANDLE_SERIAL_NUMBER_BITS;

/// `NUM_NETWORKED_EHANDLE_SERIAL_NUMBER_BITS` from `public/const.h`: the low
/// bits of a handle's serial number that a networked handle keeps.
pub const NUM_NETWORKED_EHANDLE_SERIAL_NUMBER_BITS: u32 = 10;

/// `NUM_SERIAL_NUM_BITS` from `public/const.h`: the bits of a `CBaseHandle`'s
/// serial number.
pub const NUM_SERIAL_NUM_BITS: u32 = 16;

/// `NUM_SERIAL_NUM_SHIFT_BITS` from `public/const.h`: how far a
/// `CBaseHandle`'s raw value shifts its serial number up, above the entity's
/// slot.
pub const NUM_SERIAL_NUM_SHIFT_BITS: u32 = 32 - NUM_SERIAL_NUM_BITS;

/// `CBaseEntity::Teleport` in the generic Source SDK 2013 game DLL.
/// This preserves the non-TF game layout; generated entity types use TF2.
#[doc(alias("Teleport"))]
pub const SDK2013_TELEPORT_SLOT: usize = cfg_select! {
	target_os = "windows" => 110,
	target_os = "linux" => 111,
};

/// `CBaseEntity::Teleport` in TF2's game DLL.
/// Verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
#[doc(alias("Teleport"))]
pub const TF2_TELEPORT_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_Teleport);

/// Where a game DLL's primary `CBaseEntity` vtable has `Teleport`, which
/// virtual methods TF2 declares before it move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TeleportSlot {
	/// [`TF2_TELEPORT_SLOT`], the generated vtable's.
	TeamFortress2,

	/// [`SDK2013_TELEPORT_SLOT`], that of a Source SDK 2013 game DLL built
	/// without game-specific virtual methods.
	SourceSdk2013,
}

/// Calls `CBaseEntity::AcceptInput`, which runs the input's handler before
/// returning, and returns whether the entity found the input and converted
/// the value to the input's type.
///
/// The method is called through the generated vtable, whose slot of it,
/// [`ACCEPT_INPUT_SLOT`], every game DLL shares. `variant_t` has a
/// user-provided copy constructor, through its `CHandle`, so both ABIs pass it
/// by address: `value` points to the caller's copy, which `AcceptInput` may
/// convert in place.
///
/// # Safety
///
/// - `entity` must point to a live `CBaseEntity` of the loaded game DLL, and
///   the call must be made on the server's main thread.
/// - `activator` and `caller` must each be null or point to a live
///   `CBaseEntity`, and the input's handler must accept them: some handlers
///   dereference them without checking.
/// - `value` must point to a valid `variant_t` for the call, and a string it
///   holds must stay valid for as long as the handler may keep it, as a
///   pooled string does.
/// - The input, and every game function it runs, must free entities only
///   through deferred deletion.
#[doc(alias("AcceptInput"))]
pub unsafe fn accept_input(
	entity: *mut sys::CBaseEntity,
	input: &CStr,
	activator: *mut sys::CBaseEntity,
	caller: *mut sys::CBaseEntity,
	value: *mut sys::variant_t,
	output_id: c_int,
) -> bool {
	// SAFETY: The entity is live, and every game DLL's vtable has
	// `AcceptInput` where the generated one does. The name is only read during
	// the call, and the caller upholds the rest.
	unsafe {
		vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_AcceptInput(
			input.as_ptr(),
			activator,
			caller,
			value,
			output_id,
		))
	}
}

/// Calls `CBaseEntity::GetDataDescMap`, which returns the first of the data
/// description maps of the entity's class, as [`DataMaps::new`] takes them.
///
/// The method is called through the generated vtable, whose slot of it,
/// [`GET_DATA_DESC_MAP_SLOT`], every game DLL shares.
///
/// # Safety
///
/// `entity` must point to a live `CBaseEntity` of the loaded game DLL.
#[doc(alias("GetDataDescMap"))]
pub unsafe fn data_desc_map(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: The entity is live, and every game DLL's vtable has
	// `GetDataDescMap` where the generated one does.
	unsafe { vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_GetDataDescMap()) }
}

/// Finds the offset of the field named `name`, of `field_type` and `size`
/// bytes, that `CBaseEntity`'s own map in `maps` declares.
///
/// Returns `None` unless the field lies under
/// [`BASE_ENTITY_FIELD_OFFSET_LIMIT`], at an offset aligned for its size, up
/// to a pointer's alignment.
pub fn find_base_entity_field(
	mut maps: DataMaps<'_>,
	name: &CStr,
	field_type: sys::fieldtype_t,
	size: usize,
) -> Option<usize> {
	let map = maps.find(|map| map.class_name() == Some(c"CBaseEntity"))?;

	let field = map.fields().iter().find(|field| {
		field.fieldType == field_type
			&& usize::try_from(field.fieldSizeInBytes) == Ok(size)
			&& field.name() == Some(name)
	})?;

	let offset = field.offset()?;
	let is_aligned = offset.is_multiple_of(size.min(align_of::<*const ()>()));

	(offset < BASE_ENTITY_FIELD_OFFSET_LIMIT && is_aligned).then_some(offset)
}

/// Calls `CBaseEntity::Teleport`, at `slot` of the entity's primary vtable,
/// with each of the origin, angles, and velocity to set, or null to leave it.
///
/// TF2's slot is called through the generated vtable, and the generic Source
/// SDK 2013 game DLL's through a [`TeleportFn`] read from its slot.
///
/// # Safety
///
/// - `entity` must point to a live `CBaseEntity` of the loaded game DLL,
///   whose primary vtable has `Teleport` at `slot`, and the call must be made
///   on the server's main thread.
/// - `origin`, `angles`, and `velocity` must each be null or valid for reads
///   during the call.
/// - The game functions moving the entity runs, such as those of its physics
///   and children, must free entities only through deferred deletion.
#[doc(alias("Teleport"))]
pub unsafe fn teleport(
	entity: *mut sys::CBaseEntity,
	slot: TeleportSlot,
	origin: *const sys::Vector,
	angles: *const sys::QAngle,
	velocity: *const sys::Vector,
) {
	match slot {
		// SAFETY: The entity is live, and its vtable is TF2's, as generated.
		// The caller upholds the rest.
		TeleportSlot::TeamFortress2 => unsafe {
			vcall!(entity as sys::CBaseEntity__bindgen_vtable => CBaseEntity_Teleport(
				origin, angles, velocity,
			))
		},

		TeleportSlot::SourceSdk2013 => {
			// SAFETY: The live entity starts with the pointer to its primary
			// vtable, which has `Teleport`, with the generated signature, at
			// this slot.
			let teleport = unsafe {
				let slots = vtable_pointer::<*const ()>(entity);

				transmute::<*const (), TeleportFn>(slots.add(SDK2013_TELEPORT_SLOT).read())
			};

			// SAFETY: As above; the caller upholds the rest.
			unsafe { teleport(entity, origin, angles, velocity) }
		}
	}
}
