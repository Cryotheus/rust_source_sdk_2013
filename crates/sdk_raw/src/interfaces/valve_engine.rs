//! Hand-written ABI of `IVEngineServer` that the generated bindings do not
//! describe.
//!
//! `IVEngineServer` declares no virtual destructor, so its methods occupy the
//! same slots under both the MSVC and Itanium ABIs. The slots of `ChangeLevel`
//! (0), `GetPlayerUserId` (15), `PEntityOfEntIndex` (19), `ServerCommand` (36),
//! `ServerExecute` (37), `LockNetworkStringTables` (53), `CreateFakeClient`
//! (54) and `CreateFakeClientEx` (116), counted from `public/eiface.h`, are
//! checked against the generated vtable, so a regenerated binding cannot
//! silently dispatch to another method.

use crate::vtable_slot;
use std::ffi::{CStr, c_char};

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
			IVEngineServer_ServerExecute
		) == 37
	);
	assert!(
		vtable_slot!(
			sys::IVEngineServer__bindgen_vtable,
			IVEngineServer_LockNetworkStringTables
		) == 53
	);
	assert!(CREATE_FAKE_CLIENT_SLOT == 54);
	assert!(CREATE_FAKE_CLIENT_EX_SLOT == 116);
};

// The generated methods take and return what the aliases below do.
const _: fn(&sys::IVEngineServer__bindgen_vtable) -> CreateFakeClientFn =
	|vtable| vtable.IVEngineServer_CreateFakeClient;
const _: fn(&sys::IVEngineServer__bindgen_vtable) -> CreateFakeClientExFn =
	|vtable| vtable.IVEngineServer_CreateFakeClientEx;

/// The slot of `IVEngineServer::CreateFakeClient`, from the generated binding.
#[doc(alias("CreateFakeClient"))]
pub const CREATE_FAKE_CLIENT_SLOT: usize = vtable_slot!(
	sys::IVEngineServer__bindgen_vtable,
	IVEngineServer_CreateFakeClient
);

/// The slot of `IVEngineServer::CreateFakeClientEx`, from the generated
/// binding.
#[doc(alias("CreateFakeClientEx"))]
pub const CREATE_FAKE_CLIENT_EX_SLOT: usize = vtable_slot!(
	sys::IVEngineServer__bindgen_vtable,
	IVEngineServer_CreateFakeClientEx
);

/// `IVEngineServer::CreateFakeClient`: connects a fake client named `name`,
/// returning its edict, or null if the server has no free slot.
#[doc(alias("CreateFakeClient"))]
pub type CreateFakeClientFn =
	unsafe extern "C" fn(this: *mut sys::IVEngineServer, name: *const c_char) -> *mut sys::edict_t;

/// `IVEngineServer::CreateFakeClientEx`: as [`CreateFakeClientFn`], with the
/// engine's `bReportFakeClient` as `report`, which the header defaults to true.
/// The game passes false only for Mann vs. Machine's robots, through
/// `NextBotCreatePlayerBot`.
///
/// The engine is not public. TF2's sets its choice for new fake clients,
/// calls `CreateFakeClient` through the vtable, and sets the choice back to
/// true. It tells Steam of an unreported client as it does of SourceTV: as
/// neither a bot nor a slot, and not as a player.
#[doc(alias("CreateFakeClientEx"))]
pub type CreateFakeClientExFn = unsafe extern "C" fn(
	this: *mut sys::IVEngineServer,
	name: *const c_char,
	report: bool,
) -> *mut sys::edict_t;

/// The version string `IVEngineServer` is exported and requested under.
///
/// This is `INTERFACEVERSION_VENGINESERVER` from `public/eiface.h`.
#[doc(alias("INTERFACEVERSION_VENGINESERVER"))]
pub const VERSION: &CStr = c"VEngineServer023";
