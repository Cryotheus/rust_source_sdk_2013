//! TF2's spectating: how dead players and spectators watch the game, and
//! letting a player on a team roam freely, as spectators do.
//!
//! A spectating player watches in an [`ObserverMode`], most of them following
//! a target. Clients cycle through the modes with the jump key (`spec_mode`),
//! and through the targets with the attack keys (`spec_next`, `spec_prev`).
//!
//! TF2 holds dead players on a team to more than spectators:
//!
//! - `CTFPlayer::SetObserverMode` turns [`ObserverMode::Roaming`] into
//!   [`ObserverMode::InEye`] for them, whatever `mp_forcecamera` allows, so
//!   only spectators roam.
//! - `mp_forcecamera` decides whom they may follow: with `1`, the default,
//!   only their own team (`CBasePlayer::IsValidObserverTarget`), and with
//!   `0`, anyone a spectator may. Clients read it too, to show the key that
//!   switches modes.
//! - `mp_fadetoblack` blacks their screens out, and keeps them in
//!   [`ObserverMode::Chase`].
//!
//! [`PlayerObserver::roam`] lets such a player roam anyway, as the game lets a
//! spectator. `metamod_source`'s `observer_hooks` call it for the players a
//! plugin chooses, each time the game turns their roaming into first person
//! or a map-camera chase view.
//!
//! [`sdk_raw::tf2::observer`] holds the observer modes' values, and the vtable
//! slots of the player's observer methods.
//!
//! The game declares `m_bForcedObserverMode` in `CBasePlayer`'s datamap.
//! Clearing it avoids `CheckObserverSettings` restoring a forced chase view
//! after `ValidateCurrentObserverTarget` rejected a map camera in first person.

#[cfg(test)]
#[path = "../tests/tf2/observer.rs"]
mod tests;

use crate::datatables::{NetProp, NetPropError};
use crate::entities::Entity;
use crate::entities::fields::FieldError;
use crate::{Game, InterfaceError, Server};
use sdk_raw::tf2::observer as raw;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};

/// The networked components of a player's view offset, which roaming zeroes as
/// the game does (`CTFPlayer::SetObserverMode`). In first person, the game
/// copies the target's (`CBasePlayer::CheckObserverSettings`).
pub(crate) const VIEW_OFFSET: [&CStr; 3] = [
	c"m_vecViewOffset[0]",
	c"m_vecViewOffset[1]",
	c"m_vecViewOffset[2]",
];

/// Why an observer operation failed.
#[derive(Debug, thiserror::Error)]
pub enum ObserverError {
	/// A datamap variable could not be read or written.
	#[error(transparent)]
	Field(#[from] FieldError),

	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The player is already marked for deletion.
	#[error("the player is marked for deletion")]
	MarkedForDeletion,

	/// A networked variable could not be read or written.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// The player watches in a mode that cannot turn into roaming: not
	/// spectating at all, or in the death or freeze cam that follows a death.
	#[error("the player is not spectating in a mode that can roam ({0:?})")]
	NotObserving(ObserverMode),

	/// The entity is not a TF2 player, or the server does not run TF2.
	#[error("observer operations require a TF2 player")]
	NotTfPlayer,

	/// The player's `m_iObserverMode` is not one of TF2's observer modes.
	#[error("the player's observer mode {0} is not one of TF2's")]
	UnknownMode(c_int),
}

/// How a player watches the game while spectating (`m_iObserverMode`), as TF2
/// numbers the modes (`OBS_MODE_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObserverMode {
	/// Follows the target in third person.
	#[doc(alias("OBS_MODE_CHASE"))]
	Chase,

	/// The death cam, which turns a player who just died towards their
	/// killer.
	#[doc(alias("OBS_MODE_DEATHCAM"))]
	DeathCam,

	/// Views from a fixed position, such as an `info_observer_point`.
	#[doc(alias("OBS_MODE_FIXED"))]
	Fixed,

	/// The freeze cam, which zooms in on a dead player's killer and freezes
	/// the frame.
	#[doc(alias("OBS_MODE_FREEZECAM"))]
	FreezeCam,

	/// Follows the target in first person.
	#[doc(alias("OBS_MODE_IN_EYE"))]
	InEye,

	/// Not spectating.
	#[doc(alias("OBS_MODE_NONE"))]
	None,

	/// Follows PASS Time's point of interest, such as its ball. Outside PASS
	/// Time, the game turns it into [`Self::Roaming`].
	#[doc(alias("OBS_MODE_POI"))]
	PointOfInterest,

	/// Roams freely.
	#[doc(alias("OBS_MODE_ROAMING"))]
	Roaming,
}

impl ObserverMode {
	/// The mode TF2 numbers `raw`, or `None` if it numbers none so.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		Some(match raw {
			raw::OBS_MODE_NONE => Self::None,
			raw::OBS_MODE_DEATHCAM => Self::DeathCam,
			raw::OBS_MODE_FREEZECAM => Self::FreezeCam,
			raw::OBS_MODE_FIXED => Self::Fixed,
			raw::OBS_MODE_IN_EYE => Self::InEye,
			raw::OBS_MODE_CHASE => Self::Chase,
			raw::OBS_MODE_POI => Self::PointOfInterest,
			raw::OBS_MODE_ROAMING => Self::Roaming,
			_ => return None,
		})
	}

	/// TF2's number for the mode.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::None => raw::OBS_MODE_NONE,
			Self::DeathCam => raw::OBS_MODE_DEATHCAM,
			Self::FreezeCam => raw::OBS_MODE_FREEZECAM,
			Self::Fixed => raw::OBS_MODE_FIXED,
			Self::InEye => raw::OBS_MODE_IN_EYE,
			Self::Chase => raw::OBS_MODE_CHASE,
			Self::PointOfInterest => raw::OBS_MODE_POI,
			Self::Roaming => raw::OBS_MODE_ROAMING,
		}
	}
}

