//! Hand-written ABI of `IVEngineServer` that the generated bindings do not
//! describe.
//!
//! `IVEngineServer` declares no virtual destructor, so its methods occupy the
//! same slots under both the MSVC and Itanium ABIs. The slots of `ChangeLevel`
//! (0), `GetPlayerUserId` (15), `PEntityOfEntIndex` (19), `ServerCommand` (36)
//! and `LockNetworkStringTables` (53), counted from `public/eiface.h`, are
//! checked against the generated vtable, so a regenerated binding cannot
//! silently dispatch to another method.

use crate::vtable_slot;
use std::ffi::CStr;

const _: () = {
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_ChangeLevel
		) == 0
	);
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_GetPlayerUserId
		) == 15
	);
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_PEntityOfEntIndex
		) == 19
	);
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_ServerCommand
		) == 36
	);
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_LockNetworkStringTables
		) == 53
	);
};

/// The version string `IVEngineServer` is exported and requested under.
///
/// This is `INTERFACEVERSION_VENGINESERVER` from `public/eiface.h`.
#[doc(alias = "INTERFACEVERSION_VENGINESERVER")]
pub const VERSION: &CStr = c"VEngineServer023";
