//! The teams of TF2's entities, and where to intercept their changes.
//!
//! [`Team`] names the teams as an entity's `m_iTeamNum` numbers them.
//! [`TfPlayer::force_change_team`](crate::tf2::player::TfPlayer::force_change_team)
//! moves a player to one as the game does.
//!
//! The game changes an entity's team through `CBaseEntity::ChangeTeam`, which
//! it calls through the entity's vtable. Some classes act on their team, such
//! as a respawn room (`CFuncRespawnRoom`), which only counts for its team's
//! players, or a resupply cabinet (`CRegenerateZone`), which only resupplies
//! them.
//!
//! [`sdk_raw::tf2::teams`] holds the function's signature and its vtable slot.
//! [`TeamTargets`] finds the vtable of an entity class by its C++ name, which
//! `metamod_source`'s `team_hooks` hook, to decide each change of the class's
//! entities' teams, before any entity of the class exists.

use super::class_targets::{ClassTargetError, ClassVtable, SlotTargets};
use crate::tf2::scoreboard::ScoringTeam;
use sdk_raw::players::{TEAM_SPECTATOR, TEAM_UNASSIGNED};
use sdk_raw::tf2::scoreboard::{TF_TEAM_BLUE, TF_TEAM_RED};
use sdk_raw::tf2::teams::CHANGE_TEAM_SLOT;
use std::ffi::c_int;

/// One of TF2's teams, numbered as an entity's `m_iTeamNum` holds them
/// (`game/shared/shareddefs.h` and `game/shared/tf/tf_shareddefs.h`).
///
/// [`ScoringTeam`] names the two playing teams alone, as the scoreboard
/// does, and converts to and from this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Team {
	/// No team, as players are before they first join one, and as most of
	/// the map's entities are (`TEAM_UNASSIGNED`).
	#[doc(alias("TEAM_UNASSIGNED"))]
	Unassigned,

	/// The spectators (`TEAM_SPECTATOR`).
	#[doc(alias("TEAM_SPECTATOR"))]
	Spectator,

	/// RED (`TF_TEAM_RED`).
	#[doc(alias("TF_TEAM_RED"))]
	Red,

	/// BLU (`TF_TEAM_BLUE`).
	#[doc(alias("TF_TEAM_BLUE"))]
	Blue,
}

impl Team {
	/// Every team, in native order.
	pub const ALL: [Self; 4] = [Self::Unassigned, Self::Spectator, Self::Red, Self::Blue];

	/// The teams that play: RED and BLU.
	pub const PLAYING: [Self; 2] = [Self::Red, Self::Blue];

	/// The team with this team number, or `None` for any other number, such
	/// as `TF_TEAM_PVE_INVADERS_GIANTS` or a negative one.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			TEAM_UNASSIGNED => Some(Self::Unassigned),
			TEAM_SPECTATOR => Some(Self::Spectator),
			TF_TEAM_RED => Some(Self::Red),
			TF_TEAM_BLUE => Some(Self::Blue),
			_ => None,
		}
	}

	/// Whether this is RED or BLU.
	pub const fn is_playing(self) -> bool {
		matches!(self, Self::Red | Self::Blue)
	}

	/// The other playing team: BLU for RED, RED for BLU, and `None` for the
	/// others.
	pub const fn opponent(self) -> Option<Self> {
		match self {
			Self::Red => Some(Self::Blue),
			Self::Blue => Some(Self::Red),
			Self::Unassigned | Self::Spectator => None,
		}
	}

	/// This team as a [`ScoringTeam`], or `None` unless it plays.
	pub const fn scoring(self) -> Option<ScoringTeam> {
		match self {
			Self::Red => Some(ScoringTeam::Red),
			Self::Blue => Some(ScoringTeam::Blue),
			Self::Unassigned | Self::Spectator => None,
		}
	}

	/// The team's number, as `m_iTeamNum` holds it.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Unassigned => TEAM_UNASSIGNED,
			Self::Spectator => TEAM_SPECTATOR,
			Self::Red => TF_TEAM_RED,
			Self::Blue => TF_TEAM_BLUE,
		}
	}
}

impl From<ScoringTeam> for Team {
	fn from(team: ScoringTeam) -> Self {
		match team {
			ScoringTeam::Red => Self::Red,
			ScoringTeam::Blue => Self::Blue,
		}
	}
}

/// The primary vtable of a C++ class in this server's game module, as
/// [`TeamTargets`] finds it. For an entity class, the game calls
/// `CBaseEntity::ChangeTeam` through it on the class's entities, but not on
/// those of the classes deriving from it.
///
/// The search finds any polymorphic class by name: hooking it as an entity
/// class is only sound for one deriving from `CBaseEntity`.
pub type TeamTarget<'s> = ClassVtable<'s>;

/// Why the game module could not be searched for entity classes.
pub type TeamTargetError = ClassTargetError;

/// A snapshot of TF2's game module, in which to find the vtables of its
/// entity classes, such as `CFuncRespawnRoom` for `func_respawnroom`: those
/// holding code at [`CHANGE_TEAM_SLOT`]. Snapshotting reads the whole module,
/// so find every class needed with one, or search a
/// [`ClassTargets`](super::class_targets::ClassTargets) snapshot taken for
/// other classes, through `From`.
pub type TeamTargets<'s> = SlotTargets<'s, CHANGE_TEAM_SLOT>;
