//! Entities touching one another, and where to intercept it.
//!
//! The engine tells an entity of each entity touching it through
//! `CBaseEntity::Touch`, which it calls through the touched entity's vtable at
//! every tick they touch. The function runs the entity's touch function, such
//! as an item's `ItemTouch` or a dropped ammo pack's `PackTouch`, unless its
//! class overrides it, as `func_regenerate`'s `CRegenerateZone` does.
//!
//! [`sdk_raw::tf2::touch`] holds the function's signature and its vtable slot,
//! [`TOUCH_SLOT`](sdk_raw::tf2::touch::TOUCH_SLOT). [`TouchTargets`] finds the
//! vtable of an entity class by its C++ name, which `metamod_source`'s
//! `touch_hooks` hook, to see each touch of the class's entities before and
//! after the game, and to block it.

use crate::{Game, Server};
use sdk_raw::tf2::touch::TouchVtables;
use sdk_raw::util;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The primary vtable of an entity class in this server's game module,
/// through which the game calls `CBaseEntity::Touch` on the class's entities,
/// but not on those of the classes deriving from it.
#[derive(Debug, Clone, Copy)]
pub struct TouchTarget<'s> {
	vtable: NonNull<*mut c_void>,
	_scope: PhantomData<&'s Server<'s>>,
}

impl TouchTarget<'_> {
	/// The vtable's address in the game module.
	pub const fn as_ptr(self) -> NonNull<*mut c_void> {
		self.vtable
	}
}

/// Why the game module could not be searched for entity classes.
#[derive(Debug, thiserror::Error)]
pub enum TouchTargetError {
	/// The server does not run Team Fortress 2.
	#[error("touch targets require Team Fortress 2")]
	WrongGame,

	/// The game module could not be read.
	#[error("the game module could not be inspected")]
	Image(#[from] std::io::Error),

	/// The game module is not an executable image the vtable search supports.
	#[error("the game module has an unsupported executable image")]
	InvalidImage,
}

impl From<util::Error> for TouchTargetError {
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
pub struct TouchTargets<'s> {
	vtables: TouchVtables,
	_scope: PhantomData<&'s Server<'s>>,
}

impl<'s> TouchTargets<'s> {
	/// Snapshots the server's game module.
	pub fn load(server: Server<'s>) -> Result<Self, TouchTargetError> {
		if server.game() != Game::TeamFortress2 {
			return Err(TouchTargetError::WrongGame);
		}

		// SAFETY: The game server factory is the game module's `CreateInterface`,
		// and the Server's callback scope keeps the module loaded while its sections
		// are inspected (`Server::new` condition 1).
		let vtables = unsafe { TouchVtables::load(server.game_server_factory().as_raw()) }?;

		Ok(Self {
			vtables,
			_scope: PhantomData,
		})
	}

	/// The vtable of the global C++ entity class named `class`, such as
	/// `CTFAmmoPack` for `tf_ammo_pack`, from its run-time type information.
	/// Returns `None` if the module has no such class, or more than one.
	pub fn find(&self, class: &str) -> Option<TouchTarget<'s>> {
		self.vtables.find(class).map(|vtable| TouchTarget {
			vtable,
			_scope: PhantomData,
		})
	}
}
