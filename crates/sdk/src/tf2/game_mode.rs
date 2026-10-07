//! TF2's game modes, as its game rules hold them: the level's game type and
//! HUD type, the flags of the modes that share a game type, such as King of
//! the Hill's, and the holiday and Halloween scenario the level is made for.
//!
//! The game decides most of them as the `tf_gamerules` entity activates, after
//! the level's entities spawn (`CTFGameRules::Activate`), from the logic
//! entities the level has, such as `tf_logic_koth` and `tf_logic_holiday`,
//! and the HUD type from that entity's `hud_type` key. Read them
//! from callbacks after that, such as a server activation's. The Halloween
//! scenario follows the level's name as the game rules are created, and
//! Mannpower follows `tf_powerup_mode`.
//!
//! The predicates are those of `game/shared/tf/tf_gamerules.h`, such as
//! `IsInKothMode`, which read the same networked variables. VScript's
//! functions of the same names are registered with the script VM alone, so
//! they cannot be called through the entities' script descriptors.

#[cfg(test)]
#[path = "../tests/tf2/game_mode.rs"]
mod tests;

use crate::interfaces::ValveEngine;
use crate::tf2::game_rules::{GameRules, GameRulesError};

use sdk_raw::tf2::game_mode::{
	HALLOWEEN_SCENARIO_DOOMSDAY, HALLOWEEN_SCENARIO_HIGHTOWER, HALLOWEEN_SCENARIO_LAKESIDE,
	HALLOWEEN_SCENARIO_MANN_MANOR, HALLOWEEN_SCENARIO_NONE, HALLOWEEN_SCENARIO_VIADUCT,
	HOLIDAY_APRIL_FOOLS, HOLIDAY_CHRISTMAS, HOLIDAY_COMMUNITY_UPDATE, HOLIDAY_EOTL,
	HOLIDAY_FULL_MOON, HOLIDAY_HALLOWEEN, HOLIDAY_HALLOWEEN_OR_FULL_MOON,
	HOLIDAY_HALLOWEEN_OR_FULL_MOON_OR_VALENTINES, HOLIDAY_MEET_THE_PYRO, HOLIDAY_NONE,
	HOLIDAY_SOLDIER, HOLIDAY_SUMMER, HOLIDAY_TF_BIRTHDAY, HOLIDAY_VALENTINES, TF_GAMETYPE_ARENA,
	TF_GAMETYPE_CP, TF_GAMETYPE_CTF, TF_GAMETYPE_ESCORT, TF_GAMETYPE_MVM, TF_GAMETYPE_PASSTIME,
	TF_GAMETYPE_PD, TF_GAMETYPE_RD, TF_GAMETYPE_UNDEFINED, TF_HUDTYPE_ARENA, TF_HUDTYPE_CP,
	TF_HUDTYPE_CTF, TF_HUDTYPE_ESCORT, TF_HUDTYPE_TRAINING, TF_HUDTYPE_UNDEFINED,
};

use std::ffi::{CStr, c_int};

/// The `m_nGameType` of the game rules.
const GAME_TYPE: &CStr = c"m_nGameType";

/// The `m_halloweenScenario` of the game rules.
const HALLOWEEN_SCENARIO: &CStr = c"m_halloweenScenario";

/// The `m_nHudType` of the game rules.
const HUD_TYPE: &CStr = c"m_nHudType";

/// The `m_nMapHolidayType` of the game rules.
const MAP_HOLIDAY: &CStr = c"m_nMapHolidayType";

/// The `m_bIsUsingSpells` of the game rules.
const USES_SPELLS: &CStr = c"m_bIsUsingSpells";

