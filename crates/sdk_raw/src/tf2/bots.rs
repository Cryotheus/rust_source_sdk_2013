//! Hand-written values of TF2's bots, `CTFBot` in
//! `game/server/tf/bot/tf_bot.h`, which the generated bindings do not
//! describe, and the names of the script classes that declare the native
//! methods of bots and other NextBot actors.
//!
//! A `CTFBot` is a `CTFPlayer` with the same server class and data maps, so
//! only its script class, and [`TF_BOT_TYPE`] from `GetBotType`, tell it
//! apart.

use std::ffi::{CStr, c_int};

/// The script class declaring the native methods of `NextBotCombatCharacter`,
/// the base of TF2's NextBot actors that are not players, such as `base_boss`
/// and the Halloween bosses (`game/server/NextBot/NextBot.cpp:77-93`).
pub const NEXT_BOT_COMBAT_CHARACTER_CLASS: &CStr = c"NextBotCombatCharacter";

/// The script class declaring the native methods of `CTFBot`
/// (`game/server/tf/bot/tf_bot.cpp:618-712`), which also declares the
/// NextBot queries `NextBotCombatCharacter` does.
pub const TF_BOT_CLASS: &CStr = c"CTFBot";

/// What `CTFBot::GetBotType` returns, and what TF2 passes to `IsBotOfType` to
/// recognise its bots. Other players return 0.
pub const TF_BOT_TYPE: c_int = 1337;

/// The script class declaring `GetBotType` and `IsBotOfType`
/// (`game/server/tf/tf_player.cpp:685-686`).
pub const TF_PLAYER_CLASS: &CStr = c"CTFPlayer";

/// `CTFBot::AttributeType`: the bits of a bot's attribute flags, which
/// `AddBotAttribute` and its siblings take.
pub mod attribute {
	use std::ffi::c_int;

	/// In Mann vs. Machine, pushes for the capture point.
	pub const AGGRESSIVE: c_int = 1 << 1;

	/// A Demoman that charges only in the air.
	pub const AIR_CHARGE_ONLY: c_int = 1 << 19;

	/// Always fires critical hits.
	pub const ALWAYS_CRIT: c_int = 1 << 9;

	/// Fires its weapon constantly.
	pub const ALWAYS_FIRE_WEAPON: c_int = 1 << 13;

	/// Jumps on its own, at intervals `SetAutoJump` sets.
	pub const AUTO_JUMP: c_int = 1 << 18;

	/// Moves the bot to the spectators when it is killed.
	pub const BECOME_SPECTATOR_ON_DEATH: c_int = 1 << 5;

	/// Shielded against blasts.
	pub const BLAST_IMMUNE: c_int = 1 << 24;

	/// Shielded against bullets.
	pub const BULLET_IMMUNE: c_int = 1 << 23;

	/// Does not dodge.
	pub const DISABLE_DODGE: c_int = 1 << 4;

	/// Shielded against fire.
	pub const FIRE_IMMUNE: c_int = 1 << 25;

	/// Waits for a barrage weapon, such as a rocket launcher, to reload fully
	/// before firing it.
	pub const HOLD_FIRE_UNTIL_FULL_RELOAD: c_int = 1 << 11;

	/// Ignores its enemies.
	pub const IGNORE_ENEMIES: c_int = 1 << 10;

	/// Does not pick up the flag or the bomb.
	pub const IGNORE_FLAG: c_int = 1 << 17;

	/// A non-player support character.
	pub const IS_NPC: c_int = 1 << 2;

	/// A Mann vs. Machine mini-boss.
	pub const MINIBOSS: c_int = 1 << 15;

	/// A Demoman or Soldier that opens a parachute when falling.
	pub const PARACHUTE: c_int = 1 << 26;

	/// Prefers the Vaccinator's blast resistance.
	pub const PREFER_VACCINATOR_BLAST: c_int = 1 << 21;

	/// Prefers the Vaccinator's bullet resistance.
	pub const PREFER_VACCINATOR_BULLETS: c_int = 1 << 20;

	/// Prefers the Vaccinator's fire resistance.
	pub const PREFER_VACCINATOR_FIRE: c_int = 1 << 22;

	/// Defends when it can.
	pub const PRIORITIZE_DEFENSE: c_int = 1 << 12;

	/// A Medic that deploys a projectile shield.
	pub const PROJECTILE_SHIELD: c_int = 1 << 27;

	/// Counted by the bot quota, `tf_bot_quota`. The header misspells it
	/// `QUOTA_MANANGED`.
	pub const QUOTA_MANAGED: c_int = 1 << 6;

	/// Kicks the bot from the server when it is killed.
	pub const REMOVE_ON_DEATH: c_int = 1 << 0;

	/// Keeps its buildings when it disconnects.
	pub const RETAIN_BUILDINGS: c_int = 1 << 7;

