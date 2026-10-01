#![cfg_attr(docsrs, feature(doc_cfg))]

mod generated;

use std::mem::offset_of;

pub use generated::*;

/// `CBaseEntity::AcceptInput` in the primary vtable. TF2 declares its own
/// virtual methods after it, so the slot is the same for every game.
/// Derived from `game/server/baseentity.h` with the Itanium ABI model, and
/// verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
pub const CBASEENTITY_ACCEPTINPUT_VTABLE_SLOT: usize = 39;

/// `CBaseEntity::GetDataDescMap` in the Source SDK 2013 primary vtable.
/// Derived from `game/server/cbase.h` with the Itanium ABI model.
pub const CBASEENTITY_DATAMAP_VTABLE_SLOT: usize = 12;

/// `CBaseEntity::Teleport` in the generic Source SDK 2013 game DLL.
/// This preserves the non-TF game layout; generated entity types use TF2.
pub const CBASEENTITY_TELEPORT_VTABLE_SLOT: usize = 111;

/// `CBaseEntity::Teleport` in TF2's game DLL.
/// Verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
pub const CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT: usize =
	offset_of!(CBaseEntity__bindgen_vtable, CBaseEntity_Teleport) / SLOT_SIZE;

const SLOT_SIZE: usize = size_of::<*const ()>();