/// The game type of a level (`ETFGameType`), which TF2 decides from its
/// objective entities, and which decides the HUD clients show for it, unless
/// the level sets a [`HudType`].
///
/// King of the Hill, Medieval Mode, Mannpower and the other modes that share
/// a game type have their own predicates on [`GameRules`], such as
/// [`GameRules::is_king_of_the_hill`].
#[doc(alias("ETFGameType"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum GameType {
	/// No objectives that set another game type, as in Special Delivery
	/// (`sd_`) levels and levels without objectives.
	#[doc(alias("TF_GAMETYPE_UNDEFINED"))]
	Undefined = TF_GAMETYPE_UNDEFINED,

	/// Capture the Flag, from the level's flags, also Mannpower's.
	#[doc(alias("TF_GAMETYPE_CTF"))]
	CaptureTheFlag = TF_GAMETYPE_CTF,

	/// Control points, from the level's `team_control_point_master`, also King
	/// of the Hill's.
	#[doc(alias("TF_GAMETYPE_CP"))]
	ControlPoints = TF_GAMETYPE_CP,

	/// Payload and Payload Race, from the level's `team_train_watcher`.
	#[doc(alias("TF_GAMETYPE_ESCORT"))]
	Payload = TF_GAMETYPE_ESCORT,

	/// Arena, from the level's `tf_logic_arena`.
	#[doc(alias("TF_GAMETYPE_ARENA", "IsInArenaMode"))]
	Arena = TF_GAMETYPE_ARENA,

	/// Mann vs. Machine, from the level's `tf_logic_mann_vs_machine`.
	#[doc(alias("TF_GAMETYPE_MVM"))]
	MannVsMachine = TF_GAMETYPE_MVM,

	/// Robot Destruction, from the level's `tf_logic_robot_destruction`.
	#[doc(alias("TF_GAMETYPE_RD"))]
	RobotDestruction = TF_GAMETYPE_RD,

	/// PASS Time, from the level's `passtime_logic`.
	#[doc(alias("TF_GAMETYPE_PASSTIME", "IsPasstimeMode"))]
	PassTime = TF_GAMETYPE_PASSTIME,

	/// Player Destruction, from the level's `tf_logic_player_destruction`.
	#[doc(alias("TF_GAMETYPE_PD"))]
	PlayerDestruction = TF_GAMETYPE_PD,
}

impl GameType {
	/// Every game type, in the game's order.
	pub const ALL: [Self; 9] = [
		Self::Undefined,
		Self::CaptureTheFlag,
		Self::ControlPoints,
		Self::Payload,
		Self::Arena,
		Self::MannVsMachine,
		Self::RobotDestruction,
		Self::PassTime,
		Self::PlayerDestruction,
	];

	/// The game type the game stores as `raw`, or `None` for any other value.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			TF_GAMETYPE_UNDEFINED => Some(Self::Undefined),
			TF_GAMETYPE_CTF => Some(Self::CaptureTheFlag),
			TF_GAMETYPE_CP => Some(Self::ControlPoints),
			TF_GAMETYPE_ESCORT => Some(Self::Payload),
			TF_GAMETYPE_ARENA => Some(Self::Arena),
			TF_GAMETYPE_MVM => Some(Self::MannVsMachine),
			TF_GAMETYPE_RD => Some(Self::RobotDestruction),
			TF_GAMETYPE_PASSTIME => Some(Self::PassTime),
			TF_GAMETYPE_PD => Some(Self::PlayerDestruction),
			_ => None,
		}
	}

	/// The value the game stores for the game type, as `m_nGameType` holds it.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

