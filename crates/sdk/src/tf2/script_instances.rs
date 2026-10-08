//! The script instances through which TF2's native methods take and return
//! entities.
//!
//! VScript refers to an entity by its script instance (`HSCRIPT`), which
//! `CBaseEntity::GetScriptInstance` registers with the script VM the first
//! time anything asks for it, and keeps in `m_hScriptInstance` until the
//! entity's `UpdateOnRemove` removes it (`game/server/baseentity.cpp`). The
//! native methods behind VScript's functions take entities as such instances,
//! such as `AddCondEx`'s provider or `StunPlayer`'s attacker, and return them
//! so, such as `GetHealTarget`.
//!
//! [`ScriptInstance::of`] gets an entity's instance, through the
//! `ValidateScriptScope` binding when it has none yet, as a script touching
//! the entity would. [`ScriptInstance::entity`] finds the entity a returned
//! instance belongs to, without the script VM, by comparing every entity's
//! instance.
//!
//! The member is read at the offset the `CBaseEntity` datamap gives for the
//! member after it, `m_iszScriptId`, so no layout is guessed.

#[cfg(test)]
#[path = "../tests/tf2/script_instances.rs"]
mod tests;

use crate::entities::Entity;
use crate::{Game, Server};
use sdk_raw::tf2::script_binding::{self as raw, BindingError};
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::OnceLock;

/// An entity's script instance, the `HSCRIPT` TF2's native methods take and
/// return for it, for the current engine callback.
///
/// The VM removes the instance when its entity is removed, so it must not be
/// kept past the callback.
#[doc(alias("HSCRIPT"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScriptInstance<'s> {
	raw: NonNull<sys::HSCRIPT__>,
	_scope: PhantomData<&'s Server<'s>>,
}

impl<'s> ScriptInstance<'s> {
	/// The raw handle, to pass to a native method, as through
	/// [`sdk_raw::tf2::script_binding::handle`].
	pub const fn as_raw(self) -> sys::HSCRIPT {
		self.raw.as_ptr()
	}

	/// The entity whose script instance this is, or `None` if no entity's is,
	/// as for the instance of a script object that is not an entity, or of an
	/// entity removed since.
	///
	/// Every entity's instance is compared with this one, so this costs a pass
	/// over the entity list, and nothing is asked of the script VM. Fails if
	/// the server does not run TF2, or lacks `IServerTools`.
	#[doc(alias("ToEnt"))]
	pub fn entity(self, server: Server<'s>) -> Result<Option<Entity<'s>>, ScriptInstanceError> {
		if server.game() != Game::TeamFortress2 {
			return Err(ScriptInstanceError::WrongGame);
		}

		let tools = server
			.server_tools()
			.map_err(|_| ScriptInstanceError::NoServerTools)?;

		for entity in tools.entities() {
			let offset = instance_offset(entity)?;

			// SAFETY: The entity is live in this callback, and the offset is its
			// script instance's, validated against its `CBaseEntity` datamap.
			if unsafe { raw::script_instance(raw_entity(entity), offset) } == self.as_raw() {
				return Ok(Some(entity));
			}
		}

		Ok(None)
	}

	/// The entity's script instance, if something already made it, such as a
	/// script, or an earlier [`Self::of`]. This never makes one.
	///
	/// Fails if the server does not run TF2, or the `CBaseEntity` datamap does
	/// not locate the instance.
	#[doc(alias("m_hScriptInstance"))]
	pub fn existing(
		server: Server<'s>,
		entity: Entity<'s>,
	) -> Result<Option<Self>, ScriptInstanceError> {
		if server.game() != Game::TeamFortress2 {
			return Err(ScriptInstanceError::WrongGame);
		}

		let offset = instance_offset(entity)?;

		// SAFETY: As in `entity`.
		let raw = unsafe { raw::script_instance(raw_entity(entity), offset) };

		// SAFETY: The handle is the entity's own instance, which the VM keeps
		// until the entity is removed.
		Ok(unsafe { Self::from_raw(raw) })
	}

	/// Wraps a handle, or returns `None` for null.
	///
	/// # Safety
	///
	/// `raw` must be a script instance the game's script VM registered, such
	/// as one a native method returned in this callback, which stays
	/// registered for `'s`.
	pub const unsafe fn from_raw(raw: sys::HSCRIPT) -> Option<Self> {
		match NonNull::new(raw) {
			Some(raw) => Some(Self {
				raw,
				_scope: PhantomData,
			}),

			None => None,
		}
	}

	/// The entity's script instance, made for it if it has none yet.
	///
	/// An entity without one gets one through the `CBaseEntity` binding
	/// `ValidateScriptScope`, which also gives it the script scope a script
	/// touching it would. Fails if the server does not run TF2, the entity is
	/// marked for deletion, the `CBaseEntity` datamap does not locate the
	/// instance, the game has no script VM, or it lacks the binding.
	#[doc(alias("GetScriptInstance", "ValidateScriptScope"))]
	pub fn of(server: Server<'s>, entity: Entity<'s>) -> Result<Self, ScriptInstanceError> {
		if server.game() != Game::TeamFortress2 {
			return Err(ScriptInstanceError::WrongGame);
		}

		// An entity whose `UpdateOnRemove` already removed its instance would
		// register a new one that nothing removes.
		if entity.is_marked_for_deletion() {
			return Err(ScriptInstanceError::MarkedForDeletion);
		}

		let offset = instance_offset(entity)?;

		// SAFETY: As in `entity`. The entity is not marked for deletion, and
		// `ValidateScriptScope` only creates its instance and scope.
		let raw = unsafe { raw::ensure_script_instance(raw_entity(entity), offset) }.map_err(
			|error| match error {
				BindingError::Rejected => ScriptInstanceError::NoScriptVm,

				BindingError::Unavailable | BindingError::SignatureMismatch => {
					ScriptInstanceError::UnsupportedMethod
				}
			},
		)?;

		Ok(Self {
			raw,
			_scope: PhantomData,
		})
	}
}

/// Why an entity's script instance could not be found or made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ScriptInstanceError {
	/// The server does not run Team Fortress 2.
	#[error("script instances require Team Fortress 2")]
	WrongGame,

	/// The entity is marked for deletion, and has given up its instance.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The `CBaseEntity` datamap does not declare `m_iszScriptId` where the
	/// instance handle can precede it.
	#[error("the entity's datamap does not locate its script instance")]
	UnsupportedLayout,

	/// The game has no script VM to register the instance with, as under
	/// `-scripting`.
	#[error("the game has no script VM")]
	NoScriptVm,

	/// The entity's script descriptors lack `ValidateScriptScope`, or its
	/// signature differs from the SDK's.
	#[error("the game does not expose the expected native method")]
	UnsupportedMethod,

	/// The game module does not export `IServerTools`, through which the
	/// entities are listed.
	#[error("the game does not export IServerTools")]
	NoServerTools,
}

/// The offset of every entity's script instance, from the `CBaseEntity`
/// datamap, which `entity`'s maps include.
fn instance_offset(entity: Entity<'_>) -> Result<usize, ScriptInstanceError> {
	static OFFSET: OnceLock<Option<usize>> = OnceLock::new();

	OFFSET
		.get_or_init(|| raw::script_instance_offset(entity.data_maps()))
		.ok_or(ScriptInstanceError::UnsupportedLayout)
}

/// The entity's pointer, for the raw functions.
fn raw_entity(entity: Entity<'_>) -> NonNull<sys::CBaseEntity> {
	// SAFETY: An entity's pointer is never null.
	unsafe { NonNull::new_unchecked(entity.as_ptr()) }
}
