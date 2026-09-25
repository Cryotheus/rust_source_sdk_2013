//! The engine's edict table, which pairs each networked entity with an index.

use crate::entities::Entity;
use crate::ffi::{NotThreadSafe, borrow_cstr, vcall};
use crate::interfaces::ValveEngine;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// Number of slots in the edict table.
///
/// This is `MAX_EDICTS` from `public/const.h`.
pub const MAX_EDICTS: c_int = 1 << 11;

/// `FL_EDICT_CHANGED` from `public/edict.h`, set when a networked variable changes.
const FL_EDICT_CHANGED: c_int = 1 << 0;

/// `FL_EDICT_FREE` from `public/edict.h`, set while a slot holds no entity.
const FL_EDICT_FREE: c_int = 1 << 1;

/// `FL_FULL_EDICT_CHANGED` from `public/edict.h`, set when every networked
/// variable must be compared rather than only the recorded offsets.
const FL_FULL_EDICT_CHANGED: c_int = 1 << 8;

/// `MAX_CHANGE_OFFSETS` from `public/edict.h`.
const MAX_CHANGE_OFFSETS: u16 = 19;

/// `MAX_EDICT_CHANGE_INFOS` from `public/edict.h`.
const MAX_EDICT_CHANGE_INFOS: u16 = 100;

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
#[doc(alias = "edict_t")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Edict<'s> {
	pointer: NonNull<sys::edict_t>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> Edict<'s> {
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

	/// The slot's position in the edict table, which is also its entity's index.
	#[doc(alias = "ENTINDEX")]
	#[doc(alias = "IndexOfEdict")]
	pub fn index(self) -> c_int {
		// SAFETY: The table outlives `'s`. The engine caches every slot's index
		// in the slot itself, which is what the game's `ENTINDEX` reads. Fields
		// are read without forming a reference because the engine writes to
		// edicts through its own pointers.
		let index = unsafe { (&raw const (*self.as_ptr())._base.m_EdictIndex).read() };

		c_int::from(index)
	}

	/// Whether the engine has freed the slot for reuse.
	#[doc(alias = "IsFree")]
	pub fn is_free(self) -> bool {
		self.state_flags() & FL_EDICT_FREE != 0
	}

	/// The entity occupying the slot, if any.
	#[doc(alias = "GetUnknown")]
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

	/// The class name of the slot's entity, if it has one.
	#[doc(alias = "GetClassName")]
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

	/// Records that the networked variable at `offset` bytes into the entity
	/// changed, so the engine sends it to clients.
	///
	/// This is `CBaseEdict::StateChanged(unsigned short)`, which the game's
	/// network variable wrappers call on assignment. The engine keeps a
	/// limited number of offsets per frame, past which the whole entity is
	/// compared instead.
	#[doc(alias = "StateChanged")]
	pub fn state_changed(self, engine: ValveEngine<'_>, offset: u16) {
		let flags = self.state_flags();

		if flags & FL_FULL_EDICT_CHANGED != 0 {
			return;
		}

		self.set_state_flags(flags | FL_EDICT_CHANGED);

		let (Some(accessor), Some(shared)) = (
			engine.change_accessor(self),
			engine.shared_edict_change_info(),
		) else {
			return self.mark_fully_changed(None);
		};

		let accessor = accessor.as_ptr();
		let shared = shared.as_ptr();

		// SAFETY: Both structures are engine-owned and only touched on the main
		// thread. Indices are validated against the fixed array lengths before
		// use, and no references are formed.
		unsafe {
			let serial_number = (&raw const (*shared).m_iSerialNumber).read();
			let infos = (&raw mut (*shared).m_ChangeInfos).cast::<sys::CEdictChangeInfo>();

			if (&raw const (*accessor).m_iChangeInfoSerialNumber).read() == serial_number {
				// The entity already has change info this frame, so add the offset.
				let index = (&raw const (*accessor).m_iChangeInfo).read();

				if index >= MAX_EDICT_CHANGE_INFOS {
					return self.mark_fully_changed(Some(accessor));
				}

				let info = infos.add(usize::from(index));
				let count = (&raw const (*info).m_nChangeOffsets).read();
				let offsets = (&raw mut (*info).m_ChangeOffsets).cast::<u16>();

				for slot in 0..count.min(MAX_CHANGE_OFFSETS) {
					if offsets.add(usize::from(slot)).read() == offset {
						return;
					}
				}

				if count >= MAX_CHANGE_OFFSETS {
					self.mark_fully_changed(Some(accessor));
				} else {
					offsets.add(usize::from(count)).write(offset);
					(&raw mut (*info).m_nChangeOffsets).write(count + 1);
				}
			} else {
				// Claim a new change info for this frame.
				let count = (&raw const (*shared).m_nChangeInfos).read();

				if count >= MAX_EDICT_CHANGE_INFOS {
					return self.mark_fully_changed(Some(accessor));
				}

				(&raw mut (*accessor).m_iChangeInfo).write(count);
				(&raw mut (*shared).m_nChangeInfos).write(count + 1);
				(&raw mut (*accessor).m_iChangeInfoSerialNumber).write(serial_number);

				let info = infos.add(usize::from(count));

				(&raw mut (*info).m_ChangeOffsets)
					.cast::<u16>()
					.write(offset);
				(&raw mut (*info).m_nChangeOffsets).write(1);
			}
		}
	}

	/// Records that the entity changed as a whole, so the engine compares all
	/// of its networked variables.
	///
	/// This is `CBaseEdict::StateChanged()`.
	pub fn full_state_changed(self, engine: ValveEngine<'_>) {
		self.mark_fully_changed(engine.change_accessor(self).map(NonNull::as_ptr));
	}

	fn mark_fully_changed(self, accessor: Option<*mut sys::IChangeInfoAccessor>) {
		self.set_state_flags(self.state_flags() | FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED);

		if let Some(accessor) = accessor {
			// SAFETY: The accessor is the engine's record for this edict.
			unsafe { (&raw mut (*accessor).m_iChangeInfoSerialNumber).write(0) };
		}
	}

	fn state_flags(self) -> c_int {
		// SAFETY: As for `index`.
		unsafe { (&raw const (*self.as_ptr())._base.m_fStateFlags).read() }
	}

	fn set_state_flags(self, flags: c_int) {
		// SAFETY: As for `index`. The game writes these flags the same way.
		unsafe { (&raw mut (*self.as_ptr())._base.m_fStateFlags).write(flags) };
	}
}

