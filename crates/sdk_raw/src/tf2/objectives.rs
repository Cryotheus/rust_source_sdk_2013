//! Hand-written values of TF2's objective entities: round timers, control
//! points, capture flags, round wins and spawn points, from the headers of
//! `game/shared` and `game/server` that the generated bindings do not include.

use std::ffi::{CStr, c_int};

/// The bits of `CBaseTeamObjectiveResource::m_iUpdateCapHudParity`, which the
/// game advances, modulo [`CAPHUD_PARITY_MASK`] + 1, whenever it changes what
/// clients' control point HUD shows.
///
/// This is `CAPHUD_PARITY_BITS` from `game/server/team_objectiveresource.cpp`.
pub const CAPHUD_PARITY_BITS: c_int = 6;

/// The mask that keeps `m_iUpdateCapHudParity` within its
/// [`CAPHUD_PARITY_BITS`].
///
/// This is `CAPHUD_PARITY_MASK` from `game/server/team_objectiveresource.cpp`.
pub const CAPHUD_PARITY_MASK: c_int = (1 << CAPHUD_PARITY_BITS) - 1;

/// The name `tf_logic_koth` gives the round timer it creates for BLU, at each
/// round's `RoundSpawn` (`CKothLogic::InputRoundSpawn`,
/// `game/shared/tf/tf_gamerules.cpp`).
pub const KOTH_BLUE_TIMER_NAME: &CStr = c"zz_blue_koth_timer";

/// The name `tf_logic_koth` gives the round timer it creates for RED, as for
/// [`KOTH_BLUE_TIMER_NAME`].
pub const KOTH_RED_TIMER_NAME: &CStr = c"zz_red_koth_timer";

/// The size of the objective resource's per-team data of each control point,
/// such as `m_bTeamCanCap`, which holds a point's entry for `team` at
/// `point + team * MAX_CONTROL_POINTS`.
///
/// This is `MAX_CONTROL_POINT_TEAMS` from `game/shared/shareddefs.h`.
pub const MAX_CONTROL_POINT_TEAMS: c_int = 8;

/// A spawn point players spawn at whenever it is enabled.
///
/// This is `PlayerTeamSpawnMode_Normal` from `game/server/tf/entity_tfstart.h`.
#[doc(alias("PlayerTeamSpawnMode_Normal"))]
pub const PLAYER_TEAM_SPAWN_MODE_NORMAL: c_int = 0;

/// A spawn point players spawn at only after a `trigger_player_respawn_override`
/// or a map's logic sends them to it.
///
/// This is `PlayerTeamSpawnMode_Triggered` from
/// `game/server/tf/entity_tfstart.h`.
#[doc(alias("PlayerTeamSpawnMode_Triggered"))]
pub const PLAYER_TEAM_SPAWN_MODE_TRIGGERED: c_int = 1;

/// A round timer counting down the round itself.
///
/// This is `RT_STATE_NORMAL` from `game/shared/shareddefs.h`.
pub const RT_STATE_NORMAL: c_int = 1;

/// A round timer counting down the setup before the round, while the gates
/// stay closed.
///
/// This is `RT_STATE_SETUP` from `game/shared/shareddefs.h`.
pub const RT_STATE_SETUP: c_int = 0;

/// The `team_control_point` spawn flag that keeps bots from capturing or
/// defending the point.
///
/// This is `SF_CAP_POINT_BOTS_IGNORE` from `game/server/team_control_point.h`.
pub const SF_CAP_POINT_BOTS_IGNORE: c_int = 1 << 4;

/// The `team_control_point` spawn flag that hides the point's flag model.
///
/// This is `SF_CAP_POINT_HIDEFLAG` from `game/server/team_control_point.h`.
#[doc(alias("SF_CAP_POINT_HIDEFLAG"))]
pub const SF_CAP_POINT_HIDE_FLAG: c_int = 1 << 0;

/// The `team_control_point` spawn flag that hides the point's model.
///
/// This is `SF_CAP_POINT_HIDE_MODEL` from `game/server/team_control_point.h`.
pub const SF_CAP_POINT_HIDE_MODEL: c_int = 1 << 1;

/// The `team_control_point` spawn flag that hides the point's shadow.
///
/// This is `SF_CAP_POINT_HIDE_SHADOW` from `game/server/team_control_point.h`.
pub const SF_CAP_POINT_HIDE_SHADOW: c_int = 1 << 2;

/// The `team_control_point` spawn flag that silences its capture sounds.
///
/// This is `SF_CAP_POINT_NO_CAP_SOUNDS` from `game/server/team_control_point.h`.
pub const SF_CAP_POINT_NO_CAP_SOUNDS: c_int = 1 << 3;

