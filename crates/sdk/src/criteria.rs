//! The criteria sets of the response system, which an entity fills before
//! the game picks what it says or does in response to a concept, such as a
//! TF2 player's taunt (see [`raw::criteria`]).
//!
//! The game keeps each criterion's name as a symbol of its own symbol table,
//! which is not exported, so a [`CriterionName`] tells criteria apart without
//! spelling them. A plugin learns which symbol a criterion has by watching
//! what a set gains when the game appends it, as through
//! [`CriteriaSet::append_criteria_of`] while the entity is in a state that
//! adds it. Symbols are never removed while the game's module stays loaded.
//!
//! [`raw::criteria`]: crate::raw::criteria

use crate::NotThreadSafe;
use crate::entities::Entity;
use crate::raw::criteria::entries;
use crate::raw::vcall;
use std::ffi::{CStr, c_char};
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The longest value an entry holds, without its terminator, as the game's
/// `CritEntry_t::SetValue` truncates it.
const MAX_VALUE: usize = 63;

/// A criteria set of the game's (`AI_CriteriaSet`), live for the callback in
/// which the game passed it, such as a hook of
/// `CBaseEntity::ModifyOrAppendCriteria`.
#[derive(Debug, Clone, Copy)]
pub struct CriteriaSet<'s> {
	raw: NonNull<sys::AI_CriteriaSet>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> CriteriaSet<'s> {
	/// Wraps the set at `raw`.
	///
	/// # Safety
	///
	/// `raw` points to a live criteria set of the game's, which stays live for
	/// `'s` and which only the main thread changes.
	pub const unsafe fn from_raw(raw: NonNull<sys::AI_CriteriaSet>) -> Self {
		Self {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		}
	}

	/// Has `entity` append its criteria to the set, as the game does before
	/// it picks a response for the entity: its class's
	/// `ModifyOrAppendCriteria`, through its vtable, so the hooks of the
	/// method run too. Criteria the set holds already take the entity's
	/// values; none are removed.
	#[doc(alias("ModifyOrAppendCriteria"))]
	pub fn append_criteria_of(self, entity: Entity<'s>) {
		let this = entity.as_ptr();

		// SAFETY: The entity is live for the scope, and every entity's primary
		// vtable holds the method, which takes a live set.
		unsafe {
			vcall!(this as sys::CBaseEntity__bindgen_vtable => CBaseEntity_ModifyOrAppendCriteria(self.as_ptr()))
		};
	}

	/// The set's pointer.
	pub const fn as_ptr(self) -> *mut sys::AI_CriteriaSet {
		self.raw.as_ptr()
	}

	/// The entry of the criterion `name`, if the set holds it.
	fn entry(self, name: CriterionName) -> Option<NonNull<sys::AI_CriteriaSet_CritEntry_t>> {
		// SAFETY: As for `names`.
		let entries = unsafe { entries(self.raw) }?;

		entries.into_iter().find(|entry| {
			// SAFETY: As for `names`.
			unsafe { (&raw const (*entry.as_ptr()).criterianame.m_Id).read() == name.0 }
		})
	}

	/// The names of the criteria in the set, in no particular order, or
	/// `None` if the set is not laid out as the bindings say.
	pub fn names(self) -> Option<Vec<CriterionName>> {
		// SAFETY: The set is live for the scope, and nothing else changes it
		// while its entries are read.
		let entries = unsafe { entries(self.raw) }?;

		Some(
			entries
				.into_iter()
				// SAFETY: Each entry is live in the set, and its symbol is plain data.
				.map(|entry| {
					CriterionName(unsafe {
						(&raw const (*entry.as_ptr()).criterianame.m_Id).read()
					})
				})
				.collect(),
		)
	}

	/// Sets the value of the criterion `name`, cut to 63 bytes as the game
	/// cuts it, if the set holds it. Returns whether it does.
	///
	/// The game matches a rule's criterion against the value, so another
	/// value keeps the rules that require the criterion's from matching.
	#[doc(alias("SetValue"))]
	pub fn set_value(self, name: CriterionName, value: &CStr) -> bool {
		let Some(entry) = self.entry(name) else {
			return false;
		};

		let bytes = value.to_bytes();
		let length = bytes.len().min(MAX_VALUE);

		// SAFETY: The entry is live in the set, and its value is an inline
		// array of 64 bytes, of which this writes at most 63 and a terminator.
		unsafe {
			let target = (&raw mut (*entry.as_ptr()).value).cast::<c_char>();

			target.copy_from_nonoverlapping(bytes.as_ptr().cast(), length);
			target.add(length).write(0);
		}

		true
	}

	/// The value of the criterion `name`, copied, if the set holds it.
	pub fn value(self, name: CriterionName) -> Option<Vec<u8>> {
		let entry = self.entry(name)?;

		// SAFETY: The entry is live in the set, and its inline value is plain
		// data, which the game keeps terminated.
		let value = unsafe { (&raw const (*entry.as_ptr()).value).read() };
		let bytes: Vec<u8> = value
			.iter()
			.map(|&byte| byte as u8)
			.take_while(|&byte| byte != 0)
			.collect();

		Some(bytes)
	}
}

/// The name of a criterion, as the game's symbol for it (`CUtlSymbol`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CriterionName(u16);

impl CriterionName {
	/// The criterion the game numbers `symbol`.
	pub const fn from_raw(symbol: u16) -> Self {
		Self(symbol)
	}

	/// The symbol, as the game numbers it.
	pub const fn to_raw(self) -> u16 {
		self.0
	}
}
