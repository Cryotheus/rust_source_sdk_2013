//! Raw `ISmmAPI` vtables and loader version structures.
//!
//! Pointees whose layout is not part of this crate's contract (for example
//! SDK `CGlobalVars`, `ConCommandBase`, and `edict_t` objects) remain opaque as
//! `c_void` pointers.

use std::ffi::{c_char, c_int, c_void};
use std::mem::{align_of, offset_of, size_of};

pub const SOURCE_ENGINE_TF2: c_int = 17;

pub type CreateInterfaceFn =
	Option<unsafe extern "C" fn(name: *const c_char, return_code: *mut c_int) -> *mut c_void>;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct MetamodVersionInfoPrefix {
	pub api_major: c_int,
	pub api_minor: c_int,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct MetamodVersionInfo1226 {
	pub prefix: MetamodVersionInfoPrefix,
	pub sh_iface: c_int,
	pub sh_impl: c_int,
	pub pl_min: c_int,
	pub pl_max: c_int,
	pub source_engine: c_int,
	pub game_dir: *const c_char,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct MetamodVersionInfo1469 {
	pub prefix: MetamodVersionInfoPrefix,
	pub pl_min: c_int,
	pub pl_max: c_int,
	pub source_engine: c_int,
	pub game_dir: *const c_char,
}

/// A C++ `ISmmAPI` object, owned by Metamod.
///
/// `ISmmAPI` has no virtual destructor in either supported header. Its first
/// vtable entry is therefore `LogMsg`, not a destructor thunk.
#[repr(C)]
pub struct ISmmApi {
	pub vtable: *const ISmmApiVtablePrefix,
}

/// Slots 0–10, common to both supported builds.
#[repr(C)]
pub struct ISmmApiVtablePrefix {
	pub log_message: unsafe extern "C" fn(*mut ISmmApi, *mut c_void, *const c_char, ...),
	pub get_engine_factory: unsafe extern "C" fn(*mut ISmmApi, bool) -> CreateInterfaceFn,
	pub get_physics_factory: unsafe extern "C" fn(*mut ISmmApi, bool) -> CreateInterfaceFn,
	pub get_file_system_factory: unsafe extern "C" fn(*mut ISmmApi, bool) -> CreateInterfaceFn,
	pub get_server_factory: unsafe extern "C" fn(*mut ISmmApi, bool) -> CreateInterfaceFn,
	pub get_c_globals: unsafe extern "C" fn(*mut ISmmApi) -> *mut c_void,
	pub register_con_command_base:
		unsafe extern "C" fn(*mut ISmmApi, *mut c_void, *mut c_void) -> bool,
	pub unregister_con_command_base: unsafe extern "C" fn(*mut ISmmApi, *mut c_void, *mut c_void),
	pub con_print: unsafe extern "C" fn(*mut ISmmApi, *const c_char),
	pub con_printf: unsafe extern "C" fn(*mut ISmmApi, *const c_char, ...),
	pub get_api_versions:
		unsafe extern "C" fn(*mut ISmmApi, *mut c_int, *mut c_int, *mut c_int, *mut c_int),
}

/// Slots after `GetShVersions` in 1226, or `GetApiVersions` in 1469.
#[repr(C)]
pub struct ISmmApiVtableSuffix {
	pub add_listener: unsafe extern "C" fn(*mut ISmmApi, *mut c_void, *mut c_void),
	pub meta_factory:
		unsafe extern "C" fn(*mut ISmmApi, *const c_char, *mut c_int, *mut c_int) -> *mut c_void,
	pub format_iface: unsafe extern "C" fn(*mut ISmmApi, *mut c_char, usize) -> c_int,
	pub interface_search: unsafe extern "C" fn(
		*mut ISmmApi,
		CreateInterfaceFn,
		*const c_char,
		c_int,
		*mut c_int,
	) -> *mut c_void,
	pub get_base_dir: unsafe extern "C" fn(*mut ISmmApi) -> *const c_char,
	pub path_format:
		unsafe extern "C" fn(*mut ISmmApi, *mut c_char, usize, *const c_char, ...) -> usize,
	pub client_con_printf: unsafe extern "C" fn(*mut ISmmApi, *mut c_void, *const c_char, ...),
	pub interface_match:
		unsafe extern "C" fn(*mut ISmmApi, CreateInterfaceFn, *const c_char, c_int) -> *mut c_void,
	pub enable_vsp_listener: unsafe extern "C" fn(*mut ISmmApi),
	pub get_game_dll_version: unsafe extern "C" fn(*mut ISmmApi) -> c_int,
	pub get_user_message_count: unsafe extern "C" fn(*mut ISmmApi) -> c_int,
	pub find_user_message: unsafe extern "C" fn(*mut ISmmApi, *const c_char, *mut c_int) -> c_int,
	pub get_user_message: unsafe extern "C" fn(*mut ISmmApi, c_int, *mut c_int) -> *const c_char,
	pub get_vsp_version: unsafe extern "C" fn(*mut ISmmApi) -> c_int,
	pub get_source_engine_build: unsafe extern "C" fn(*mut ISmmApi) -> c_int,
	pub get_vsp_info: unsafe extern "C" fn(*mut ISmmApi, *mut c_int) -> *mut c_void,
	pub format: unsafe extern "C" fn(*mut ISmmApi, *mut c_char, usize, *const c_char, ...) -> usize,
	// the last parameter's type, `va_list`, is pointer-shaped on both supported 64-bit targets.
	pub format_args:
		unsafe extern "C" fn(*mut ISmmApi, *mut c_char, usize, *const c_char, *mut c_void) -> usize,
}

/// You should be using [`ISmmApi`].
#[repr(C)]
pub struct ISmmApiVtable1226 {
	pub prefix: ISmmApiVtablePrefix,
	pub get_sh_versions: unsafe extern "C" fn(*mut ISmmApi, *mut c_int, *mut c_int),
	pub suffix: ISmmApiVtableSuffix,
}

/// You should be using [`ISmmApi`].
#[repr(C)]
pub struct ISmmApiVtable1469 {
	pub prefix: ISmmApiVtablePrefix,
	pub suffix: ISmmApiVtableSuffix,
	pub register_con_command: unsafe extern "C" fn(*mut ISmmApi, *mut c_void, *mut c_void) -> bool,
	pub register_con_var: unsafe extern "C" fn(*mut ISmmApi, *mut c_void, *mut c_void) -> bool,
	pub unregister_con_command: unsafe extern "C" fn(*mut ISmmApi, *mut c_void, *mut c_void),
	pub unregister_con_var: unsafe extern "C" fn(*mut ISmmApi, *mut c_void, *mut c_void),
	pub get_detour_interface: unsafe extern "C" fn(*mut ISmmApi, c_int) -> *mut c_void,
}

const _: () = {
	const SLOT: usize = size_of::<*const ()>();
	const SLOT_ALIGN: usize = align_of::<*const ()>();

	macro_rules! assert_offset {
		($ty:ty, $field:ident, $offset:expr) => {
			assert!(offset_of!($ty, $field) == $offset);
		};
	}

	macro_rules! assert_slot {
		($ty:ty, $field:ident, $slot:expr) => {
			assert_offset!($ty, $field, $slot * SLOT);
		};
	}

	assert!(size_of::<c_int>() == 4);
	assert!(SLOT == 8);
	assert!(size_of::<CreateInterfaceFn>() == SLOT);
	assert!(align_of::<CreateInterfaceFn>() == SLOT_ALIGN);

	assert_offset!(MetamodVersionInfoPrefix, api_major, 0);
	assert_offset!(MetamodVersionInfoPrefix, api_minor, size_of::<c_int>());
	assert!(size_of::<MetamodVersionInfoPrefix>() == 2 * size_of::<c_int>());
	assert!(align_of::<MetamodVersionInfoPrefix>() == align_of::<c_int>());

	assert_offset!(MetamodVersionInfo1226, prefix, 0);
	assert_offset!(MetamodVersionInfo1226, sh_iface, 2 * size_of::<c_int>());
	assert_offset!(MetamodVersionInfo1226, sh_impl, 3 * size_of::<c_int>());
	assert_offset!(MetamodVersionInfo1226, pl_min, 4 * size_of::<c_int>());
	assert_offset!(MetamodVersionInfo1226, pl_max, 5 * size_of::<c_int>());
	assert_offset!(
		MetamodVersionInfo1226,
		source_engine,
		6 * size_of::<c_int>()
	);
	assert_offset!(MetamodVersionInfo1226, game_dir, 4 * SLOT);
	assert!(size_of::<MetamodVersionInfo1226>() == 5 * SLOT);
	assert!(align_of::<MetamodVersionInfo1226>() == SLOT_ALIGN);

	assert_offset!(MetamodVersionInfo1469, prefix, 0);
	assert_offset!(MetamodVersionInfo1469, pl_min, 2 * size_of::<c_int>());
	assert_offset!(MetamodVersionInfo1469, pl_max, 3 * size_of::<c_int>());
	assert_offset!(
		MetamodVersionInfo1469,
		source_engine,
		4 * size_of::<c_int>()
	);
	assert_offset!(MetamodVersionInfo1469, game_dir, 3 * SLOT);
	assert!(size_of::<MetamodVersionInfo1469>() == 4 * SLOT);
	assert!(align_of::<MetamodVersionInfo1469>() == SLOT_ALIGN);

	assert_slot!(ISmmApi, vtable, 0);
	assert!(size_of::<ISmmApi>() == SLOT);
	assert!(align_of::<ISmmApi>() == SLOT_ALIGN);

	assert_slot!(ISmmApiVtablePrefix, log_message, 0);
	assert_slot!(ISmmApiVtablePrefix, get_engine_factory, 1);
	assert_slot!(ISmmApiVtablePrefix, get_physics_factory, 2);
	assert_slot!(ISmmApiVtablePrefix, get_file_system_factory, 3);
	assert_slot!(ISmmApiVtablePrefix, get_server_factory, 4);
	assert_slot!(ISmmApiVtablePrefix, get_c_globals, 5);
	assert_slot!(ISmmApiVtablePrefix, register_con_command_base, 6);
	assert_slot!(ISmmApiVtablePrefix, unregister_con_command_base, 7);
	assert_slot!(ISmmApiVtablePrefix, con_print, 8);
	assert_slot!(ISmmApiVtablePrefix, con_printf, 9);
	assert_slot!(ISmmApiVtablePrefix, get_api_versions, 10);
	assert!(size_of::<ISmmApiVtablePrefix>() == 11 * SLOT);
	assert!(align_of::<ISmmApiVtablePrefix>() == SLOT_ALIGN);

	assert_slot!(ISmmApiVtableSuffix, add_listener, 0);
	assert_slot!(ISmmApiVtableSuffix, meta_factory, 1);
	assert_slot!(ISmmApiVtableSuffix, format_iface, 2);
	assert_slot!(ISmmApiVtableSuffix, interface_search, 3);
	assert_slot!(ISmmApiVtableSuffix, get_base_dir, 4);
	assert_slot!(ISmmApiVtableSuffix, path_format, 5);
	assert_slot!(ISmmApiVtableSuffix, client_con_printf, 6);
	assert_slot!(ISmmApiVtableSuffix, interface_match, 7);
	assert_slot!(ISmmApiVtableSuffix, enable_vsp_listener, 8);
	assert_slot!(ISmmApiVtableSuffix, get_game_dll_version, 9);
	assert_slot!(ISmmApiVtableSuffix, get_user_message_count, 10);
	assert_slot!(ISmmApiVtableSuffix, find_user_message, 11);
	assert_slot!(ISmmApiVtableSuffix, get_user_message, 12);
	assert_slot!(ISmmApiVtableSuffix, get_vsp_version, 13);
	assert_slot!(ISmmApiVtableSuffix, get_source_engine_build, 14);
	assert_slot!(ISmmApiVtableSuffix, get_vsp_info, 15);
	assert_slot!(ISmmApiVtableSuffix, format, 16);
	assert_slot!(ISmmApiVtableSuffix, format_args, 17);
	assert!(size_of::<ISmmApiVtableSuffix>() == 18 * SLOT);
	assert!(align_of::<ISmmApiVtableSuffix>() == SLOT_ALIGN);

	assert_slot!(ISmmApiVtable1226, prefix, 0);
	assert_slot!(ISmmApiVtable1226, get_sh_versions, 11);
	assert_slot!(ISmmApiVtable1226, suffix, 12);
	assert!(size_of::<ISmmApiVtable1226>() == 30 * SLOT);
	assert!(align_of::<ISmmApiVtable1226>() == SLOT_ALIGN);

	assert_slot!(ISmmApiVtable1469, prefix, 0);
	assert_slot!(ISmmApiVtable1469, suffix, 11);
	assert_slot!(ISmmApiVtable1469, register_con_command, 29);
	assert_slot!(ISmmApiVtable1469, register_con_var, 30);
	assert_slot!(ISmmApiVtable1469, unregister_con_command, 31);
	assert_slot!(ISmmApiVtable1469, unregister_con_var, 32);
	assert_slot!(ISmmApiVtable1469, get_detour_interface, 33);
	assert!(size_of::<ISmmApiVtable1469>() == 34 * SLOT);
	assert!(align_of::<ISmmApiVtable1469>() == SLOT_ALIGN);
};