/// The Halloween scenario of a level (`HalloweenScenarioType`), which the
/// game decides from the level's name, for Halloween's bosses and effects.
#[doc(alias("HalloweenScenarioType"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum HalloweenScenario {
	/// Mann Manor (`cp_manor_event`), with the Horseless Headless Horsemann.
	#[doc(alias("HALLOWEEN_SCENARIO_MANN_MANOR"))]
	MannManor = HALLOWEEN_SCENARIO_MANN_MANOR,

	/// Eyeaduct (`koth_viaduct_event`), with Monoculus.
	#[doc(alias("HALLOWEEN_SCENARIO_VIADUCT"))]
	Eyeaduct = HALLOWEEN_SCENARIO_VIADUCT,

	/// Ghost Fort (`koth_lakeside_event`), with Merasmus.
	#[doc(alias("HALLOWEEN_SCENARIO_LAKESIDE"))]
	GhostFort = HALLOWEEN_SCENARIO_LAKESIDE,

	/// Helltower (`plr_hightower_event`), whose players always use spells.
	#[doc(alias("HALLOWEEN_SCENARIO_HIGHTOWER"))]
	Helltower = HALLOWEEN_SCENARIO_HIGHTOWER,

	/// Carnival of Carnage (`sd_doomsday_event`).
	#[doc(alias("HALLOWEEN_SCENARIO_DOOMSDAY"))]
	CarnivalOfCarnage = HALLOWEEN_SCENARIO_DOOMSDAY,
}

impl HalloweenScenario {
	/// Every scenario, in the game's order.
	pub const ALL: [Self; 5] = [
		Self::MannManor,
		Self::Eyeaduct,
		Self::GhostFort,
		Self::Helltower,
		Self::CarnivalOfCarnage,
	];

	/// The scenario the game stores as `raw`, or `None` for no scenario
	/// (`HALLOWEEN_SCENARIO_NONE`) and for any value it does not know.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			HALLOWEEN_SCENARIO_MANN_MANOR => Some(Self::MannManor),
			HALLOWEEN_SCENARIO_VIADUCT => Some(Self::Eyeaduct),
			HALLOWEEN_SCENARIO_LAKESIDE => Some(Self::GhostFort),
			HALLOWEEN_SCENARIO_HIGHTOWER => Some(Self::Helltower),
			HALLOWEEN_SCENARIO_DOOMSDAY => Some(Self::CarnivalOfCarnage),
			_ => None,
		}
	}

	/// The value the game stores for the scenario, as `m_halloweenScenario`
	/// holds it.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

/// A holiday of TF2's item schema (`EHoliday`), which a level can be made
/// for, and which decides when holiday items and effects are active.
///
/// The combined holidays, such as [`Self::HalloweenOrFullMoon`], are active
/// while any of theirs is.
#[doc(alias("EHoliday"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Holiday {
	/// TF2's birthday, around the 24th of August.
	#[doc(alias("kHoliday_TFBirthday"))]
	Birthday = HOLIDAY_TF_BIRTHDAY,

	/// Halloween, the Scream Fortress event.
	#[doc(alias("kHoliday_Halloween"))]
	Halloween = HOLIDAY_HALLOWEEN,

	/// Christmas, the Smissmas event.
	#[doc(alias("kHoliday_Christmas"))]
	Christmas = HOLIDAY_CHRISTMAS,

	/// A community update.
	#[doc(alias("kHoliday_CommunityUpdate"))]
	CommunityUpdate = HOLIDAY_COMMUNITY_UPDATE,

	/// The End of the Line update.
	#[doc(alias("kHoliday_EOTL"))]
	EndOfTheLine = HOLIDAY_EOTL,

	/// Valentine's Day.
	#[doc(alias("kHoliday_Valentines"))]
	Valentines = HOLIDAY_VALENTINES,

	/// The Meet the Pyro event.
	#[doc(alias("kHoliday_MeetThePyro"))]
	MeetThePyro = HOLIDAY_MEET_THE_PYRO,

	/// A full moon.
	#[doc(alias("kHoliday_FullMoon"))]
	FullMoon = HOLIDAY_FULL_MOON,

	/// Halloween or a full moon.
	#[doc(alias("kHoliday_HalloweenOrFullMoon"))]
	HalloweenOrFullMoon = HOLIDAY_HALLOWEEN_OR_FULL_MOON,

	/// Halloween, a full moon, or Valentine's Day.
	#[doc(alias("kHoliday_HalloweenOrFullMoonOrValentines"))]
	HalloweenOrFullMoonOrValentines = HOLIDAY_HALLOWEEN_OR_FULL_MOON_OR_VALENTINES,

	/// April Fools' Day.
	#[doc(alias("kHoliday_AprilFools"))]
	AprilFools = HOLIDAY_APRIL_FOOLS,

	/// The Soldier event.
	#[doc(alias("kHoliday_Soldier"))]
	Soldier = HOLIDAY_SOLDIER,

	/// The Summer event.
	#[doc(alias("kHoliday_Summer"))]
	Summer = HOLIDAY_SUMMER,
}