/// The spawn flag that keeps an item from respawning once picked up, which
/// TF2 also sets on the health and ammo it drops.
///
/// This is `SF_NORESPAWN` from `game/server/player.h`.
pub const SF_NORESPAWN: c_int = 1 << 30;

/// A capture flag dropped away from its home, which returns home once its
/// return time runs out.
///
/// This is `TF_FLAGINFO_DROPPED` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_FLAGINFO_DROPPED: c_int = 1 << 1;

/// A capture flag at its home position.
///
/// This is `TF_FLAGINFO_HOME` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_FLAGINFO_HOME: c_int = 0;

/// A capture flag that a player of the other team carries.
///
/// This is `TF_FLAGINFO_STOLEN` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_FLAGINFO_STOLEN: c_int = 1 << 0;

/// The capture flag of attack and defend, which only one team can carry.
///
/// This is `TF_FLAGTYPE_ATTACK_DEFEND` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_FLAGTYPE_ATTACK_DEFEND: c_int = 1;

/// The intelligence of capture the flag.
///
/// This is `TF_FLAGTYPE_CTF` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_FLAGTYPE_CTF: c_int = 0;

/// The capture flag of an invasion, such as Doomsday's Australium.
///
/// This is `TF_FLAGTYPE_INVADE` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_FLAGTYPE_INVADE: c_int = 3;

/// The bottle a Player Destruction player carries.
///
/// This is `TF_FLAGTYPE_PLAYER_DESTRUCTION` from
/// `game/shared/tf/tf_shareddefs.h`.
pub const TF_FLAGTYPE_PLAYER_DESTRUCTION: c_int = 6;

/// A resource-control flag.
///
/// This is `TF_FLAGTYPE_RESOURCE_CONTROL` from `game/shared/tf/tf_shareddefs.h`.
pub const TF_FLAGTYPE_RESOURCE_CONTROL: c_int = 4;

/// The reactor core of Robot Destruction.
///
/// This is `TF_FLAGTYPE_ROBOT_DESTRUCTION` from
/// `game/shared/tf/tf_shareddefs.h`.
pub const TF_FLAGTYPE_ROBOT_DESTRUCTION: c_int = 5;

/// A territory-control flag.
///
/// This is `TF_FLAGTYPE_TERRITORY_CONTROL` from
/// `game/shared/tf/tf_shareddefs.h`.
pub const TF_FLAGTYPE_TERRITORY_CONTROL: c_int = 2;

/// Every control point was captured.
///
/// This is `WINREASON_ALL_POINTS_CAPTURED` from
/// `game/shared/teamplayroundbased_gamerules.h`, as are the other `WINREASON_`
/// values, which TF2 numbers in this order.
pub const WINREASON_ALL_POINTS_CAPTURED: c_int = 1;

/// The defenders held out until the round timer ran out, which a
/// `game_round_win` reports unless its `win_reason` key says otherwise.
pub const WINREASON_DEFEND_UNTIL_TIME_LIMIT: c_int = 4;

/// The flag capture limit was reached.
pub const WINREASON_FLAG_CAPTURE_LIMIT: c_int = 3;

/// No reason.
pub const WINREASON_NONE: c_int = 0;

/// The other team was eliminated, as in Arena.
pub const WINREASON_OPPONENTS_DEAD: c_int = 2;

/// Player Destruction's points were collected.
pub const WINREASON_PD_POINTS: c_int = 12;

/// Robot Destruction's reactor cores were collected.
pub const WINREASON_RD_CORES_COLLECTED: c_int = 10;

/// Robot Destruction's reactor core was captured.
pub const WINREASON_RD_REACTOR_CAPTURED: c_int = 9;

/// Robot Destruction's reactor core was returned.
pub const WINREASON_RD_REACTOR_RETURNED: c_int = 11;

/// A team scored, as in Passtime.
pub const WINREASON_SCORED: c_int = 13;

/// The round ended in a stalemate.
pub const WINREASON_STALEMATE: c_int = 5;

/// A stopwatch round was won by the team playing it.
pub const WINREASON_STOPWATCH_PLAYING_ROUNDS: c_int = 16;

/// The final stopwatch round was won by the team watching it.
pub const WINREASON_STOPWATCH_WATCHING_FINAL_ROUND: c_int = 15;

/// A stopwatch round was won by the team watching it.
pub const WINREASON_STOPWATCH_WATCHING_ROUNDS: c_int = 14;

/// The map's time limit ran out.
pub const WINREASON_TIMELIMIT: c_int = 6;

/// The win difference limit was reached.
pub const WINREASON_WINDIFFLIMIT: c_int = 8;

/// The win limit was reached.
pub const WINREASON_WINLIMIT: c_int = 7;
