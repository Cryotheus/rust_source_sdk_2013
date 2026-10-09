//! Hand-written ABI of the engine's and game's interfaces, and of the factories
//! that export them.

pub mod bot_manager;
pub mod cvar;
pub mod engine_replay;
pub mod engine_sound;
pub mod engine_trace;
pub mod game_event;
pub mod model_info;
pub mod network_string_tables;
pub mod player_info_manager;
pub mod plugin_helpers;
pub mod server_game_clients;
pub mod server_game_dll;
pub mod server_game_ents;
pub mod server_game_tags;
pub mod server_tools;
pub mod temp_entities;
pub mod valve_engine;
pub mod voice_server;

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr::NonNull;

/// The signature of `CreateInterface`, which every Source module exports to
/// hand out the interfaces it implements, by name.
///
/// This is `CreateInterfaceFn` from `public/tier1/interface.h`, whose
/// generated binding, `sys::CreateInterfaceFn`, is nullable.
#[doc(alias("CreateInterface"))]
pub type CreateInterfaceFn =
	unsafe extern "C" fn(name: *const c_char, return_code: *mut c_int) -> *mut c_void;

// The generated binding is this signature, made nullable.
const _: fn(CreateInterfaceFn) -> sys::CreateInterfaceFn = Some;

/// Asks `factory` for the interface exported under `name`, such as
/// `VEngineServer023`, or returns `None` if the factory's module does not
/// export it.
///
/// A factory reports a missing interface by returning null, and also sets its
/// return code to `IFACE_FAILED` rather than `IFACE_OK`; only the null is
/// checked, so a factory that leaves the code unset is still understood.
///
/// The pointer is the factory's object, of the class exported under `name`.
/// It does not establish that class's layout or the object's lifetime.
///
/// # Safety
///
/// `factory` must be the `CreateInterface` export of a module that stays
/// loaded for the call, and must be called on a thread where that module
/// allows it, such as the server's main thread for the engine's and game's
/// modules.
pub unsafe fn create_interface(factory: CreateInterfaceFn, name: &CStr) -> Option<NonNull<c_void>> {
	let mut return_code = 0;

	// SAFETY: As the caller promises; the name is terminated and the return
	// code is a local.
	NonNull::new(unsafe { factory(name.as_ptr(), &raw mut return_code) })
}
