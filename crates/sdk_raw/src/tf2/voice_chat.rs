//! Hand-written ABI of TF2's voice chat rules, which the generated bindings
//! omit: the game's `IVoiceGameMgrHelper`, which `game/shared/voice_gamemgr.h`
//! declares, and TF2's `CVoiceGameMgrHelper` of `game/shared/tf/tf_gamerules.cpp`
//! implements, deciding which players hear each other's voice chat.

use crate::abi::CppDestructors;
use std::ffi::c_void;

/// The signature of `IVoiceGameMgrHelper::CanPlayerHearPlayer`,
/// `bool (CBasePlayer *listener, CBasePlayer *talker, bool &proximity)`,
/// with the helper as its receiver.
#[doc(alias("CanPlayerHearPlayer"))]
pub type CanPlayerHearPlayerFn = unsafe extern "C" fn(
	this: *mut c_void,
	listener: *mut sys::CBasePlayer,
	talker: *mut sys::CBasePlayer,
	proximity: *mut bool,
) -> bool;

/// The slot of `IVoiceGameMgrHelper::CanPlayerHearPlayer` in the helper's
/// primary vtable, which the interface declares after its virtual destructor
/// alone.
#[doc(alias("CanPlayerHearPlayer"))]
pub const CAN_PLAYER_HEAR_PLAYER_SLOT: usize = CppDestructors::VTABLE_SLOTS;

/// How often the game asks the helper who hears whom, in seconds: it updates
/// every player's voice chat masks at most once this often.
///
/// This is `UPDATE_INTERVAL` from `game/shared/voice_gamemgr.cpp`.
pub const VOICE_MASK_UPDATE_INTERVAL: f32 = 0.3;

/// The undecorated name of TF2's voice chat helper class, by which its
/// run-time type information is found. The game keeps one instance of it,
/// `g_VoiceGameMgrHelper`.
#[doc(alias("g_VoiceGameMgrHelper"))]
pub const VOICE_GAME_MGR_HELPER_CLASS: &str = "CVoiceGameMgrHelper";