impl Holiday {
	/// Every holiday, in the game's order.
	pub const ALL: [Self; 13] = [
		Self::Birthday,
		Self::Halloween,
		Self::Christmas,
		Self::CommunityUpdate,
		Self::EndOfTheLine,
		Self::Valentines,
		Self::MeetThePyro,
		Self::FullMoon,
		Self::HalloweenOrFullMoon,
		Self::HalloweenOrFullMoonOrValentines,
		Self::AprilFools,
		Self::Soldier,
		Self::Summer,
	];

	/// The holiday the game stores as `raw`, or `None` for no holiday
	/// (`kHoliday_None`) and for any value it does not know.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			HOLIDAY_TF_BIRTHDAY => Some(Self::Birthday),
			HOLIDAY_HALLOWEEN => Some(Self::Halloween),
			HOLIDAY_CHRISTMAS => Some(Self::Christmas),
			HOLIDAY_COMMUNITY_UPDATE => Some(Self::CommunityUpdate),
			HOLIDAY_EOTL => Some(Self::EndOfTheLine),
			HOLIDAY_VALENTINES => Some(Self::Valentines),
			HOLIDAY_MEET_THE_PYRO => Some(Self::MeetThePyro),
			HOLIDAY_FULL_MOON => Some(Self::FullMoon),
			HOLIDAY_HALLOWEEN_OR_FULL_MOON => Some(Self::HalloweenOrFullMoon),

			HOLIDAY_HALLOWEEN_OR_FULL_MOON_OR_VALENTINES => {
				Some(Self::HalloweenOrFullMoonOrValentines)
			}

			HOLIDAY_APRIL_FOOLS => Some(Self::AprilFools),
			HOLIDAY_SOLDIER => Some(Self::Soldier),
			HOLIDAY_SUMMER => Some(Self::Summer),
			_ => None,
		}
	}

	/// The value the game stores for the holiday.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

/// The HUD a level has clients show, in place of its game type's, as the
/// `hud_type` key of its `tf_gamerules` entity sets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum HudType {
	/// The game type's own HUD.
	#[doc(alias("TF_HUDTYPE_UNDEFINED"))]
	Undefined = TF_HUDTYPE_UNDEFINED,

	/// Capture the Flag's HUD.
	#[doc(alias("TF_HUDTYPE_CTF"))]
	CaptureTheFlag = TF_HUDTYPE_CTF,

	/// The control points' HUD.
	#[doc(alias("TF_HUDTYPE_CP"))]
	ControlPoints = TF_HUDTYPE_CP,

	/// Payload's HUD.
	#[doc(alias("TF_HUDTYPE_ESCORT"))]
	Payload = TF_HUDTYPE_ESCORT,

	/// Arena's HUD. The game never sets it from the key.
	#[doc(alias("TF_HUDTYPE_ARENA"))]
	Arena = TF_HUDTYPE_ARENA,

	/// The training levels' HUD.
	#[doc(alias("TF_HUDTYPE_TRAINING"))]
	Training = TF_HUDTYPE_TRAINING,
}

impl HudType {
	/// Every HUD type, in the game's order.
	pub const ALL: [Self; 6] = [
		Self::Undefined,
		Self::CaptureTheFlag,
		Self::ControlPoints,
		Self::Payload,
		Self::Arena,
		Self::Training,
	];

