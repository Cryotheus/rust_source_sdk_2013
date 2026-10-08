//! Hand-written ABI of TF2's players that the generated bindings omit: the
//! player states of `game/shared/tf/tf_shareddefs.h`, the HUD elements of
//! `game/shared/shareddefs.h` that a player's `m_iHideHUD` hides, the names
//! of TF2's class and team menus, and the vtable slots of the player's view
//! and weapon methods.

use crate::abi::CppDestructors;
use crate::vtable_slot;
use std::ffi::{CStr, c_int};

// SourceMod's `gamedata/sdktools.games/game.tf.txt` lists `EyeAngles` at 138
// and `CommitSuicide` at 454 on Windows, and at 139 and 454 on Linux, and its
// `sdkhooks.games/engine.ep2v.txt` lists `Weapon_Switch` at 275 on Windows and
// 276 on Linux, as the generated vtable has them. `EyePosition` directly
// precedes `EyeAngles` in `CBasePlayer`'s declaration. The generated methods
// have the signatures below, with `CTFPlayer` receivers.
const _: () = {
	assert!(EYE_ANGLES_SLOT == 137 + CppDestructors::VTABLE_SLOTS);
	assert!(EYE_POSITION_SLOT + 1 == EYE_ANGLES_SLOT);
	assert!(COMMIT_SUICIDE_SLOT == 454);
	assert!(WEAPON_SWITCH_SLOT == 274 + CppDestructors::VTABLE_SLOTS);

	let _: fn(
		&sys::CTFPlayer__bindgen_vtable,
	) -> unsafe extern "C" fn(*mut sys::CTFPlayer) -> *const sys::QAngle =
		|vtable| vtable.CTFPlayer_EyeAngles;
	let _: fn(
		&sys::CTFPlayer__bindgen_vtable,
	) -> unsafe extern "C" fn(*mut sys::CTFPlayer, bool, bool) =
		|vtable| vtable.CTFPlayer_CommitSuicide;
	let _: fn(
		&sys::CTFPlayer__bindgen_vtable,
	) -> unsafe extern "C" fn(
		*mut sys::CTFPlayer,
		*mut sys::CBaseCombatWeapon,
		c_int,
	) -> bool = |vtable| vtable.CTFPlayer_Weapon_Switch;
};

/// The slot of `CTFPlayer::CommitSuicide(bool, bool)` in a TF2 player's
/// primary vtable, from the generated binding.
#[doc(alias("CommitSuicide"))]
pub const COMMIT_SUICIDE_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_CommitSuicide);

/// The slot of `CBasePlayer::EyeAngles` in a TF2 player's primary vtable, from
/// the generated binding. It returns a reference to the player's own angles.
#[doc(alias("EyeAngles"))]
pub const EYE_ANGLES_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_EyeAngles);

/// The slot of `CBasePlayer::EyePosition` in a TF2 player's primary vtable,
/// from the generated binding. It returns a `Vector` by value: through a
/// hidden result pointer under the MSVC ABI, and in registers under the
/// Itanium ABI.
#[doc(alias("EyePosition"))]
pub const EYE_POSITION_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_EyePosition);

/// `HIDEHUD_ALL`: hides the whole HUD.
pub const HIDEHUD_ALL: c_int = 1 << 2;

/// `HIDEHUD_BITCOUNT` under `TF_DLL`: how many of the low bits of
/// `m_iHideHUD` TF2 uses, the `HIDEHUD_*` constants' bits.
pub const HIDEHUD_BITCOUNT: u32 = 18;

/// `HIDEHUD_BONUS_PROGRESS`: hides the bonus progress display of other
/// Source games.
pub const HIDEHUD_BONUS_PROGRESS: c_int = 1 << 11;

/// `HIDEHUD_BUILDING_STATUS`: hides the Engineer's building status.
pub const HIDEHUD_BUILDING_STATUS: c_int = 1 << 12;

/// `HIDEHUD_CHAT`: hides chat and the other communication elements, such as
/// the voice icons.
pub const HIDEHUD_CHAT: c_int = 1 << 7;

