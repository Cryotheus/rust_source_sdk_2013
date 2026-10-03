//! Hand-written ABI of `IVEngineServer` that the generated bindings do not
//! describe.
//!
//! `IVEngineServer` declares no virtual destructor, so its methods occupy the
//! same slots under both the MSVC and Itanium ABIs. The slots of the methods
//! below, counted from `public/eiface.h`, are checked against the generated
//! vtable, so a regenerated binding cannot silently dispatch to another method.

use crate::vtable_slot;

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
