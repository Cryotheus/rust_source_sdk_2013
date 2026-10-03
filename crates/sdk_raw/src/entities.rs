//! Hand-written ABI of `CBaseEntity`, such as the vtable slots of methods that
//! the generated bindings do not give for every game.
//!
//! The generated `CBaseEntity` vtable is TF2's. Slots are numbered from the
//! primary vtable's first entry, counting each destructor slot, under the
//! target's C++ ABI.

use crate::vtable_slot;

// TF2's slots of these methods match the generated vtable on each ABI.
const _: () = {
	use sys::CBaseEntity__bindgen_vtable as Vtable;

	assert!(ACCEPT_INPUT_SLOT == vtable_slot!(Vtable, CBaseEntity_AcceptInput));
	assert!(GET_DATA_DESC_MAP_SLOT == vtable_slot!(Vtable, CBaseEntity_GetDataDescMap));
};

/// `CBaseEntity::AcceptInput` in the primary vtable. TF2 declares its own
/// virtual methods after it, so the slot is the same for every game.
/// Derived from `game/server/baseentity.h` with the MSVC ABI model on Windows
/// and the Itanium ABI model on Linux, and verified against SourceMod's
/// `sdktools.games/game.tf.txt` gamedata.
#[doc(alias = "AcceptInput")]
pub const ACCEPT_INPUT_SLOT: usize = cfg_select! {
	target_os = "windows" => 38,
	target_os = "linux" => 39,
};

/// `CBaseEntity::GetDataDescMap` in the Source SDK 2013 primary vtable.
/// Derived from `game/server/cbase.h` with the MSVC ABI model on Windows and
/// the Itanium ABI model on Linux.
#[doc(alias = "GetDataDescMap")]
pub const GET_DATA_DESC_MAP_SLOT: usize = cfg_select! {
	target_os = "windows" => 11,
	target_os = "linux" => 12,
};

/// `CBaseEntity::Teleport` in the generic Source SDK 2013 game DLL.
/// This preserves the non-TF game layout; generated entity types use TF2.
#[doc(alias = "Teleport")]
pub const SDK2013_TELEPORT_SLOT: usize = cfg_select! {
	target_os = "windows" => 110,
	target_os = "linux" => 111,
};

/// `CBaseEntity::Teleport` in TF2's game DLL.
/// Verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
#[doc(alias = "Teleport")]
pub const TF2_TELEPORT_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_Teleport);
