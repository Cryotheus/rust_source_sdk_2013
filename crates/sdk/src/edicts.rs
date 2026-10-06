//! The engine's edict table, which pairs each networked entity with an index.

#[cfg(test)]
#[path = "tests/edicts.rs"]
mod tests;

use crate::entities::Entity;
use crate::interfaces::ValveEngine;
use crate::{NotThreadSafe, Server};
use sdk_raw::edicts::FL_EDICT_FREE;
use sdk_raw::util::cstr::borrow_cstr;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

pub use sdk_raw::edicts::MAX_EDICTS;

/// One slot of the engine's edict table, as referred to by an `edict_t *`.
///
/// Each networked entity occupies an edict, whose position in the table is the
/// entity's index. Slot 0 is the world, and the player of each client uses
/// the slot one past the client's own, so players occupy the slots right after
/// the world.
///
/// The engine resets the table when a level loads and gives a slot to another
/// entity once the previous entity is removed. A handle is therefore bound to
/// the scope that produced it, and its slot may have become
/// [free](Self::is_free) or been reassigned by the time it is used. Handles
/// compare equal when they refer to the same slot.
#[doc(alias("edict_t"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Edict<'s> {
	pointer: NonNull<sys::edict_t>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> Edict<'s> {
	/// Wraps a pointer to a slot of the edict table that the engine or game
	/// passed to a callback, such as a client's in a hooked
	/// `IServerGameClients` method, for the callback's scope.
	///
	/// # Safety
	///
	/// `pointer` must identify an element of the edict table of the server
	/// `_server` belongs to, which stays allocated for `'s`, and the call must
	/// obey [`Server::new`]'s main-thread and reentrancy contract.
	pub unsafe fn from_live(_server: Server<'s>, pointer: NonNull<sys::edict_t>) -> Self {
		// SAFETY: The caller vouches for the slot's lifetime during `'s`.
		unsafe { Self::from_raw(pointer) }
	}

	/// Wraps a pointer to a slot of the edict table.
	///
	/// # Safety
	///
	/// `pointer` must identify an element of the engine's edict table, which
	/// must stay allocated for `'s`.
	pub(crate) const unsafe fn from_raw(pointer: NonNull<sys::edict_t>) -> Self {
		Self {
			pointer,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::edict_t {
		self.pointer.as_ptr()
	}

	/// The class name of the slot's entity, if it has one.
	#[doc(alias("GetClassName"))]
	pub fn class_name(self) -> Option<&'s CStr> {
		if self.is_free() {
			return None;
		}

		// SAFETY: As for `index`.
		let networkable =
			NonNull::new(unsafe { (&raw const (*self.as_ptr())._base.m_pNetworkable).read() })?;

		// SAFETY: The networkable belongs to the live entity. Class names are
		// pooled strings, which live until the level ends.
		unsafe { borrow_cstr(vcall!(networkable.as_ptr() => IServerNetworkable_GetClassName())) }
	}

	/// The entity occupying the slot, if any.
	#[doc(alias("GetBaseEntity", "GetUnknown"))]
	pub fn entity(self) -> Option<Entity<'s>> {
		if self.is_free() {
			return None;
		}

		// SAFETY: As for `index`.
		let unknown = NonNull::new(unsafe { (&raw const (*self.as_ptr())._base.m_pUnk).read() })?;

		// SAFETY: An occupied slot points at its live entity, and entities are
		// not freed immediately during `'s`.
		let entity = unsafe { vcall!(unknown.as_ptr() => IServerUnknown_GetBaseEntity()) };

		// SAFETY: The engine returned a live entity.
		NonNull::new(entity).map(|entity| unsafe { Entity::from_raw(entity) })
	}

	/// Records that the entity changed as a whole, so the engine compares all
	/// of its networked variables.
	///
	/// This is `CBaseEdict::StateChanged()`.
	#[doc(alias("StateChanged"))]
	pub fn full_state_changed(self, engine: ValveEngine<'_>) {
		let accessor = engine.change_accessor(self);

		// SAFETY: The slot belongs to the engine's edict table, which outlives
		// `'s`, and the accessor is the engine's for it. This runs on the main
		// thread (`Server::new`).
		unsafe { sdk_raw::edicts::full_state_changed(self.as_ptr(), accessor) };
	}

	/// The slot's position in the edict table, which is also its entity's index.
	#[doc(alias("ENTINDEX", "IndexOfEdict", "m_EdictIndex"))]
	pub fn index(self) -> c_int {
		// SAFETY: The table outlives `'s`. The engine caches every slot's index
		// in the slot itself, which is what the game's `ENTINDEX` reads. Fields
		// are read without forming a reference because the engine writes to
		// edicts through its own pointers.
		let index = unsafe { (&raw const (*self.as_ptr())._base.m_EdictIndex).read() };

		c_int::from(index)
	}

	/// Whether the engine has freed the slot for reuse.
	#[doc(alias("FL_EDICT_FREE", "IsFree"))]
	pub fn is_free(self) -> bool {
		self.state_flags() & FL_EDICT_FREE != 0
	}

	/// Records that the networked variable at `offset` bytes into the entity
	/// changed, so the engine sends it to clients.
	///
	/// This is `CBaseEdict::StateChanged(unsigned short)`, which the game's
	/// network variable wrappers call on assignment. The engine keeps a
	/// limited number of offsets per frame, past which the whole entity is
	/// compared instead.
	#[doc(alias("StateChanged"))]
	pub fn state_changed(self, engine: ValveEngine<'_>, offset: u16) {
		// SAFETY: As for `full_state_changed`, and the shared change info is the
		// engine's.
		unsafe {
			sdk_raw::edicts::state_changed(self.as_ptr(), offset, || {
				engine
					.change_accessor(self)
					.zip(engine.shared_edict_change_info())
			})
		};
	}

	/// Reads the slot's `m_fStateFlags`, a set of `FL_EDICT_*` flags.
	fn state_flags(self) -> c_int {
		// SAFETY: As for `index`.
		unsafe { (&raw const (*self.as_ptr())._base.m_fStateFlags).read() }
	}
}
