//! Hand-written ABI of user messages, and of the recipient filters Rust
//! implements for the engine.

use crate::abi::CppDestructors;
use std::ffi::c_int;
use std::fmt::{Debug, Formatter};
use std::mem::offset_of;

/// `bool IRecipientFilter::IsReliable() const` and `IsInitMessage`.
type FlagFn = unsafe extern "C" fn(this: *const sys::IRecipientFilter) -> bool;

/// `int IRecipientFilter::GetRecipientCount() const`.
type RecipientCountFn = unsafe extern "C" fn(this: *const sys::IRecipientFilter) -> c_int;

/// `int IRecipientFilter::GetRecipientIndex(int slot) const`.
type RecipientIndexFn =
	unsafe extern "C" fn(this: *const sys::IRecipientFilter, slot: c_int) -> c_int;

// The hand-written vtable lines up with the generated one under the target's
// ABI, whose destructor slots `CppDestructors` selects.
const _: () = {
	assert!(
		offset_of!(RecipientFilterVtable, is_reliable)
			== offset_of!(
				sys::IRecipientFilter__bindgen_vtable,
				IRecipientFilter_IsReliable
			)
	);
	assert!(
		offset_of!(RecipientFilterVtable, is_init_message)
			== offset_of!(
				sys::IRecipientFilter__bindgen_vtable,
				IRecipientFilter_IsInitMessage
			)
	);
	assert!(
		offset_of!(RecipientFilterVtable, recipient_count)
			== offset_of!(
				sys::IRecipientFilter__bindgen_vtable,
				IRecipientFilter_GetRecipientCount
			)
	);
	assert!(
		offset_of!(RecipientFilterVtable, recipient_index)
			== offset_of!(
				sys::IRecipientFilter__bindgen_vtable,
				IRecipientFilter_GetRecipientIndex
			)
	);
	assert!(
		size_of::<RecipientFilterVtable>() == size_of::<sys::IRecipientFilter__bindgen_vtable>()
	);
};

// The generated binding has these signatures.
const _: [fn(&sys::IRecipientFilter__bindgen_vtable) -> FlagFn; 2] = [
	|vtable| vtable.IRecipientFilter_IsReliable,
	|vtable| vtable.IRecipientFilter_IsInitMessage,
];

const _: fn(&sys::IRecipientFilter__bindgen_vtable) -> RecipientCountFn =
	|vtable| vtable.IRecipientFilter_GetRecipientCount;

const _: fn(&sys::IRecipientFilter__bindgen_vtable) -> RecipientIndexFn =
	|vtable| vtable.IRecipientFilter_GetRecipientIndex;

/// An `IRecipientFilter` implemented in Rust, which lends the engine a list of
/// player indices while it sends a message or sound.
///
/// Only the interface's virtual methods are implemented, so the filter must
/// not be given to game code that casts it to the game's `CRecipientFilter`,
/// such as `CSceneEntity::SetRecipientFilter`. The engine never deletes a
/// filter it is given, so the destructor slots do nothing.
#[doc(alias("IRecipientFilter"))]
#[repr(C)]
pub struct RecipientFilter<'a> {
	vtable: &'static RecipientFilterVtable,
	players: &'a [c_int],
	reliable: bool,
}

impl<'a> RecipientFilter<'a> {
	const VTABLE: RecipientFilterVtable = RecipientFilterVtable {
		destructor: CppDestructors::new_noop(),
		is_reliable: Self::is_reliable,
		is_init_message: Self::is_init_message,
		recipient_count: Self::recipient_count,
		recipient_index: Self::recipient_index,
	};

	/// A filter that sends to `players`, by player index, in their reliable
	/// streams if `reliable` is set.
	pub const fn new(players: &'a [c_int], reliable: bool) -> Self {
		Self {
			vtable: &Self::VTABLE,
			players,
			reliable,
		}
	}

	/// `IsInitMessage`: the filter never sends a level's initial messages.
	unsafe extern "C" fn is_init_message(_: *const sys::IRecipientFilter) -> bool {
		false
	}

	/// `IsReliable`.
	unsafe extern "C" fn is_reliable(this: *const sys::IRecipientFilter) -> bool {
		// SAFETY: The engine calls it on the filter it was given.
		unsafe { Self::read(this) }.1
	}

	/// The players and reliability of the filter at `this`.
	///
	/// # Safety
	///
	/// `this` must be a live filter made by [`RecipientFilter::new`], whose
	/// borrow of the players outlives the result.
	unsafe fn read<'b>(this: *const sys::IRecipientFilter) -> (&'b [c_int], bool) {
		let this = this.cast::<RecipientFilter<'b>>();

		// SAFETY: As the caller promises. The filter only lends out what it
		// borrows, for the call.
		unsafe { ((*this).players, (*this).reliable) }
	}

	/// `GetRecipientCount`, clamped to `int`.
	unsafe extern "C" fn recipient_count(this: *const sys::IRecipientFilter) -> c_int {
		// SAFETY: As for `is_reliable`.
		let (players, _) = unsafe { Self::read(this) };

		c_int::try_from(players.len()).unwrap_or(c_int::MAX)
	}

	/// `GetRecipientIndex`, or -1 for a slot out of range.
	unsafe extern "C" fn recipient_index(this: *const sys::IRecipientFilter, slot: c_int) -> c_int {
		// SAFETY: As for `is_reliable`.
		let (players, _) = unsafe { Self::read(this) };

		usize::try_from(slot)
			.ok()
			.and_then(|slot| players.get(slot))
			.copied()
			.unwrap_or(-1)
	}

	/// The filter as the `IRecipientFilter` the engine takes, valid while the
	/// filter is borrowed. The engine only reads through it.
	pub const fn as_raw(&self) -> *mut sys::IRecipientFilter {
		(&raw const *self).cast_mut().cast()
	}
}

impl Debug for RecipientFilter<'_> {
	fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("RecipientFilter")
			.field("players", &self.players)
			.field("reliable", &self.reliable)
			.finish_non_exhaustive()
	}
}

/// `IRecipientFilter`'s vtable, which the assertions at the top of the module
/// check against the generated one for the target's ABI.
#[repr(C)]
struct RecipientFilterVtable {
	destructor: CppDestructors,
	is_reliable: FlagFn,
	is_init_message: FlagFn,
	recipient_count: RecipientCountFn,
	recipient_index: RecipientIndexFn,
}
