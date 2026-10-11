//! The clients that what the server sends through a hooked engine function
//! reaches, as the hook's callback sees them: the recipient filter the game
//! passed the engine, such as with a sound
//! ([`sound_hooks`](crate::hooks::sound)) or a temporary entity
//! ([`temp_entity_hooks`](crate::hooks::temp_entity)).

use source_sdk_2013::raw::vcall;
use source_sdk_2013::sys;
use std::ffi::c_int;
use std::marker::PhantomData;
use std::ptr::NonNull;

/// The recipient filter of something the server is sending, which lists the
/// clients the engine sends it to. Valid only for the hooked call.
#[derive(Debug, Clone, Copy)]
pub struct Recipients<'a> {
	filter: NonNull<sys::IRecipientFilter>,
	_call: PhantomData<&'a sys::IRecipientFilter>,
}

impl Recipients<'_> {
	/// # Safety
	///
	/// `filter` must be the game's live recipient filter of a call on the main
	/// thread, which the returned view must not outlive.
	pub(crate) const unsafe fn new(filter: NonNull<sys::IRecipientFilter>) -> Self {
		Self {
			filter,
			_call: PhantomData,
		}
	}

	/// Whether the engine sends it to no client.
	pub fn is_empty(self) -> bool {
		self.len() == 0
	}

	/// Whether the engine sends it reliably.
	#[doc(alias("IsReliable"))]
	pub fn is_reliable(self) -> bool {
		// SAFETY: The filter is live for the call, on the main thread.
		unsafe { vcall!(self.filter.as_ptr().cast_const() => IRecipientFilter_IsReliable()) }
	}

	/// The entity index of each client the engine sends it to.
	#[doc(alias("GetRecipientIndex"))]
	pub fn iter(self) -> impl Iterator<Item = c_int> {
		(0..self.len()).map(move |slot| {
			// SAFETY: As for `is_reliable`, and the slot is below the count.
			unsafe {
				vcall!(self.filter.as_ptr().cast_const() => IRecipientFilter_GetRecipientIndex(slot as c_int))
			}
		})
	}

	/// How many clients the engine sends it to.
	#[doc(alias("GetRecipientCount"))]
	pub fn len(self) -> usize {
		// SAFETY: As for `is_reliable`.
		let count = unsafe {
			vcall!(self.filter.as_ptr().cast_const() => IRecipientFilter_GetRecipientCount())
		};

		usize::try_from(count).unwrap_or(0)
	}
}
