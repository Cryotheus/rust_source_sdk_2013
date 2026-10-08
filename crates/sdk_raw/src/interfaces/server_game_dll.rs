//! Hand-written ABI of `IServerGameDLL` that the generated bindings do not
//! describe.

use crate::vtable_slot;
use std::ffi::CStr;

/// `void IServerGameDLL::GameFrame(bool simulating)`, which the engine calls
/// once per server frame to run the game's.
#[doc(alias("GameFrame"))]
pub type GameFrameFn = unsafe extern "C" fn(this: *mut sys::IServerGameDLL, simulating: bool);

/// `void IServerGameDLL::SetServerHibernation(bool hibernating)`, which the
/// engine calls as the server starts and stops hibernating.
#[doc(alias("SetServerHibernation"))]
pub type SetServerHibernationFn =
	unsafe extern "C" fn(this: *mut sys::IServerGameDLL, hibernating: bool);

/// `void IServerGameDLL::Think(bool finalTick)`, which the engine calls once
/// per frame, even when no level is loaded.
#[doc(alias("Think"))]
pub type ThinkFn = unsafe extern "C" fn(this: *mut sys::IServerGameDLL, final_tick: bool);

const _: () = assert!(
	vtable_slot!(
		sys::IServerGameDLL__bindgen_vtable,
		IServerGameDLL_GameFrame
	) == GAME_FRAME_SLOT
);

const _: () = assert!(
	vtable_slot!(
		sys::IServerGameDLL__bindgen_vtable,
		IServerGameDLL_SetServerHibernation
	) == SET_SERVER_HIBERNATION_SLOT
);

const _: () =
	assert!(vtable_slot!(sys::IServerGameDLL__bindgen_vtable, IServerGameDLL_Think) == THINK_SLOT);

// The generated binding has this signature.
const _: fn(&sys::IServerGameDLL__bindgen_vtable) -> GameFrameFn =
	|vtable| vtable.IServerGameDLL_GameFrame;

// The generated binding has this signature.
const _: fn(&sys::IServerGameDLL__bindgen_vtable) -> SetServerHibernationFn =
	|vtable| vtable.IServerGameDLL_SetServerHibernation;

// The generated binding has this signature.
const _: fn(&sys::IServerGameDLL__bindgen_vtable) -> ThinkFn = |vtable| vtable.IServerGameDLL_Think;

/// The vtable slot of `IServerGameDLL::GameFrame`.
///
/// The interface declares no virtual destructor, so the slot is the same under
/// the MSVC and Itanium ABIs.
pub const GAME_FRAME_SLOT: usize = 5;

/// The vtable slot of `IServerGameDLL::SetServerHibernation`, the same under
/// both ABIs, as for [`GAME_FRAME_SLOT`].
pub const SET_SERVER_HIBERNATION_SLOT: usize = 38;

/// The vtable slot of `IServerGameDLL::Think`, the same under both ABIs, as
/// for [`GAME_FRAME_SLOT`].
pub const THINK_SLOT: usize = 31;

/// The version string `IServerGameDLL` is exported and requested under.
///
/// This is `INTERFACEVERSION_SERVERGAMEDLL` from `public/eiface.h`.
#[doc(alias("INTERFACEVERSION_SERVERGAMEDLL"))]
pub const VERSION: &CStr = c"ServerGameDLL012";
