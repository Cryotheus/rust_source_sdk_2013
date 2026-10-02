//! Adapts generic primary-vtable discovery to TF2 vote hook targets.

use super::VoteHookTargetError;
use sdk_raw::util;
use std::ffi::c_void;
use std::ptr::NonNull;

pub(super) struct VotingOvft(util::Image);

impl VotingOvft {
	/// # Safety
	/// The factory's module must remain loaded throughout the snapshot.
	pub(super) unsafe fn load(factory: usize) -> Result<Self, VoteHookTargetError> {
		// SAFETY: The caller keeps the factory's module loaded while inspected.
		let image = unsafe { util::Image::load(factory) }.map_err(|error| match error {
			util::Error::InvalidImage => VoteHookTargetError::InvalidImage,
			util::Error::Io(error) => VoteHookTargetError::Image(error),
		})?;

		Ok(Self(image))
	}

	pub(super) fn find(&self, class: &str, slot: usize) -> Option<NonNull<*mut c_void>> {
		NonNull::new(self.0.primary_vtable(class, slot)? as *mut *mut c_void)
	}
}
