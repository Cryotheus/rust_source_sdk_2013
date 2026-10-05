//! Team Fortress 2's game-specific raw bindings: functions, layouts, header
//! values, and vtable slots of its game DLL and engine that the generated
//! bindings do not describe.
//!
//! A `CBaseEntity *` to a TF2 player is also a `CTFPlayer *`, the type the
//! generated vtable of `CTFPlayer` takes: every class from `CTFPlayer` down to
//! `CBaseEntity` starts with its primary base, as this module asserts against
//! the generated layouts. The Itanium bindings describe `CBasePlayer` and
//! `CBaseAnimating` as opaque blobs, whose single, polymorphic primary bases
//! that ABI also places first.

pub mod attributes;
pub mod class;
pub mod conditions;
pub mod damage;
pub mod game_events;
pub mod gc;
pub mod host_timescale;
pub mod item_generation;
pub mod ragdolls;
pub mod respawn;
pub mod scoreboard;
pub mod script_binding;
pub mod voice;
pub mod voting;
pub mod weapons;
pub mod wearables;

use std::mem::offset_of;

const _: () = {
	assert!(
		offset_of!(sys::CTFPlayer, _base) == 0
			&& offset_of!(sys::CBaseMultiplayerPlayer, _base) == 0
			&& offset_of!(sys::CAI_ExpresserHost<sys::CBasePlayer>, _base) == 0
			&& offset_of!(sys::CBaseCombatCharacter, _base) == 0
			&& offset_of!(sys::CBaseFlex, _base) == 0
			&& offset_of!(sys::CBaseAnimatingOverlay, _base) == 0
	);

	#[cfg(target_os = "windows")]
	assert!(
		offset_of!(sys::CBasePlayer, _base) == 0 && offset_of!(sys::CBaseAnimating, _base) == 0
	);
};
