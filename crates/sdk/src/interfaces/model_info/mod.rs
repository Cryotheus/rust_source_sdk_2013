//! `IVModelInfo`, the server's registry of precached models.

use sdk_raw::util::cstr::copy_cstr;
use sdk_raw::vcall;
use std::ffi::{CStr, CString, c_int};

/// The name in the studio header of the model the engine loads in place of
/// one whose file it cannot find.
const ERROR_MODEL: &[u8] = b"error.mdl";

interface! {
	/// The server's registry of precached models (`IVModelInfo`).
	#[doc(alias = "IVModelInfo")]
	pub struct ModelInfo(sys::IVModelInfo) = Engine c"VModelInfoServer004";
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
	#[doc(alias = "GetModelIndex")]
	pub fn model_index(self, name: &CStr) -> Option<c_int> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let index = unsafe { vcall!(self.as_ptr() => IVModelInfo_GetModelIndex(name.as_ptr())) };

		(index >= 0).then_some(index)
	}

	/// The name of the model at a precache index, or `None` if no model has
	/// that index.
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

	/// The name a model's studio header gives, such as
	/// `player\items\scout\bonk_helmet.mdl`, or `None` if no model has the
	/// index or it is not a studio model.
	#[doc(alias = "GetStudiomodel")]
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

#[cfg(test)]
mod tests {
	use super::*;
	use sdk_raw::util::mock::{mock_vtable, unexpected_call};
	use std::cell::Cell;
	use std::mem::zeroed;
	use std::ptr::{NonNull, null};

	thread_local! {
		/// The studio header the mock gives every model.
		static STUDIO: Cell<*mut sys::studiohdr_t> = const { Cell::new(std::ptr::null_mut()) };
	}

	/// A model pointer the mock hands out for index -2 only, never read.
	const MODEL: *const sys::model_t = 0x10 as *const sys::model_t;

	unsafe extern "C" fn get_model(_: *mut sys::IVModelInfo, index: c_int) -> *const sys::model_t {
		if index == -2 { MODEL } else { null() }
	}

	unsafe extern "C" fn get_studiomodel(
		_: *mut sys::IVModelInfo,
		model: *const sys::model_t,
	) -> *mut sys::studiohdr_t {
		assert_eq!(model, MODEL);
		STUDIO.get()
	}

	#[test]
	fn missing_models_are_recognized_by_their_stand_in() {
		let vtable = unsafe {
			mock_vtable::<sys::IVModelInfo__bindgen_vtable>(
				unexpected_call as *const (),
				|vtable| {
					(&raw mut (*vtable).IVModelInfo_GetModel).write(get_model);
					(&raw mut (*vtable).IVModelInfo_GetStudiomodel).write(get_studiomodel);
				},
			)
		};
		let mut raw = sys::IVModelInfo {
			vtable_: &raw const *vtable,
		};
		let info = unsafe { ModelInfo::from_raw(NonNull::from(&mut raw)) };

		let mut missing = studio(b"error.mdl");
		STUDIO.set(&raw mut *missing);
		assert_eq!(info.studio_name(-2).as_deref(), Some(c"error.mdl"));
		assert!(info.is_error_model(-2));

		let mut found = studio(br"player\items\scout\bonk_helmet.mdl");
		STUDIO.set(&raw mut *found);
		assert!(!info.is_error_model(-2));

		// A name filling the whole buffer has no terminator to stop at.
		let mut full = studio(&[b'a'; 64]);
		STUDIO.set(&raw mut *full);
		assert_eq!(
			info.studio_name(-2).map(|name| name.to_bytes().len()),
			Some(64)
		);

		// Unknown indices and models without a studio header.
		assert_eq!(info.studio_name(5), None);
		assert!(!info.is_error_model(5));
		STUDIO.set(std::ptr::null_mut());
		assert_eq!(info.studio_name(-2), None);
	}

	/// A studio header named `name`, as the engine loads one.
	fn studio(name: &[u8]) -> Box<sys::studiohdr_t> {
		// SAFETY: The header is plain data, and only its name is read.
		let mut header: Box<sys::studiohdr_t> = Box::new(unsafe { zeroed() });

		for (slot, &byte) in header.name.iter_mut().zip(name) {
			*slot = byte as _;
		}

		header
	}
}
