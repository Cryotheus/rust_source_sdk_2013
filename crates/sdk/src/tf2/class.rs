//! TF2's playable classes.

use sdk_raw::tf2::class::{
	TF_CLASS_DEMOMAN, TF_CLASS_ENGINEER, TF_CLASS_HEAVYWEAPONS, TF_CLASS_MEDIC, TF_CLASS_PYRO,
	TF_CLASS_SCOUT, TF_CLASS_SNIPER, TF_CLASS_SOLDIER, TF_CLASS_SPY,
};

use std::ffi::c_int;
use std::str::FromStr;

/// One of TF2's nine playable classes, numbered as the `TF_CLASS_*` constants
/// in `game/shared/tf/tf_shareddefs.h`.
///
/// The Civilian (`TF_CLASS_CIVILIAN`, 10), which players cannot choose and
/// which has no voice lines, is left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PlayerClass {
	/// `TF_CLASS_SCOUT`: 1.
	#[doc(alias("TF_CLASS_SCOUT"))]
	Scout = TF_CLASS_SCOUT as isize,

	/// `TF_CLASS_SNIPER`: 2.
	#[doc(alias("TF_CLASS_SNIPER"))]
	Sniper = TF_CLASS_SNIPER as isize,

	/// `TF_CLASS_SOLDIER`: 3.
	#[doc(alias("TF_CLASS_SOLDIER"))]
	Soldier = TF_CLASS_SOLDIER as isize,

	/// `TF_CLASS_DEMOMAN`: 4.
	#[doc(alias("TF_CLASS_DEMOMAN"))]
	Demoman = TF_CLASS_DEMOMAN as isize,

	/// `TF_CLASS_MEDIC`: 5.
	#[doc(alias("TF_CLASS_MEDIC"))]
	Medic = TF_CLASS_MEDIC as isize,

	/// `TF_CLASS_HEAVYWEAPONS`: 6.
	#[doc(alias("TF_CLASS_HEAVYWEAPONS"))]
	Heavy = TF_CLASS_HEAVYWEAPONS as isize,

	/// `TF_CLASS_PYRO`: 7.
	#[doc(alias("TF_CLASS_PYRO"))]
	Pyro = TF_CLASS_PYRO as isize,

	/// `TF_CLASS_SPY`: 8.
	#[doc(alias("TF_CLASS_SPY"))]
	Spy = TF_CLASS_SPY as isize,

	/// `TF_CLASS_ENGINEER`: 9.
	#[doc(alias("TF_CLASS_ENGINEER"))]
	Engineer = TF_CLASS_ENGINEER as isize,
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
			TF_CLASS_SCOUT => Some(Self::Scout),
			TF_CLASS_SNIPER => Some(Self::Sniper),
			TF_CLASS_SOLDIER => Some(Self::Soldier),
			TF_CLASS_DEMOMAN => Some(Self::Demoman),
			TF_CLASS_MEDIC => Some(Self::Medic),
			TF_CLASS_HEAVYWEAPONS => Some(Self::Heavy),
			TF_CLASS_PYRO => Some(Self::Pyro),
			TF_CLASS_SPY => Some(Self::Spy),
			TF_CLASS_ENGINEER => Some(Self::Engineer),
			_ => None,
		}
	}

	/// The class named `name`, compared ignoring ASCII case, or `None` for any
	/// other name: the name `joinclass` takes, which [`Self::name`] gives, such
	/// as `heavyweapons`, or the short name TF2 also uses, such as `heavy` or
	/// `demo`, as `TF2_GetClass` and TF2's class icons do.
	///
	/// ```
	/// use source_sdk_2013::tf2::PlayerClass;
	///
	/// assert_eq!(PlayerClass::from_name("Heavy"), Some(PlayerClass::Heavy));
	/// assert_eq!(PlayerClass::from_name("heavyweapons"), Some(PlayerClass::Heavy));
	/// assert_eq!(PlayerClass::from_name("civilian"), None);
	/// ```
	#[doc(alias("TF2_GetClass", "joinclass"))]
	pub fn from_name(name: &str) -> Option<Self> {
		Self::ALL.into_iter().find(|class| {
			name.eq_ignore_ascii_case(class.name()) || name.eq_ignore_ascii_case(class.short_name())
		})
	}

	/// The class's name as TF2's class data and the `joinclass` command spell
	/// it, such as `heavyweapons` (`g_aRawPlayerClassNames`).
	#[doc(alias("g_aRawPlayerClassNames"))]
	pub const fn name(self) -> &'static str {
		match self {
			Self::Scout => "scout",
			Self::Sniper => "sniper",
			Self::Soldier => "soldier",
			Self::Demoman => "demoman",
			Self::Medic => "medic",
			Self::Heavy => "heavyweapons",
			Self::Pyro => "pyro",
			Self::Spy => "spy",
			Self::Engineer => "engineer",
		}
	}

	/// The class's short name, as TF2 names its class icons and default custom
	/// models, such as `heavy` (`g_aRawPlayerClassNamesShort`).
	#[doc(alias("g_aRawPlayerClassNamesShort"))]
	pub const fn short_name(self) -> &'static str {
		match self {
			Self::Demoman => "demo",
			Self::Heavy => "heavy",
			class => class.name(),
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

impl FromStr for PlayerClass {
	type Err = UnknownClassName;

	/// The class [`PlayerClass::from_name`] finds.
	fn from_str(name: &str) -> Result<Self, Self::Err> {
		Self::from_name(name).ok_or_else(|| UnknownClassName(name.to_owned()))
	}
}

/// A name of none of TF2's playable classes, which parsing a [`PlayerClass`]
/// refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} names none of TF2's playable classes")]
pub struct UnknownClassName(pub String);