	/// The HUD type the game stores as `raw`, or `None` for any other value.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			TF_HUDTYPE_UNDEFINED => Some(Self::Undefined),
			TF_HUDTYPE_CTF => Some(Self::CaptureTheFlag),
			TF_HUDTYPE_CP => Some(Self::ControlPoints),
			TF_HUDTYPE_ESCORT => Some(Self::Payload),
			TF_HUDTYPE_ARENA => Some(Self::Arena),
			TF_HUDTYPE_TRAINING => Some(Self::Training),
			_ => None,
		}
	}

	/// The value the game stores for the HUD type, as `m_nHudType` holds it.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

impl GameRules<'_> {
	/// Whether Helltower's players are in the underworld
	/// (`m_bHelltowerPlayersInHell`), as Helltower's logic entity or a script's
	/// `SetPlayersInHell` sets it.
	#[doc(alias("m_bHelltowerPlayersInHell", "ArePlayersInHell"))]
	pub fn are_players_in_hell(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bHelltowerPlayersInHell")
	}

	/// The level's game type (`m_nGameType`).
	///
	/// Fails with [`GameRulesError::UnknownValue`] for a value [`GameType`]
	/// does not know.
	#[doc(alias("m_nGameType", "GetGameType"))]
	pub fn game_type(self) -> Result<GameType, GameRulesError> {
		let raw = self.read::<c_int>(GAME_TYPE)?;

		GameType::from_raw(raw).ok_or(GameRulesError::UnknownValue {
			variable: GAME_TYPE,
			value: raw,
		})
	}

	/// The level's Halloween scenario (`m_halloweenScenario`), or `None` for a
	/// level without one.
	///
	/// Fails with [`GameRulesError::UnknownValue`] for a value
	/// [`HalloweenScenario`] does not know.
	#[doc(alias("m_halloweenScenario", "GetHalloweenScenario"))]
	pub fn halloween_scenario(self) -> Result<Option<HalloweenScenario>, GameRulesError> {
		let raw = self.read::<c_int>(HALLOWEEN_SCENARIO)?;

		match HalloweenScenario::from_raw(raw) {
			None if raw != HALLOWEEN_SCENARIO_NONE => Err(GameRulesError::UnknownValue {
				variable: HALLOWEEN_SCENARIO,
				value: raw,
			}),

			scenario => Ok(scenario),
		}
	}

	/// Whether the game is a matchmade competitive or casual match
	/// (`m_bCompetitiveMode`).
	#[doc(alias("m_bCompetitiveMode", "IsCompetitiveMode"))]
	pub fn is_competitive(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bCompetitiveMode")
	}

	/// Whether the level is a hybrid of Capture the Flag and control points
	/// (`m_bPlayingHybrid_CTF_CP`), from its `tf_logic_hybrid_ctf_cp`.
	#[doc(alias("m_bPlayingHybrid_CTF_CP", "IsPlayingHybrid_CTF_CP"))]
	pub fn is_hybrid_ctf_cp(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bPlayingHybrid_CTF_CP")
	}

	/// Whether the level plays King of the Hill (`m_bPlayingKoth`), from its
	/// `tf_logic_koth`.
	#[doc(alias("m_bPlayingKoth", "IsInKothMode"))]
	pub fn is_king_of_the_hill(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bPlayingKoth")
	}

	/// Whether the level plays Mann vs. Machine (`m_bPlayingMannVsMachine`).
	#[doc(alias("m_bPlayingMannVsMachine", "IsMannVsMachineMode"))]
	pub fn is_mann_vs_machine(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bPlayingMannVsMachine")
	}

	/// Whether the game plays Mannpower (`m_bPowerupMode`), as
	/// `tf_powerup_mode` sets it.
	#[doc(alias("m_bPowerupMode", "IsPowerupMode"))]
	pub fn is_mannpower(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bPowerupMode")
	}

	/// Whether the level plays Medieval Mode (`m_bPlayingMedieval`), from its
	/// `tf_logic_medieval` or `tf_medieval`.
	#[doc(alias("m_bPlayingMedieval", "IsInMedievalMode"))]
	pub fn is_medieval(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bPlayingMedieval")
	}

	/// Whether the level plays Robot Destruction or Player Destruction
	/// (`m_bPlayingRobotDestructionMode`), which share their logic entity. The
	/// [game type](Self::game_type) tells them apart.
	#[doc(alias("m_bPlayingRobotDestructionMode", "IsPlayingRobotDestructionMode"))]
	pub fn is_robot_destruction(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bPlayingRobotDestructionMode")
	}

	/// Whether the level plays Special Delivery
	/// (`m_bPlayingSpecialDeliveryMode`), as levels named `sd_` do.
	#[doc(alias("m_bPlayingSpecialDeliveryMode", "IsPlayingSpecialDeliveryMode"))]
	pub fn is_special_delivery(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bPlayingSpecialDeliveryMode")
	}

	/// Whether a truce holds (`m_bTruceActive`), as while a Halloween boss
	/// fights both teams, during which players cannot hurt those of other teams.
	#[doc(alias("m_bTruceActive", "IsTruceActive"))]
	pub fn is_truce_active(self) -> Result<bool, GameRulesError> {
		self.read(c"m_bTruceActive")
	}

	/// The HUD the level has clients show (`m_nHudType`).
	///
	/// Fails with [`GameRulesError::UnknownValue`] for a value [`HudType`]
	/// does not know.
	#[doc(alias("m_nHudType", "GetHUDType"))]
	pub fn hud_type(self) -> Result<HudType, GameRulesError> {
		let raw = self.read::<c_int>(HUD_TYPE)?;

		HudType::from_raw(raw).ok_or(GameRulesError::UnknownValue {
			variable: HUD_TYPE,
			value: raw,
		})
	}

	/// The holiday the level is made for (`m_nMapHolidayType`), from its
	/// `tf_logic_holiday`, or `None` for a level made for none.
	///
	/// Fails with [`GameRulesError::UnknownValue`] for a value [`Holiday`]
	/// does not know.
	#[doc(alias("m_nMapHolidayType", "IsHolidayMap"))]
	pub fn map_holiday(self) -> Result<Option<Holiday>, GameRulesError> {
		let raw = self.read::<c_int>(MAP_HOLIDAY)?;

		match Holiday::from_raw(raw) {
			None if raw != HOLIDAY_NONE => Err(GameRulesError::UnknownValue {
				variable: MAP_HOLIDAY,
				value: raw,
			}),

			holiday => Ok(holiday),
		}
	}

	/// Whether the level turned spells on (`m_bIsUsingSpells`), as a
	/// `tf_logic_holiday`'s `HalloweenSetUsingSpells` input or a script's
	/// `SetUsingSpells` does, so players pick up spellbooks.
	///
	/// The game's own `IsUsingSpells` is also true while `tf_spells_enabled`
	/// is set, and on [Helltower](HalloweenScenario::Helltower), neither of
	/// which this reads.
	#[doc(alias("m_bIsUsingSpells"))]
	pub fn uses_spells(self) -> Result<bool, GameRulesError> {
		self.read(USES_SPELLS)
	}

	/// Turns the level's spells on or off, as [`Self::uses_spells`] reads
	/// them, and records the change for clients, as `CTFGameRules`'
	/// `SetUsingSpells` does.
	#[doc(alias("SetUsingSpells", "HalloweenSetUsingSpells"))]
	pub fn set_uses_spells(
		self,
		engine: ValveEngine<'_>,
		using: bool,
	) -> Result<(), GameRulesError> {
		// SAFETY: The game assigns the flag either value through
		// `SetUsingSpells`, from scripts and inputs alike, and reads it only to
		// decide whether players may use spells.
		unsafe { self.set(engine, USES_SPELLS, using) }
	}
}
