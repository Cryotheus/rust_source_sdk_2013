//! TF2's airblast: the compression blast of the Pyro's flame throwers, which
//! reflects projectiles and pushes players, and where to intercept its pushes.
//!
//! A flame thrower's airblast goes through the players and projectiles in
//! front of its owner, and tells the weapon of each player it reaches through
//! `CTFWeaponBase::DeflectPlayer`, which the game calls through the weapon's
//! vtable. The flame throwers' function, which the Dragon's Fury keeps,
//! extinguishes a burning teammate, and pushes a player of the other team
//! away: it knocks them into the air, slows them for a moment, stops a
//! charging Demoman's charge, and credits the Pyro if the player then falls to
//! their death. The weapon reflects projectiles through another function,
//! `DeflectEntity`.
//!
//! [`sdk_raw::tf2::airblast`] holds the function's signature and vtable slot,
//! and `metamod_source`'s `airblast_hooks` hook it in the classes
//! [`airblast_vtables`] finds.

use crate::{Game, Server};
use sdk_raw::tf2::airblast::{REFERENCE_CLASS, find_airblast_vtables};
use sdk_raw::util;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// Why the vtables of the weapon classes with airblast could not be found.
#[derive(Debug, thiserror::Error)]
pub enum AirblastVtableError {
	/// The server does not run Team Fortress 2.
	#[error("airblast hooks require Team Fortress 2")]
	WrongGame,

	/// The game module could not be read.
	#[error("the game module could not be inspected")]
	Image(#[from] std::io::Error),

	/// The game module is not an executable image the vtable search supports.
	#[error("the game module has an unsupported executable image")]
	InvalidImage,

	/// The class named so has no unique primary vtable in the game module.
	#[error("no unique primary vtable found for TF2's weapon class `{0}`")]
	NotFound(&'static str),
}

impl From<util::Error> for AirblastVtableError {
	fn from(error: util::Error) -> Self {
		match error {
			util::Error::InvalidImage => Self::InvalidImage,
			util::Error::Io(error) => Self::Image(error),
		}
	}
}

/// The primary vtables in this server's game module of the weapon classes
/// whose airblast pushes players, and of a weapon class without airblast to
/// compare them with. Intended for the Metamod adapter; no weapon is
/// retained.
#[derive(Debug, Clone, Copy)]
pub struct AirblastVtables<'s> {
	weapons: [NonNull<*mut c_void>; 2],
	reference: NonNull<*mut c_void>,
	_scope: PhantomData<&'s Server<'s>>,
}

impl AirblastVtables<'_> {
	/// The vtable of the rocket launchers' class, `CTFRocketLauncher`, which
	/// keeps `CTFWeaponBase`'s `DeflectProjectiles` and `DeflectPlayer`: the
	/// first, as the weapons with airblast do, and the second, which they
	/// override.
	pub const fn reference(self) -> NonNull<*mut c_void> {
		self.reference
	}

	/// The vtables of the classes of [`AirblastWeapon::ALL`], in its order.
	pub const fn weapons(self) -> [NonNull<*mut c_void>; 2] {
		self.weapons
	}
}

/// The weapons whose airblast pushes players.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AirblastWeapon {
	/// The Pyro's flame throwers: the stock one, and every other but the
	/// Dragon's Fury, such as the Backburner and the Degreaser. The
	/// Phlogistinator is one too, without airblast.
	#[doc(alias("CTFFlameThrower", "tf_weapon_flamethrower"))]
	FlameThrower,

	/// The Dragon's Fury, whose class derives from the flame throwers' and keeps
	/// their push.
	#[doc(alias("CTFWeaponFlameBall", "tf_weapon_rocketlauncher_fireball"))]
	DragonsFury,
}

impl AirblastWeapon {
	/// Both weapons, in the order [`AirblastVtables::weapons`] gives their
	/// vtables.
	pub const ALL: [Self; 2] = [Self::FlameThrower, Self::DragonsFury];

	/// The weapon's undecorated C++ class name, used to find its RTTI.
	pub const fn class(self) -> &'static str {
		match self {
			Self::FlameThrower => sdk_raw::tf2::airblast::FLAME_THROWER_CLASS,
			Self::DragonsFury => sdk_raw::tf2::airblast::DRAGONS_FURY_CLASS,
		}
	}
}

/// Finds the vtables of the weapon classes whose airblast pushes players, and
/// of the rocket launchers' class to compare them with, through the game
/// server module's run-time type information, which needs no weapon. A class
/// without a unique vtable is an error. Snapshots the whole module and
/// searches it once for all three, so call it once, such as while loading.
///
/// The search only checks that each vtable reaches
/// [`DEFLECT_PLAYER_SLOT`](sdk_raw::tf2::airblast::DEFLECT_PLAYER_SLOT) and
/// holds code there, not that the class is the weapon it is named after.
pub fn airblast_vtables(server: Server<'_>) -> Result<AirblastVtables<'_>, AirblastVtableError> {
	if server.game() != Game::TeamFortress2 {
		return Err(AirblastVtableError::WrongGame);
	}

	// SAFETY: The game server factory is the game module's `CreateInterface`,
	// and the Server's callback scope keeps the module loaded while its sections
	// are inspected (`Server::new` condition 1).
	let [flame_thrower, dragons_fury, reference] =
		unsafe { find_airblast_vtables(server.game_server_factory().as_raw()) }?;

	let found = |vtable: Option<NonNull<*mut c_void>>, class| {
		vtable.ok_or(AirblastVtableError::NotFound(class))
	};

	Ok(AirblastVtables {
		weapons: [
			found(flame_thrower, AirblastWeapon::FlameThrower.class())?,
			found(dragons_fury, AirblastWeapon::DragonsFury.class())?,
		],
		reference: found(reference, REFERENCE_CLASS)?,
		_scope: PhantomData,
	})
}