#[cfg(test)]
pub(crate) mod test_support {
	use super::*;
	use std::ptr::null_mut;

	/// Builds an edict as the engine lays out slot `index` of its table.
	pub(crate) fn mock_edict(index: c_int, free: bool) -> sys::edict_t {
		sys::edict_t {
			_base: sys::CBaseEdict {
				m_fStateFlags: if free { FL_EDICT_FREE } else { 0 },
				m_NetworkSerialNumber: 0,
				m_EdictIndex: index.try_into().unwrap(),
				m_pNetworkable: null_mut(),
				m_pUnk: null_mut(),
			},
			freetime: 0.0,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::test_support::mock_edict;
	use super::*;
	use crate::ffi::test_support::{mock_vtable, unexpected_call};
	use std::cell::Cell;
	use std::ptr::null_mut;

	#[test]
	fn reads_the_cached_index_and_free_flag() {
		let mut table = [
			mock_edict(0, false),
			mock_edict(1, true),
			mock_edict(2, false),
		];
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

	thread_local! {
		static ACCESSOR: Cell<*mut sys::IChangeInfoAccessor> = const { Cell::new(null_mut()) };
		static SHARED: Cell<*mut sys::CSharedEdictChangeInfo> = const { Cell::new(null_mut()) };
	}

	unsafe extern "C" fn change_accessor(
		_: *mut sys::IVEngineServer,
		_: *const sys::edict_t,
	) -> *mut sys::IChangeInfoAccessor {
		ACCESSOR.get()
	}

	unsafe extern "C" fn shared_change_info(
		_: *mut sys::IVEngineServer,
	) -> *mut sys::CSharedEdictChangeInfo {
		SHARED.get()
	}

	#[test]
	fn state_changes_follow_the_engines_change_tracking() {
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
		ACCESSOR.set(&raw mut accessor);
		SHARED.set(&raw mut *shared);

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

		// Overflowing the offsets falls back to a full comparison.
		for offset in 0..MAX_CHANGE_OFFSETS {
			edict.state_changed(engine, 100 + offset);
		}

		assert_eq!(
			slot._base.m_fStateFlags,
			FL_EDICT_CHANGED | FL_FULL_EDICT_CHANGED
		);
		assert_eq!(accessor.m_iChangeInfoSerialNumber, 0);

		// Once fully changed, offsets are no longer recorded.
		let recorded = shared.m_ChangeInfos[3];
		edict.state_changed(engine, 2);
		assert_eq!(
			shared.m_ChangeInfos[3].m_nChangeOffsets,
			recorded.m_nChangeOffsets
		);

		ACCESSOR.set(null_mut());
		SHARED.set(null_mut());
	}
}
