//! Hand-written ABI of TF2's player respawns: the vtable slot of
//! `CTFPlayer::ForceRespawn`, through which every respawn of a dead player
//! passes, and its signature.

use crate::abi::CppDestructors;
use crate::vtable_slot;

/// The signature of `CTFPlayer::ForceRespawn`, `void ()`, with the player as
/// its receiver.
///
/// The generated method takes a `CTFPlayer` receiver. It is the player's
/// primary base, `CBaseEntity`, at the same address, so the method can be
/// called and hooked with an entity receiver.
#[doc(alias("ForceRespawn"))]
pub type ForceRespawnFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity);

// `ForceRespawn` is slot 337 of TF2's 64-bit Windows `server.dll`, whose
// `CTFPlayer` vtable, found through its run-time type information, holds there
// the function referencing the `"CTFPlayer::ForceRespawn"` profiling string.
// That string has no other reference in the module, so no inlined copy of the
// function exists, and no call reaches it directly: every caller, the
// `game_forcerespawn` inputs, respawn waves, round restarts and the script
// bindings included, calls through the vtable. `CTFBot` and
// `NextBotPlayer<CTFPlayer>` keep the same function at the slot. SourceMod's
// `gamedata/sm-tf2.games.txt` lists 337 for Windows and 338 for Linux, whose
// Itanium vtables start with two destructor slots instead of MSVC's one.
const _: () = {
	assert!(FORCE_RESPAWN_SLOT == 336 + CppDestructors::VTABLE_SLOTS);
	assert!(
		FORCE_RESPAWN_SLOT == vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_ForceRespawn)
	);
};

// The generated method takes no argument and returns nothing.
const _: fn(&sys::CTFPlayer__bindgen_vtable) -> unsafe extern "C" fn(*mut sys::CTFPlayer) =
	|vtable| vtable.CTFPlayer_ForceRespawn;

/// The slot of `CTFPlayer::ForceRespawn` in a TF2 player's primary vtable,
/// from the generated binding.
#[doc(alias("ForceRespawn"))]
pub const FORCE_RESPAWN_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_ForceRespawn);
