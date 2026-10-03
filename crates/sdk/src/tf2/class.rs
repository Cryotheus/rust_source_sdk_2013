//! TF2's playable classes.

use std::ffi::c_int;

/// One of TF2's nine playable classes, numbered as the `TF_CLASS_*` constants
/// in `game/shared/tf/tf_shareddefs.h`.
///
/// The Civilian (`TF_CLASS_CIVILIAN`, 10), which players cannot choose and
/// which has no voice lines, is left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlayerClass {
	/// `TF_CLASS_SCOUT`: 1.
	#[doc(alias = "TF_CLASS_SCOUT")]
	Scout = 1,

	/// `TF_CLASS_SNIPER`: 2.
	#[doc(alias = "TF_CLASS_SNIPER")]
	Sniper = 2,

	/// `TF_CLASS_SOLDIER`: 3.
	#[doc(alias = "TF_CLASS_SOLDIER")]
	Soldier = 3,

	/// `TF_CLASS_DEMOMAN`: 4.
	#[doc(alias = "TF_CLASS_DEMOMAN")]
	Demoman = 4,

	/// `TF_CLASS_MEDIC`: 5.
	#[doc(alias = "TF_CLASS_MEDIC")]
	Medic = 5,

	/// `TF_CLASS_HEAVYWEAPONS`: 6.
	#[doc(alias = "TF_CLASS_HEAVYWEAPONS")]
	Heavy = 6,

	/// `TF_CLASS_PYRO`: 7.
	#[doc(alias = "TF_CLASS_PYRO")]
	Pyro = 7,

	/// `TF_CLASS_SPY`: 8.
	#[doc(alias = "TF_CLASS_SPY")]
	Spy = 8,

	/// `TF_CLASS_ENGINEER`: 9.
	#[doc(alias = "TF_CLASS_ENGINEER")]
	Engineer = 9,
}

impl PlayerClass {
	/// Every class, in native order.
	pub const ALL: [Self; 9] = [
		Self::Scout,
		Self::Sniper,
		Self::Soldier,
		Self::Demoman,
		Self::Medic,
		Self::Heavy,
		Self::Pyro,
		Self::Spy,
		Self::Engineer,
	];

	/// The class with this `TF_CLASS_*` number, or `None` for any other
	/// number, including `TF_CLASS_UNDEFINED` (0) and the Civilian (10).
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			1 => Some(Self::Scout),
			2 => Some(Self::Sniper),
			3 => Some(Self::Soldier),
			4 => Some(Self::Demoman),
			5 => Some(Self::Medic),
			6 => Some(Self::Heavy),
			7 => Some(Self::Pyro),
			8 => Some(Self::Spy),
			9 => Some(Self::Engineer),
			_ => None,
		}
	}

	/// The class's directory under `scenes/Player/`, as TF2's response rules
	/// spell it, such as `Heavy`.
	pub const fn scene_directory(self) -> &'static str {
		match self {
			Self::Scout => "Scout",
			Self::Sniper => "Sniper",
			Self::Soldier => "Soldier",
			Self::Demoman => "Demoman",
			Self::Medic => "Medic",
			Self::Heavy => "Heavy",
			Self::Pyro => "Pyro",
			Self::Spy => "Spy",
			Self::Engineer => "Engineer",
		}
	}

	/// The class's `TF_CLASS_*` number.
	pub const fn to_raw(self) -> c_int {
		self as c_int
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn classes_round_trip_through_native_numbers() {
		for (raw, class) in (1..).zip(PlayerClass::ALL) {
			assert_eq!(class.to_raw(), raw);
			assert_eq!(PlayerClass::from_raw(raw), Some(class));
		}

		for raw in [-1, 0, 10, 11] {
			assert_eq!(PlayerClass::from_raw(raw), None);
		}

		assert_eq!(PlayerClass::Heavy.scene_directory(), "Heavy");
		assert_eq!(PlayerClass::Demoman.scene_directory(), "Demoman");
	}
}
