//! Tests of reading models through the engine's model information
//! (`IVModelInfo`).

use sdk_raw::test_support::{mock_vtable, unexpected_call};
use source_sdk_2013::Module;
use source_sdk_2013::interfaces::ModelInfo;
use source_sdk_2013::test_support::leak;
use source_sdk_2013::test_support::server::{export, mock_server};
use std::cell::Cell;
use std::ffi::c_int;
use std::mem::zeroed;
use std::ptr::{null, null_mut};

/// A model pointer the mock hands out for index -2 only, never read.
const MODEL: *const sys::model_t = 0x10 as *const sys::model_t;

thread_local! {
	/// The studio header the mock gives every model.
	static STUDIO: Cell<*mut sys::studiohdr_t> = const { Cell::new(null_mut()) };
}

/// `IVModelInfo::GetModel`, which knows only the model at index -2.
unsafe extern "C" fn get_model(_: *mut sys::IVModelInfo, index: c_int) -> *const sys::model_t {
	if index == -2 { MODEL } else { null() }
}

/// `IVModelInfo::GetStudiomodel`, which returns [`STUDIO`].
unsafe extern "C" fn get_studiomodel(
	_: *mut sys::IVModelInfo,
	model: *const sys::model_t,
) -> *mut sys::studiohdr_t {
	assert_eq!(model, MODEL);
	STUDIO.get()
}

#[test]
fn missing_models_are_recognized_by_their_stand_in() {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the
	// vtable being built.
	let vtable = unsafe {
		mock_vtable::<sys::IVModelInfo__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVModelInfo_GetModel).write(get_model);
			(&raw mut (*vtable).IVModelInfo_GetStudiomodel).write(get_studiomodel);
		})
	};

	export(
		Module::Engine,
		ModelInfo::VERSION,
		leak(sys::IVModelInfo {
			vtable_: Box::leak(vtable),
		}),
	);

	let scope = ();
	let info = mock_server(&scope).model_info().unwrap();

	STUDIO.set(studio(b"error.mdl"));
	assert_eq!(info.studio_name(-2).as_deref(), Some(c"error.mdl"));
	assert!(info.is_error_model(-2));

	STUDIO.set(studio(br"player\items\scout\bonk_helmet.mdl"));
	assert!(!info.is_error_model(-2));

	// A name filling the whole buffer has no terminator to stop at.
	STUDIO.set(studio(&[b'a'; 64]));
	assert_eq!(
		info.studio_name(-2).map(|name| name.to_bytes().len()),
		Some(64)
	);

	// Unknown indices and models without a studio header.
	assert_eq!(info.studio_name(5), None);
	assert!(!info.is_error_model(5));
	STUDIO.set(null_mut());
	assert_eq!(info.studio_name(-2), None);
}

/// A leaked studio header named `name`, as the engine loads one.
fn studio(name: &[u8]) -> *mut sys::studiohdr_t {
	// SAFETY: The header is plain data, and only its name is read.
	let mut header: sys::studiohdr_t = unsafe { zeroed() };

	for (slot, &byte) in header.name.iter_mut().zip(name) {
		*slot = byte as _;
	}

	leak(header)
}
