//! Hand-written ABI of `IServerGameDLL` that the generated bindings do not
//! describe.

use crate::vtable_slot;
use std::ffi::CStr;

/// `void IServerGameDLL::GameFrame(bool simulating)`, which the engine calls
/// once per server frame to run the game's.
#[doc(alias("GameFrame"))]
pub type GameFrameFn = unsafe extern "C" fn(this: *mut sys::IServerGameDLL, simulating: bool);

const _: () = assert!(
	vtable_slot!(
		sys::IServerGameDLL__bindgen_vtable,
		IServerGameDLL_GameFrame
	) == GAME_FRAME_SLOT
);

// The generated binding has this signature.
const _: fn(&sys::IServerGameDLL__bindgen_vtable) -> GameFrameFn =
	|vtable| vtable.IServerGameDLL_GameFrame;

/// The vtable slot of `IServerGameDLL::GameFrame`.
///
/// The interface declares no virtual destructor, so the slot is the same under
/// the MSVC and Itanium ABIs.
pub const GAME_FRAME_SLOT: usize = 5;

/// The version string `IServerGameDLL` is exported and requested under.
///
/// This is `INTERFACEVERSION_SERVERGAMEDLL` from `public/eiface.h`.
#[doc(alias("INTERFACEVERSION_SERVERGAMEDLL"))]
pub const VERSION: &CStr = c"ServerGameDLL012";