	/// Spawns with every weapon fully charged, such as an ÜberCharge.
	pub const SPAWN_WITH_FULL_CHARGE: c_int = 1 << 8;

	/// Holds its fire.
	pub const SUPPRESS_FIRE: c_int = 1 << 3;

	/// Teleports to its hint target instead of walking out of its spawn.
	pub const TELEPORT_TO_HINT: c_int = 1 << 14;

	/// Shows its health in the boss health bar.
	pub const USE_BOSS_HEALTH_BAR: c_int = 1 << 16;
}

/// The `TFBOT_*` behaviour flags, which `SetBehaviorFlag` and its siblings
/// take. They match the spawn flags of the `bot_generator` entity.
pub mod behavior {
	use std::ffi::c_int;

	/// `TFBOT_IGNORE_ENEMY_DEMOMEN`.
	pub const IGNORE_ENEMY_DEMOMEN: c_int = 0x0008;

	/// `TFBOT_IGNORE_ENEMY_ENGINEERS`.
	pub const IGNORE_ENEMY_ENGINEERS: c_int = 0x0040;

	/// `TFBOT_IGNORE_ENEMY_HEAVIES`.
	pub const IGNORE_ENEMY_HEAVIES: c_int = 0x0010;

	/// `TFBOT_IGNORE_ENEMY_MEDICS`.
	pub const IGNORE_ENEMY_MEDICS: c_int = 0x0020;

	/// `TFBOT_IGNORE_ENEMY_PYROS`.
	pub const IGNORE_ENEMY_PYROS: c_int = 0x0004;

	/// `TFBOT_IGNORE_ENEMY_SCOUTS`.
	pub const IGNORE_ENEMY_SCOUTS: c_int = 0x0001;

	/// `TFBOT_IGNORE_ENEMY_SENTRY_GUNS`.
	pub const IGNORE_ENEMY_SENTRY_GUNS: c_int = 0x0200;

	/// `TFBOT_IGNORE_ENEMY_SNIPERS`.
	pub const IGNORE_ENEMY_SNIPERS: c_int = 0x0080;

	/// `TFBOT_IGNORE_ENEMY_SOLDIERS`.
	pub const IGNORE_ENEMY_SOLDIERS: c_int = 0x0002;

	/// `TFBOT_IGNORE_ENEMY_SPIES`.
	pub const IGNORE_ENEMY_SPIES: c_int = 0x0100;

	/// `TFBOT_IGNORE_SCENARIO_GOALS`.
	pub const IGNORE_SCENARIO_GOALS: c_int = 0x0400;
}

/// `CTFBot::DifficultyType`: a bot's skill, which `SetDifficulty` takes and
/// `GetDifficulty` returns.
pub mod difficulty {
	use std::ffi::c_int;

	/// `EASY`.
	pub const EASY: c_int = 0;

	/// `EXPERT`.
	pub const EXPERT: c_int = 3;

	/// `HARD`.
	pub const HARD: c_int = 2;

	/// `NORMAL`.
	pub const NORMAL: c_int = 1;

	/// `UNDEFINED`, which no bot is given.
	pub const UNDEFINED: c_int = -1;
}

/// `CTFBot::MissionType`: what a bot is doing, which `SetMission` takes and
/// `GetMission` returns.
pub mod mission {
	use std::ffi::c_int;

	/// `MISSION_DESTROY_SENTRIES`: finds and destroys enemy sentry guns and
	/// other buildings.
	pub const DESTROY_SENTRIES: c_int = 2;

	/// `MISSION_ENGINEER`: harasses the enemy from an Engineer's nest.
	pub const ENGINEER: c_int = 5;

	/// `NO_MISSION`.
	pub const NO_MISSION: c_int = 0;

	/// `MISSION_REPROGRAMMED`: a Mann vs. Machine robot hacked to turn on its
	/// team.
	pub const REPROGRAMMED: c_int = 6;

	/// `MISSION_SEEK_AND_DESTROY`: finds and kills enemy players.
	pub const SEEK_AND_DESTROY: c_int = 1;

	/// `MISSION_SNIPER`: harasses the enemy as a team of Snipers.
	pub const SNIPER: c_int = 3;

	/// `MISSION_SPY`: harasses the enemy as a team of Spies.
	pub const SPY: c_int = 4;
}

/// `CTFBot::WeaponRestrictionType`: the bits of a bot's weapon restrictions,
/// which `AddWeaponRestriction` and its siblings take. No bit, `ANY_WEAPON`,
/// restricts nothing.
pub mod weapon_restriction {
	use std::ffi::c_int;

	/// `MELEE_ONLY`.
	pub const MELEE_ONLY: c_int = 0x0001;

	/// `PRIMARY_ONLY`.
	pub const PRIMARY_ONLY: c_int = 0x0002;

	/// `SECONDARY_ONLY`.
	pub const SECONDARY_ONLY: c_int = 0x0004;
}
