//! A mock of Metamod's `ISmmAPI`, of either supported version.

use crate::api::{MetamodApi, MetamodApiBinding, MetamodVersion};
use crate::sys::api::ISmmApi;
use crate::sys::sourcehook::MMIFACE_SOURCEHOOK;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::mem::MaybeUninit;
use std::ptr::{self, NonNull};

/// The slots of the longer of the versions' vtables, the 2.0 one.
const SLOTS: usize = 34;

/// Metamod's `ISmmAPI`, with the hooking library of its version.
#[repr(C)]
pub(crate) struct MockSmm {
	api: ISmmApi,
	/// The `ISourceHook` that `MetaFactory` returns.
	pub(crate) sourcehook: *mut c_void,
	/// The KHook that the 2.0 `GetDetourInterface` returns.
	pub(crate) khook: *mut c_void,
	_vtable: Box<[MaybeUninit<*const ()>; SLOTS]>,
}

impl MockSmm {
	/// A Metamod of `version`, whose vtable has `GetApiVersions`,
	/// `MetaFactory`, `GetDetourInterface` on 2.0, and then `methods` at their
	/// slots. Its other slots are uninitialized.
	pub(crate) fn new(version: MetamodVersion, methods: &[(usize, *const ())]) -> Box<Self> {
		let own: &[(usize, *const ())] = match version {
			MetamodVersion::Stable1226 => &[
				(10, stable_api_versions as *const ()),
				(13, meta_factory as *const ()),
			],

			MetamodVersion::Dev1469 => &[
				(10, dev_api_versions as *const ()),
				(12, meta_factory as *const ()),
				(33, detour_interface as *const ()),
			],
		};

		let mut vtable = Box::new([MaybeUninit::uninit(); SLOTS]);

		for &(slot, method) in own.iter().chain(methods) {
			vtable[slot].write(method);
		}

		Box::new(Self {
			api: ISmmApi {
				vtable: vtable.as_ptr().cast(),
			},
			sourcehook: ptr::null_mut(),
			khook: ptr::null_mut(),
			_vtable: vtable,
		})
	}

	/// The mock, as a plugin's callback sees Metamod.
	pub(crate) fn api(&self) -> MetamodApi<'_> {
		// SAFETY: The mock has the methods the detection calls.
		let binding = unsafe { MetamodApiBinding::detect(NonNull::from(self).cast()) }
			.expect("the mock is of a supported version");

		// SAFETY: The mock outlives the view, which the test uses on its own
		// thread.
		unsafe { binding.for_callback(self) }
	}
}

unsafe extern "C" fn detour_interface(this: *mut ISmmApi, _plugin: c_int) -> *mut c_void {
	// SAFETY: Every mock `ISmmApi` is a `MockSmm`.
	unsafe { (*this.cast::<MockSmm>()).khook }
}

unsafe extern "C" fn dev_api_versions(
	_this: *mut ISmmApi,
	major: *mut c_int,
	minor: *mut c_int,
	plugin_current: *mut c_int,
	plugin_minimum: *mut c_int,
) {
	// SAFETY: Metamod's caller passes its variables.
	unsafe {
		major.write(2);
		minor.write(1);
		plugin_current.write(18);
		plugin_minimum.write(18);
	}
}

unsafe extern "C" fn meta_factory(
	this: *mut ISmmApi,
	name: *const c_char,
	_return_code: *mut c_int,
	_plugin: *mut c_int,
) -> *mut c_void {
	// SAFETY: Every mock `ISmmApi` is a `MockSmm`, and callers pass a name.
	unsafe {
		match CStr::from_ptr(name) == MMIFACE_SOURCEHOOK {
			true => (*this.cast::<MockSmm>()).sourcehook,
			false => ptr::null_mut(),
		}
	}
}

unsafe extern "C" fn stable_api_versions(
	_this: *mut ISmmApi,
	major: *mut c_int,
	minor: *mut c_int,
	plugin_current: *mut c_int,
	plugin_minimum: *mut c_int,
) {
	// SAFETY: Metamod's caller passes its variables.
	unsafe {
		major.write(2);
		minor.write(0);
		plugin_current.write(16);
		plugin_minimum.write(14);
	}
}
