//! A mock engine shared by the unit tests of the crate's core modules, such
//! as `edicts` and `datatables`.

use crate::interfaces::ValveEngine;
use crate::test_support::edicts::{change_accessor, shared_change_info};
use crate::test_support::leak;
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::ptr::NonNull;

/// A leaked engine whose `GetChangeAccessor` and `GetSharedEdictChangeInfo`
/// return the change tracking set on this thread through
/// [`set_change_accessor`] and [`set_shared_change_info`], or null, so that
/// every change is a full one.
///
/// [`set_change_accessor`]: crate::test_support::edicts::set_change_accessor
/// [`set_shared_change_info`]: crate::test_support::edicts::set_shared_change_info
pub(crate) fn change_tracking_engine() -> ValveEngine<'static> {
	// SAFETY: The vtable holds only function pointers, `unexpected_call` aborts
	// whichever slot reaches it, and the patch only writes slots of the vtable
	// being built.
	let vtable = unsafe {
		mock_vtable::<sys::IVEngineServer__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IVEngineServer_GetChangeAccessor).write(change_accessor);
			(&raw mut (*vtable).IVEngineServer_GetSharedEdictChangeInfo).write(shared_change_info);
		})
	};
	let engine = leak(sys::IVEngineServer {
		vtable_: Box::leak(vtable),
	});

	// SAFETY: The engine and its vtable are leaked, so they outlive every use,
	// and the vtable answers what edicts call to mark changes.
	unsafe { ValveEngine::from_raw(NonNull::new(engine).unwrap()) }
}
