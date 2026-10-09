//! Hand-written ABI of `IEngineReplay`, which the generated bindings do not
//! describe: the engine's services for its replay system, from
//! `common/replay/ienginereplay.h`.
//!
//! `IEngineReplay` derives from `IBaseInterface`, whose virtual destructor
//! comes first in the vtable: one slot under MSVC and two under the Itanium
//! ABI. Its methods follow in the order the header declares them.

use crate::abi::CppDestructors;
use crate::vtable_slot;
use std::ffi::{CStr, c_void};

/// `void IEngineReplay::RecalculateTags()`, which has the game server
/// recalculate its tags (`sv_tags`) from the console variables that tag it.
#[doc(alias("RecalculateTags"))]
pub type RecalculateTagsFn = unsafe extern "C" fn(this: *mut IEngineReplay);

const _: () = assert!(vtable_slot!(IEngineReplayVtable, recalculate_tags) == RECALCULATE_TAGS_SLOT);

/// The vtable slot of `IEngineReplay::RecalculateTags`: its 24th method, after
/// the destructor slots.
#[doc(alias("RecalculateTags"))]
pub const RECALCULATE_TAGS_SLOT: usize = CppDestructors::VTABLE_SLOTS + 23;

/// The version string `IEngineReplay` is expected to be exported and is
/// requested under.
///
/// This is `ENGINE_REPLAY_INTERFACE_VERSION` from
/// `common/replay/ienginereplay.h`.
#[doc(alias("ENGINE_REPLAY_INTERFACE_VERSION"))]
pub const VERSION: &CStr = c"EngineReplay001";

/// `IEngineReplay`, the engine's services for its replay system, which the
/// engine module is expected to export: the header declares its version,
/// and TF2's client library requests it from the engine's factory. The
/// engine is not public, so this is inferred, and finding the interface can
/// fail.
#[doc(alias("CEngineReplay"))]
#[repr(C)]
pub struct IEngineReplay {
	/// The pointer to the object's vtable.
	pub vtable_: *const IEngineReplayVtable,
}

/// The vtable of `IEngineReplay`. Only the methods this crate calls have
/// their signatures; the others are opaque.
#[repr(C)]
pub struct IEngineReplayVtable {
	/// `IBaseInterface`'s virtual destructor.
	pub destructors: [*const c_void; CppDestructors::VTABLE_SLOTS],

	/// `IsSupportedModAndPlatform`.
	pub is_supported_mod_and_platform: *const c_void,

	/// `GetHostTime`.
	pub get_host_time: *const c_void,

	/// `GetHostTickCount`.
	pub get_host_tick_count: *const c_void,

	/// `TimeToTicks`.
	pub time_to_ticks: *const c_void,

	/// `TicksToTime`.
	pub ticks_to_time: *const c_void,

	/// `ReadDemoHeader`.
	pub read_demo_header: *const c_void,

	/// `GetGameDir`.
	pub get_game_dir: *const c_void,

	/// `Cbuf_AddText`.
	pub cbuf_add_text: *const c_void,

	/// `Cbuf_Execute`.
	pub cbuf_execute: *const c_void,

	/// `Host_Disconnect`.
	pub host_disconnect: *const c_void,

	/// `HostState_Shutdown`.
	pub host_state_shutdown: *const c_void,

	/// `GetModDir`.
	pub get_mod_dir: *const c_void,

	/// `CopyFile`.
	pub copy_file: *const c_void,

	/// `LZSS_Compress`.
	pub lzss_compress: *const c_void,

	/// `LZSS_Decompress`.
	pub lzss_decompress: *const c_void,

	/// `MD5_HashBuffer`.
	pub md5_hash_buffer: *const c_void,

	/// `GetReplayServer`.
	pub get_replay_server: *const c_void,

	/// `GetReplayServerAsIServer`.
	pub get_replay_server_as_iserver: *const c_void,

	/// `GetGameServer`.
	pub get_game_server: *const c_void,

	/// `GetSessionRecordBuffer`.
	pub get_session_record_buffer: *const c_void,

	/// `IsDedicated`.
	pub is_dedicated: *const c_void,

	/// `ResetReplayRecordBuffer`.
	pub reset_replay_record_buffer: *const c_void,

	/// `GetReplayDemoHeader`.
	pub get_replay_demo_header: *const c_void,

	/// `RecalculateTags`.
	pub recalculate_tags: RecalculateTagsFn,

	/// `NET_GetHostnameAsIP`.
	pub net_get_hostname_as_ip: *const c_void,
}
