//! The engine's edict table, which pairs each networked entity with an index.

use crate::NotThreadSafe;
use crate::entities::Entity;
use crate::interfaces::ValveEngine;
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

#[cfg(test)]
mod tests {
	use super::*;

	use crate::test_support::edicts::{
		change_accessor, edict_table, set_change_accessor, set_shared_change_info,
		shared_change_info,
	};

	use sdk_raw::edicts::{FL_EDICT_CHANGED, FL_FULL_EDICT_CHANGED};
	use sdk_raw::test_support::edicts::mock_edict;
	use sdk_raw::test_support::{mock_vtable, unexpected_call};
	use std::ptr::null_mut;

	#[test]
	fn reads_the_cached_index_and_free_flag() {
		let mut table = edict_table(3, |slot| slot == 1);
		let base = table.as_mut_ptr();
		let edict = |slot: usize| unsafe { Edict::from_raw(NonNull::new(base.add(slot)).unwrap()) };

		assert_eq!(edict(0).index(), 0);
		assert_eq!(edict(2).index(), 2);
		assert!(!edict(0).is_free());
		assert!(edict(1).is_free());
		assert_eq!(edict(1), edict(1));
		assert_ne!(edict(1), edict(2));
		assert_eq!(edict(2).as_ptr(), unsafe { base.add(2) });
		assert_eq!(edict(1).entity(), None);
		assert_eq!(edict(0).class_name(), None);
	}

	/// The algorithm itself is tested in `sdk_raw::edicts`; this checks that
	/// the engine's change tracking reaches it.
	#[test]
	fn state_changes_reach_the_engines_change_tracking() {
		let vtable = unsafe {
			mock_vtable::<sys::IVEngineServer__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVEngineServer_GetChangeAccessor).write(change_accessor);
					(&raw mut (*vtable).IVEngineServer_GetSharedEdictChangeInfo)
						.write(shared_change_info);
				},
			)
		};

		let mut interface = sys::IVEngineServer {
			vtable_: &raw const *vtable,
		};
		let engine = unsafe { ValveEngine::from_raw(NonNull::from(&mut interface)) };
		let mut accessor = sys::IChangeInfoAccessor {
			m_iChangeInfo: 0,
			m_iChangeInfoSerialNumber: 0,
		};
		let mut shared = Box::new(unsafe { std::mem::zeroed::<sys::CSharedEdictChangeInfo>() });

		shared.m_iSerialNumber = 7;
		shared.m_nChangeInfos = 3;
		set_change_accessor(&raw mut accessor);
		set_shared_change_info(&raw mut *shared);

		let mut slot = mock_edict(4, false);
		let edict = unsafe { Edict::from_raw(NonNull::from(&mut slot)) };

		// The first change this frame claims the next free change info.
		edict.state_changed(engine, 40);
		assert_eq!(slot._base.m_fStateFlags, FL_EDICT_CHANGED);
		assert_eq!(
			(accessor.m_iChangeInfo, accessor.m_iChangeInfoSerialNumber),
			(3, 7)
		);
		assert_eq!(shared.m_nChangeInfos, 4);
		assert_eq!(shared.m_ChangeInfos[3].m_nChangeOffsets, 1);
		assert_eq!(shared.m_ChangeInfos[3].m_ChangeOffsets[0], 40);

		// Later changes append new offsets once.
		edict.state_changed(engine, 44);
		edict.state_changed(engine, 40);
		assert_eq!(shared.m_ChangeInfos[3].m_nChangeOffsets, 2);
		assert_eq!(shared.m_ChangeInfos[3].m_ChangeOffsets[1], 44);

		// A full change releases the change info through the accessor.
		edict.full_state_changed(engine);
		assert_eq!(
			slot._base.m_fStateFlags,
			FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED
		);
		assert_eq!(accessor.m_iChangeInfoSerialNumber, 0);

		// Without the engine's change tracking, every change is a full one.
		set_change_accessor(null_mut());
		set_shared_change_info(null_mut());

		let mut slot = mock_edict(5, false);
		let edict = unsafe { Edict::from_raw(NonNull::from(&mut slot)) };

		edict.state_changed(engine, 40);
		assert_eq!(
			slot._base.m_fStateFlags,
			FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED
		);
	}
}
