//! TF2's buildings: the sentry guns, dispensers and teleporters Engineers
//! build, and where to intercept where they may be built.
//!
//! While an Engineer holds a blueprint, the game checks each frame whether the
//! building may be built where the blueprint is, through
//! `CBaseObject::IsPlacementPosValid`, which it calls through the building's
//! vtable. Among other things, the check refuses respawn rooms, `func_nobuild`
//! brushes, places that hurt, and blueprints across a respawn room
//! visualizer. Clients only show the result: a blueprint the server refuses
//! cannot be built.
//!
//! [`sdk_raw::tf2::objects`] holds the function's signature and its vtable
//! slot. `metamod_source`'s `placement_hooks` hook it in each class
//! [`object_vtables`] finds.

use crate::{Game, Server};
use sdk_raw::tf2::objects::ObjectVtables;
use sdk_raw::util;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// Why the buildings' placement checks cannot all be hooked.
#[derive(Debug, thiserror::Error)]
pub enum ObjectHookTargetError {
	/// The server does not run Team Fortress 2.
	#[error("building hooks require Team Fortress 2")]
	WrongGame,

	/// The game module could not be read.
	#[error("the game module could not be inspected")]
	Image(#[from] std::io::Error),

	/// The game module is not an executable image the vtable search supports.
	#[error("the game module has an unsupported executable image")]
	InvalidImage,

	/// The building's class has no unique primary vtable in the game module.
	#[error("no unique primary vtable found for TF2 building {0:?}")]
	UnsupportedObject(ObjectKind),
}

impl From<util::Error> for ObjectHookTargetError {
	fn from(error: util::Error) -> Self {
		match error {
			util::Error::InvalidImage => Self::InvalidImage,
			util::Error::Io(error) => Self::Image(error),
		}
	}
}

/// The buildings Engineers place from a blueprint.
#[doc(alias("CBaseObject"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
	/// A dispenser.
	#[doc(alias("CObjectDispenser", "obj_dispenser"))]
	Dispenser,

	/// A sentry gun, mini-sentries included.
	#[doc(alias("CObjectSentrygun", "obj_sentrygun"))]
	Sentry,

	/// A teleporter entrance or exit.
	#[doc(alias("CObjectTeleporter", "obj_teleporter"))]
	Teleporter,
}

impl ObjectKind {
	/// Every building placed from a blueprint.
	pub const ALL: [Self; 3] = [Self::Dispenser, Self::Sentry, Self::Teleporter];

	/// The building's undecorated C++ class name, used to find its RTTI.
	const fn class(self) -> &'static str {
		match self {
			Self::Dispenser => "CObjectDispenser",
			Self::Sentry => "CObjectSentrygun",
			Self::Teleporter => "CObjectTeleporter",
		}
	}
}

/// A building class's primary vtable in this server's game module. Intended
/// for the Metamod adapter; no building is retained.
#[derive(Debug, Clone, Copy)]
pub struct ObjectVtable<'s> {
	/// The building whose class owns this vtable.
	pub kind: ObjectKind,
	vtable: NonNull<*mut c_void>,
	_scope: PhantomData<&'s Server<'s>>,
}

impl ObjectVtable<'_> {
	/// The vtable's address in the game module.
	pub const fn as_ptr(self) -> NonNull<*mut c_void> {
		self.vtable
	}
}

/// Finds every building class placed from a blueprint before any hooks are
/// installed. Missing or ambiguous RTTI is an error; some buildings are never
/// silently left out. Call once during plugin load: it reads the whole game
/// module.
pub fn object_vtables(server: Server<'_>) -> Result<Vec<ObjectVtable<'_>>, ObjectHookTargetError> {
	if server.game() != Game::TeamFortress2 {
		return Err(ObjectHookTargetError::WrongGame);
	}

	// SAFETY: The game server factory is the game module's `CreateInterface`,
	// and the Server's callback scope keeps the module loaded while its sections
	// are inspected (`Server::new` condition 1).
	let vtables = unsafe { ObjectVtables::load(server.game_server_factory().as_raw()) }?;

	ObjectKind::ALL
		.into_iter()
		.map(|kind| {
			let pointer = vtables
				.find(kind.class())
				.ok_or(ObjectHookTargetError::UnsupportedObject(kind))?;

			Ok(ObjectVtable {
				kind,
				vtable: pointer,
				_scope: PhantomData,
			})
		})
		.collect()
}
