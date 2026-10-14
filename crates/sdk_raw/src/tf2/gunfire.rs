//! Hand-written ABI of how TF2 tells its bots of gunfire, which the generated
//! bindings omit: `NextBotManager::OnWeaponFired`, which
//! `game/server/NextBot/NextBotManager.h` declares, on the game's one bot
//! manager, TF2's `CTFBotManager` (`TheNextBots()`).
//!
//! The game calls it as a player fires a weapon, from
//! `CTFGameStats::Event_PlayerFiredWeapon`, which TF2's guns, projectile
//! weapons, melee swings and flame throwers call as they attack, with the
//! player and the weapon; as a spy's invisibility watch cloaks or uncloaks them,
//! with the watch; and as a bot bumps into an enemy, with the enemy and their
//! active weapon. TF2's bots notice the shooter of what they hear this way.

use crate::abi::CppDestructors;
use std::ffi::c_void;

/// The signature of `NextBotManager::OnWeaponFired`,
/// `void (CBaseCombatCharacter *whoFired, CBaseCombatWeapon *weapon)`, with
/// the manager as its receiver.
#[doc(alias("OnWeaponFired"))]
pub type OnWeaponFiredFn = unsafe extern "C" fn(
	this: *mut c_void,
	who_fired: *mut sys::CBaseCombatCharacter,
	weapon: *mut sys::CBaseCombatWeapon,
);

/// The undecorated name of TF2's bot manager class, by which its run-time
/// type information is found. The game keeps one instance of it,
/// `sTFBotManager`, which `TheNextBots()` returns.
#[doc(alias("sTFBotManager", "TheNextBots"))]
pub const BOT_MANAGER_CLASS: &str = "CTFBotManager";

/// The slot of `NextBotManager::OnWeaponFired` in the bot manager's primary
/// vtable. `NextBotManager` declares its virtual destructor, then `Update`,
/// `OnMapLoaded`, `OnRoundRestart`, `OnBeginChangeLevel`, `OnKilled`, `OnSound`
/// and `OnSpokeConcept`, then `OnWeaponFired`, and has no base class.
#[doc(alias("OnWeaponFired"))]
pub const ON_WEAPON_FIRED_SLOT: usize = CppDestructors::VTABLE_SLOTS + 7;