/// A TF2 player's spectating, scoped to one engine callback.
#[derive(Debug, Clone, Copy)]
pub struct PlayerObserver<'s> {
	server: Server<'s>,
	player: Entity<'s>,
}

impl<'s> PlayerObserver<'s> {
	/// Wraps a player's spectating, or returns [`ObserverError::NotTfPlayer`]
	/// unless the server runs TF2 and `player`'s datamaps include `CTFPlayer`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, ObserverError> {
		if server.game() != Game::TeamFortress2 || !player.has_data_map_class(c"CTFPlayer") {
			return Err(ObserverError::NotTfPlayer);
		}

		Ok(Self { server, player })
	}

	/// How the player watches the game (`m_iObserverMode`).
	#[doc(alias("m_iObserverMode", "GetObserverMode"))]
	pub fn mode(self) -> Result<ObserverMode, ObserverError> {
		let raw = self
			.net_prop(c"m_iObserverMode")?
			.get::<c_int>(self.player)?;

		ObserverMode::from_raw(raw).ok_or(ObserverError::UnknownMode(raw))
	}

	/// Resolves one of the player's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, ObserverError> {
		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.player, name)?)
	}

	/// The player.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// Lets the player roam freely, as the game lets spectators: switches a
	/// player who spectates in any mode following a target, or from a fixed
	/// position, to [`ObserverMode::Roaming`], and moves them behind their
	/// target, if any, as the game does (`CBasePlayer::SetObserverTarget`).
	/// A player already roaming keeps their position and view offset.
	/// The game's forced-mode flag (`m_bForcedObserverMode`) is cleared so
	/// its next observer-settings check
	/// does not return the player to the mode forced by a map camera.
	///
	/// This is meant for players on a team, whom the game itself never lets
	/// roam. They roam until the game changes their mode again, as it does
	/// when they cycle through the modes or respawn. While they roam, the game
	/// leaves their view alone, and lets them cycle to their next mode.
	///
	/// Fails with [`ObserverError::NotObserving`] if the player is not
	/// spectating, or still watches the death or freeze cam that follows a
	/// death, and with [`ObserverError::MarkedForDeletion`] for a player
	/// marked for deletion. Nothing is changed unless every networked
	/// variable it writes resolves and the datamap declares the forced-mode
	/// flag as a boolean.
	pub fn roam(self) -> Result<(), ObserverError> {
		if self.player.is_marked_for_deletion() {
			return Err(ObserverError::MarkedForDeletion);
		}

		match self.mode()? {
			ObserverMode::Roaming => {
				if self.player.data_field::<bool>(c"m_bForcedObserverMode")? {
					self.player.set_data_field(
						self.server.valve_engine()?,
						c"m_bForcedObserverMode",
						false,
					)?;
				}
				return Ok(());
			}

			ObserverMode::Fixed
			| ObserverMode::InEye
			| ObserverMode::Chase
			| ObserverMode::PointOfInterest => {}

			mode => return Err(ObserverError::NotObserving(mode)),
		}

		let engine = self.server.valve_engine()?;
		let mode = self.net_prop(c"m_iObserverMode")?;
		let [x, y, z] = VIEW_OFFSET;
		let view_offset = [self.net_prop(x)?, self.net_prop(y)?, self.net_prop(z)?];
		let target = self.target()?;
		// Validate the server-only field before changing the player's view.
		self.player.data_field::<bool>(c"m_bForcedObserverMode")?;
		self.player
			.set_data_field(engine, c"m_bForcedObserverMode", false)?;

		// SAFETY: The game assigns the mode itself, as `CTFPlayer::SetObserverMode`
		// does for spectators.
		unsafe { mode.set(engine, self.player, raw::OBS_MODE_ROAMING) }?;

		for component in view_offset {
			// SAFETY: The game zeroes the view offset itself as a player starts
			// roaming.
			unsafe { component.set(engine, self.player, 0.0f32) }?;
		}

		if let Some(target) = target {
			let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

			// SAFETY: `new` found `CTFPlayer` in the player's datamaps, so it is
			// one, whose entity base `sdk_raw::tf2` asserts is at offset zero.
			// The player and the target are live during `'s`, on the main
			// thread. `SetObserverTarget` checks the target, then, as the player
			// roams, moves them behind it, which deletes nothing.
			unsafe {
				vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_SetObserverTarget(
					target.as_ptr(),
				));
			}
		}

		Ok(())
	}

	/// The entity the player follows (`m_hObserverTarget`), or `None` if they
	/// follow nothing, or it no longer exists.
	#[doc(alias("m_hObserverTarget", "GetObserverTarget"))]
	pub fn target(self) -> Result<Option<Entity<'s>>, ObserverError> {
		let handle = self
			.net_prop(c"m_hObserverTarget")?
			.get_handle(self.player)?;

		Ok(self.server.server_tools()?.entity_by_handle(handle))
	}
}
