//! `IVModelInfo`, the server's registry of precached models.

use sdk_raw::util::cstr::copy_cstr;
use sdk_raw::vcall;
use std::ffi::{CStr, CString, c_int};

/// The name in the studio header of the model the engine loads in place of
/// one whose file it cannot find.
const ERROR_MODEL: &[u8] = b"error.mdl";

interface! {
	/// The server's registry of precached models (`IVModelInfo`).
	#[doc(alias("IVModelInfo"))]
	pub struct ModelInfo(sys::IVModelInfo) = Engine sdk_raw::interfaces::model_info::VERSION;
}

impl<'s> ModelInfo<'s> {
	/// Whether the model at an index, including a dynamic model's negative
	/// index, is the engine's stand-in for a model file it could not load
	/// (`error.mdl`). `false` if no model has the index, or it is not a studio
	/// model.
	///
	/// TF2's 64-bit Windows server was observed to have loaded a dynamic model,
	/// an item's per-class model, by the time the item was equipped, and to
	/// load `error.mdl` for one whose file is missing, as for an item worn by a
	/// class it has no model for. The owner's client then drew nothing for
	/// it. Linux servers have not been tested.
	pub fn is_error_model(self, index: c_int) -> bool {
		self.studio_name(index)
			.is_some_and(|name| name.to_bytes().eq_ignore_ascii_case(ERROR_MODEL))
	}

	/// The precache index of a model, such as `models/player/scout.mdl`, or
	/// `None` if the model is not precached. Dynamic models, whose indices are
	/// negative, also give `None`.
	#[doc(alias("GetModelIndex"))]
	pub fn model_index(self, name: &CStr) -> Option<c_int> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let index = unsafe { vcall!(self.as_ptr() => IVModelInfo_GetModelIndex(name.as_ptr())) };

		(index >= 0).then_some(index)
	}

	/// The name of the model at a precache index, or `None` if no model has
	/// that index.
	#[doc(alias("GetModelName"))]
	pub fn model_name(self, index: c_int) -> Option<CString> {
		// SAFETY: As for `model_index`.
		let model = unsafe { vcall!(self.as_ptr() => IVModelInfo_GetModel(index)) };

		if model.is_null() {
			return None;
		}

		// SAFETY: As for `model_index`, and the model is live.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IVModelInfo_GetModelName(model))) }
	}

	/// The name a model's studio header gives, such as
	/// `player\items\scout\bonk_helmet.mdl`, or `None` if no model has the
	/// index or it is not a studio model.
	#[doc(alias("GetStudiomodel"))]
	pub fn studio_name(self, index: c_int) -> Option<CString> {
		// SAFETY: As for `model_index`.
		let model = unsafe { vcall!(self.as_ptr() => IVModelInfo_GetModel(index)) };

		if model.is_null() {
			return None;
		}

		// SAFETY: As for `model_index`, and the model is live. The studio header
		// belongs to the model, which stays loaded while an entity uses it, and
		// its name is copied before anything else runs.
		let studio = unsafe { vcall!(self.as_ptr() => IVModelInfo_GetStudiomodel(model)) };

		if studio.is_null() {
			return None;
		}

		// SAFETY: As above. The name is a fixed buffer within the header, read
		// in place without forming a reference to the header.
		let name = unsafe { (&raw const (*studio).name).read() };
		let bytes = name.map(|byte| byte as u8);
		let len = bytes
			.iter()
			.position(|&byte| byte == 0)
			.unwrap_or(bytes.len());

		CString::new(&bytes[..len]).ok()
	}
}
