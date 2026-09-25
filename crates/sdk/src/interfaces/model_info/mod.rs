//! `IVModelInfo`, the server's registry of precached models.

use crate::ffi::{copy_cstr, vcall};
use std::ffi::{CStr, CString, c_int};

interface! {
	/// The server's registry of precached models (`IVModelInfo`).
	#[doc(alias = "IVModelInfo")]
	pub struct ModelInfo(sys::IVModelInfo) = Engine c"VModelInfoServer004";
}

impl<'s> ModelInfo<'s> {
	/// The precache index of a model, such as `models/player/scout.mdl`.
	#[doc(alias = "GetModelIndex")]
	pub fn model_index(self, name: &CStr) -> Option<c_int> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let index = unsafe { vcall!(self.as_ptr() => IVModelInfo_GetModelIndex(name.as_ptr())) };

		(index >= 0).then_some(index)
	}

	/// The name of the model at a precache index.
	#[doc(alias = "GetModelName")]
	pub fn model_name(self, index: c_int) -> Option<CString> {
		// SAFETY: As for `model_index`.
		let model = unsafe { vcall!(self.as_ptr() => IVModelInfo_GetModel(index)) };

		if model.is_null() {
			return None;
		}

		// SAFETY: As for `model_index`, and the model is live.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IVModelInfo_GetModelName(model))) }
	}
}
