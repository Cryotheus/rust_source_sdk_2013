use super::GameEvent;
use crate::players::UserId;
use std::ffi::{CString, c_int};

#[derive(Debug, Clone)]
pub struct GeserPlayerDeath {
	/// > user ID who died
	userid: UserId,

	victim_entindex: Option<UserId>,

	/// > ent index of inflictor (a sentry, for example)
	inflictor_entindex: c_int,

	/// > user ID who killed
	attacker: Option<UserId>,

	/// > weapon name killer used
	weapon: CString,

	/// > ID of weapon killer used
	weaponid: c_int,

	/// > bits of type of damage
	damagebits: c_int,

	/// > type of custom kill
	customkill: c_int,

	/// > user ID of assister
	assister: Option<UserId>,

	/// > weapon name that should be printed on the log
	weapon_logclassname: CString,

	/// > victim's stun flags at the moment of death
	stun_flags: c_int,

	/// >death flags.
	death_flags: c_int,

	silent_kill: bool,

	playerpenetratecount: c_int,

	/// > contains a string to use if "assister" is -1
	assister_fallback: CString,

	/// > Kill streak count (level)
	kill_streak_total: c_int,

	/// > Kill streak for killing weapon
	kill_streak_wep: c_int,

	/// > Kill streak for assister count
	kill_streak_assist: c_int,

	/// > Victims kill streak
	kill_streak_victim: c_int,

	/// > Duck streak increment from this kill
	ducks_streaked: c_int,

	/// > Duck streak count for attacker
	duck_streak_total: c_int,

	/// > Duck streak count for assister
	duck_streak_assist: c_int,

	/// > (former) duck streak count for victim
	duck_streak_victim: c_int,

	/// > was the victim rocket jumping
	rocket_jump: bool,

	/// > item def index of weapon killer used
	weapon_def_index: c_int,

	/// > Crit type of kill.  0: None 1: Mini 2: Full
	crit_type: c_int,
}

impl GeserPlayerDeath {
	pub fn deserialize(event: GameEvent) -> Result<Self, GeserError> {
		todo!()
	}
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum GeserError {
	#[error("GameEventId does not match schema's expected ID")]
	GameEventIdMismatch,
}