/// `HIDEHUD_CLOAK_AND_FEIGN`: hides the item effect meters, such as the Spy's
/// cloak.
pub const HIDEHUD_CLOAK_AND_FEIGN: c_int = 1 << 13;

/// `HIDEHUD_CROSSHAIR`: hides the crosshair.
pub const HIDEHUD_CROSSHAIR: c_int = 1 << 8;

/// `HIDEHUD_FLASHLIGHT`: hides the flashlight meter of other Source games.
pub const HIDEHUD_FLASHLIGHT: c_int = 1 << 1;

/// `HIDEHUD_HEALTH`: hides the health display.
pub const HIDEHUD_HEALTH: c_int = 1 << 3;

/// `HIDEHUD_INVEHICLE`: hides what a player in a vehicle should not see.
pub const HIDEHUD_INVEHICLE: c_int = 1 << 10;

/// `HIDEHUD_MATCH_STATUS`: hides the match status, such as the round timer
/// and the team status at the top of the screen.
pub const HIDEHUD_MATCH_STATUS: c_int = 1 << 17;

/// `HIDEHUD_METAL`: hides the Engineer's metal display.
pub const HIDEHUD_METAL: c_int = 1 << 15;

/// `HIDEHUD_MISCSTATUS`: hides miscellaneous status elements, such as the
/// pickup history and the kill feed.
pub const HIDEHUD_MISCSTATUS: c_int = 1 << 6;

/// `HIDEHUD_NEEDSUIT`: hides what a player without Half-Life 2's HEV suit
/// should not see.
pub const HIDEHUD_NEEDSUIT: c_int = 1 << 5;

/// `HIDEHUD_PIPES_AND_CHARGE`: hides the Demoman's sticky bomb count and
/// shield charge meter.
pub const HIDEHUD_PIPES_AND_CHARGE: c_int = 1 << 14;

/// `HIDEHUD_PLAYERDEAD`: hides what a dead player should not see.
pub const HIDEHUD_PLAYERDEAD: c_int = 1 << 4;

/// `HIDEHUD_TARGET_ID`: hides the target ID, the name and health of the
/// player under the crosshair.
pub const HIDEHUD_TARGET_ID: c_int = 1 << 16;

/// `HIDEHUD_VEHICLE_CROSSHAIR`: hides the vehicle crosshair of other Source
/// games.
pub const HIDEHUD_VEHICLE_CROSSHAIR: c_int = 1 << 9;

/// `HIDEHUD_WEAPONSELECTION`: hides the ammo count and the weapon selection.
pub const HIDEHUD_WEAPONSELECTION: c_int = 1 << 0;

/// `PANEL_CLASS_BLUE`: BLU's class menu.
pub const PANEL_CLASS_BLUE: &CStr = c"class_blue";

/// `PANEL_CLASS_RED`: RED's class menu.
pub const PANEL_CLASS_RED: &CStr = c"class_red";

/// `PANEL_TEAM`: the team menu, from `game/shared/viewport_panel_names.h`.
pub const PANEL_TEAM: &CStr = c"team";

/// `TF_STATE_ACTIVE`: playing, alive or not.
pub const TF_STATE_ACTIVE: c_int = 0;

/// `TF_STATE_COUNT`: the number of player states.
pub const TF_STATE_COUNT: c_int = 4;

/// `TF_STATE_DYING`: dying, from death until the player starts to observe.
pub const TF_STATE_DYING: c_int = 3;

/// `TF_STATE_OBSERVER`: observing, as spectators and unassigned players do.
pub const TF_STATE_OBSERVER: c_int = 2;

/// `TF_STATE_WELCOME`: just joined the server, before choosing a team.
pub const TF_STATE_WELCOME: c_int = 1;

/// The slot of `CTFPlayer::Weapon_Switch(CBaseCombatWeapon *, int)` in a TF2
/// player's primary vtable, from the generated binding.
#[doc(alias("Weapon_Switch"))]
pub const WEAPON_SWITCH_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_Weapon_Switch);
