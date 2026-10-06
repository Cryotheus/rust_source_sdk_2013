//! The teams of TF2's entities, and where to intercept their changes.
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

use crate::{Game, Server};
use sdk_raw::tf2::teams::TeamVtables;
use sdk_raw::util;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The primary vtable of a C++ class in this server's game module. For an
/// entity class, the game calls `CBaseEntity::ChangeTeam` through it on the
/// class's entities, but not on those of the classes deriving from it.
///
/// The search finds any polymorphic class by name: hooking it as an entity
/// class is only sound for one deriving from `CBaseEntity`.
#[derive(Debug, Clone, Copy)]
pub struct TeamTarget<'s> {
	vtable: NonNull<*mut c_void>,
	_scope: PhantomData<&'s Server<'s>>,
}

impl TeamTarget<'_> {
	/// The vtable's address in the game module.
	pub const fn as_ptr(self) -> NonNull<*mut c_void> {
		self.vtable
	}
}

/// Why the game module could not be searched for entity classes.
#[derive(Debug, thiserror::Error)]
pub enum TeamTargetError {
	/// The server does not run Team Fortress 2.
	#[error("team targets require Team Fortress 2")]
	WrongGame,

	/// The game module could not be read.
	#[error("the game module could not be inspected")]
	Image(#[from] std::io::Error),

	/// The game module is not an executable image the vtable search supports.
	#[error("the game module has an unsupported executable image")]
	InvalidImage,
}

impl From<util::Error> for TeamTargetError {
	fn from(error: util::Error) -> Self {
		match error {
			util::Error::InvalidImage => Self::InvalidImage,
			util::Error::Io(error) => Self::Image(error),
		}
	}
}

/// A snapshot of TF2's game module, in which to find the vtables of its
/// entity classes. Snapshotting reads the whole module, so find every class
/// needed with one.
#[derive(Debug, Clone)]
pub struct TeamTargets<'s> {
	vtables: TeamVtables,
	_scope: PhantomData<&'s Server<'s>>,
}

impl<'s> TeamTargets<'s> {
	/// Snapshots the server's game module.
	pub fn load(server: Server<'s>) -> Result<Self, TeamTargetError> {
		if server.game() != Game::TeamFortress2 {
			return Err(TeamTargetError::WrongGame);
		}

		// SAFETY: The game server factory is the game module's `CreateInterface`,
		// and the Server's callback scope keeps the module loaded while its sections
		// are inspected (`Server::new` condition 1).
		let vtables = unsafe { TeamVtables::load(server.game_server_factory().as_raw()) }?;

		Ok(Self {
			vtables,
			_scope: PhantomData,
		})
	}

	/// The vtable of the global C++ class named `class`, such as
	/// `CFuncRespawnRoom` for `func_respawnroom`, from its run-time type
	/// information. Returns `None` if the module has no such class, or more
	/// than one. Whether the class is an entity class is the caller's to know.
	///
	/// Each search reads the whole snapshot a few times, so find the classes
	/// needed together with [`Self::find_all`].
	pub fn find(&self, class: &str) -> Option<TeamTarget<'s>> {
		self.vtables.find(class).map(|vtable| TeamTarget {
			vtable,
			_scope: PhantomData,
		})
	}

	/// The vtables [`Self::find`] finds for each of `classes`, in their order.
	/// The snapshot is read as often for every class as [`Self::find`] reads
	/// it for one.
	pub fn find_all(&self, classes: &[&str]) -> Vec<Option<TeamTarget<'s>>> {
		self.vtables
			.find_all(classes)
			.into_iter()
			.map(|vtable| {
				vtable.map(|vtable| TeamTarget {
					vtable,
					_scope: PhantomData,
				})
			})
			.collect()
	}
}
