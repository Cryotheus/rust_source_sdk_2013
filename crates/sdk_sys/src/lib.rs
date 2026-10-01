//! Raw Source SDK bindings generated for TF2's server configuration.
//!
//! Game-side records include economy items and NextBot. Generated virtual
//! tables describe each class's primary address point for the target C++ ABI;
//! secondary base interfaces retain their own generated tables.

use std::mem::offset_of;

const SLOT_SIZE: usize = size_of::<*const ()>();

#[cfg(any(
	all(target_os = "linux", target_arch = "x86_64", target_env = "gnu"),
	all(target_os = "windows", target_arch = "x86_64", target_env = "msvc")
))]
const _: () = {
	type Equip = unsafe extern "C" fn(*mut CTFPlayer, *mut CBaseCombatWeapon);
	type GetSlot = unsafe extern "C" fn(*const CTFPlayer, i32) -> *mut CBaseCombatWeapon;
	type Remove = unsafe extern "C" fn(*mut CTFPlayer, *mut CBaseCombatWeapon) -> bool;
	type Give = unsafe extern "C" fn(
		*mut CTFPlayer,
		*const std::ffi::c_char,
		i32,
		*const CEconItemView,
		bool,
	) -> *mut CBaseEntity;
	type WeaponSlot = unsafe extern "C" fn(*const CTFWeaponBase) -> i32;

	let _: fn(&CTFPlayer__bindgen_vtable) -> Equip = |vtable| vtable.CTFPlayer_Weapon_Equip;
	let _: fn(&CTFPlayer__bindgen_vtable) -> GetSlot = |vtable| vtable.CTFPlayer_Weapon_GetSlot;
	let _: fn(&CTFPlayer__bindgen_vtable) -> Remove = |vtable| vtable.CTFPlayer_RemovePlayerItem;
	let _: fn(&CTFPlayer__bindgen_vtable) -> Give = |vtable| vtable.CTFPlayer_GiveNamedItem1;
	let _: fn(&CTFWeaponBase__bindgen_vtable) -> WeaponSlot = |vtable| vtable.CTFWeaponBase_GetSlot;
};

cfg_select! {
	all(target_os = "linux", target_arch = "x86", target_env = "gnu") => {
		compile_error!("Not yet supported");
	}

	all(target_os = "linux", target_arch = "x86_64", target_env = "gnu") => {
		mod linux_64;

		pub use linux_64::*;

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

		/// `CBaseEntity::AcceptInput` in the primary vtable. TF2 declares its own
		/// virtual methods after it, so the slot is the same for every game.
		/// Derived from `game/server/baseentity.h` with the Itanium ABI model, and
		/// verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
		pub const CBASEENTITY_ACCEPTINPUT_VTABLE_SLOT: usize = 39;
	}

	all(target_os = "windows", target_arch = "x86", target_env = "msvc") => {
		compile_error!("Not yet supported");
	}

	all(target_os = "windows", target_arch = "x86_64", target_env = "msvc") => {
		mod windows_64;

		pub use windows_64::*;

		/// `CBaseEntity::GetDataDescMap` in the Source SDK 2013 primary vtable.
		/// Derived from `game/server/cbase.h` with the MSVC ABI model.
		pub const CBASEENTITY_DATAMAP_VTABLE_SLOT: usize = 11;

		/// `CBaseEntity::Teleport` in the generic Source SDK 2013 game DLL.
		/// This preserves the non-TF game layout; generated entity types use TF2.
		pub const CBASEENTITY_TELEPORT_VTABLE_SLOT: usize = 110;

		/// `CBaseEntity::Teleport` in TF2's game DLL.
		/// Verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
		pub const CBASEENTITY_TF2_TELEPORT_VTABLE_SLOT: usize = offset_of!(CBaseEntity__bindgen_vtable, CBaseEntity_Teleport) / SLOT_SIZE;

		/// `CBaseEntity::AcceptInput` in the primary vtable. TF2 declares its own
		/// virtual methods after it, so the slot is the same for every game.
		/// Derived from `game/server/baseentity.h` with the MSVC ABI model, and
		/// verified against SourceMod's `sdktools.games/game.tf.txt` gamedata.
		pub const CBASEENTITY_ACCEPTINPUT_VTABLE_SLOT: usize = 38;
	}

	_ => {
		compile_error!("Unsupported target");
	}
}
