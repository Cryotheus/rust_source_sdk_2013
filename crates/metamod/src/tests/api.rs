//! Tests of `crate::api`: Metamod's API and loader information, through each
//! version's C++ layouts.

use super::*;
use crate::sys::api::SOURCE_ENGINE_TF2;
use crate::test_support::smm::MockSmm;
use std::cell::Cell;
use std::ffi::c_char;
use std::mem::MaybeUninit;
use std::ptr::{null, null_mut};

thread_local! {
	/// Whether the last factory requested was Metamod's synthetic wrapper.
	static LAST_SYNTHETIC: Cell<Option<bool>> = const { Cell::new(None) };
}

unsafe extern "C" fn create_interface(
	_name: *const c_char,
	_return_code: *mut c_int,
) -> *mut c_void {
	NonNull::<c_void>::dangling().as_ptr()
}

unsafe extern "C" fn create_server_interface(
	_name: *const c_char,
	_return_code: *mut c_int,
) -> *mut c_void {
	2_usize as *mut c_void
}

unsafe extern "C" fn dev_engine_build(_this: *mut ISmmApi) -> c_int {
	19
}

#[test]
fn dispatches_common_and_optional_methods_for_both_abis() {
	for version in [MetamodVersion::Stable1226, MetamodVersion::Dev1469] {
		let methods: &[(usize, *const ())] = match version {
			MetamodVersion::Stable1226 => &[
				(1, engine_factory as *const ()),
				(4, server_factory as *const ()),
				(11, source_hook_versions as *const ()),
				(19, interface_match as *const ()),
				(26, stable_engine_build as *const ()),
			],

			MetamodVersion::Dev1469 => &[
				(1, engine_factory as *const ()),
				(4, server_factory as *const ()),
				(18, interface_match as *const ()),
				(25, dev_engine_build as *const ()),
			],
		};

		let mut smm = MockSmm::new(version, methods);

		smm.khook = NonNull::<c_void>::dangling().as_ptr();

		let api = smm.api();

		assert_eq!(api.version(), version);

		assert_eq!(
			api.source_engine_build(),
			if version == MetamodVersion::Stable1226 {
				SOURCE_ENGINE_TF2
			} else {
				19
			}
		);

		assert_eq!(
			api.find_engine_interface::<c_void>(c"TestInterface001"),
			Some(NonNull::dangling())
		);
		assert_eq!(
			api.find_server_interface::<c_void>(c"VSERVERTOOLS003"),
			NonNull::new(2_usize as *mut c_void)
		);

		// The original factories are requested, not Metamod's synthetic wrappers.
		assert_eq!(
			api.engine_factory().map(|factory| factory as usize),
			Some(create_interface as *const () as usize)
		);
		assert_eq!(LAST_SYNTHETIC.get(), Some(false));
		assert_eq!(
			api.server_factory().map(|factory| factory as usize),
			Some(create_server_interface as *const () as usize)
		);
		assert_eq!(LAST_SYNTHETIC.get(), Some(false));

		match version {
			MetamodVersion::Stable1226 => {
				assert!(api.supports(MetamodFeature::SourceHookVersions));
				assert!(!api.supports(MetamodFeature::DetourInterface));

				assert_eq!(
					api.source_hook_versions(),
					Some(SourceHookVersions {
						interface: 7,
						implementation: 8
					})
				);

				assert_eq!(
					api.detour_interface(1),
					Err(UnsupportedFeature(MetamodFeature::DetourInterface))
				);
			}

			MetamodVersion::Dev1469 => {
				assert!(!api.supports(MetamodFeature::SourceHookVersions));
				assert!(api.supports(MetamodFeature::DetourInterface));
				assert_eq!(api.source_hook_versions(), None);
				assert_eq!(api.detour_interface(1), Ok(Some(NonNull::dangling())));
			}
		}
	}
}

unsafe extern "C" fn engine_factory(_this: *mut ISmmApi, synthetic: bool) -> CreateInterfaceFn {
	LAST_SYNTHETIC.set(Some(synthetic));
	Some(create_interface)
}

unsafe extern "C" fn interface_match(
	_this: *mut ISmmApi,
	factory: CreateInterfaceFn,
	name: *const c_char,
	_minimum: c_int,
) -> *mut c_void {
	let Some(factory) = factory else {
		return null_mut();
	};

	// SAFETY: Metamod passes the factory a name, and it may take a null return
	// code.
	unsafe { factory(name, null_mut()) }
}

#[test]
fn reads_both_loader_layouts_without_assuming_an_engine() {
	let stable = MetamodVersionInfo1226 {
		prefix: MetamodVersionInfoPrefix {
			api_major: 2,
			api_minor: 0,
		},
		sh_iface: 0,
		sh_impl: 0,
		pl_min: 14,
		pl_max: 16,
		source_engine: SOURCE_ENGINE_TF2,
		game_dir: null(),
	};

	let dev = MetamodVersionInfo1469 {
		prefix: MetamodVersionInfoPrefix {
			api_major: 2,
			api_minor: 1,
		},
		pl_min: 18,
		pl_max: 18,
		source_engine: 19,
		game_dir: null(),
	};

	assert_eq!(
		// SAFETY: The information of the version its prefix names, as Metamod's
		// loader supplies it.
		unsafe { LoaderVersionInfo::from_raw((&raw const stable).cast()) },
		Some(LoaderVersionInfo {
			version: MetamodVersion::Stable1226,
			source_engine: SOURCE_ENGINE_TF2
		})
	);

	assert_eq!(
		// SAFETY: As above.
		unsafe { LoaderVersionInfo::from_raw((&raw const dev).cast()) },
		Some(LoaderVersionInfo {
			version: MetamodVersion::Dev1469,
			source_engine: 19
		})
	);

	// SAFETY: Null is refused before it is read.
	assert!(unsafe { LoaderVersionInfo::from_raw(null()) }.is_none());
}

#[test]
fn rejects_unrecognized_api_version_before_version_specific_vtable_access() {
	// Only the slots both versions share, up to `GetApiVersions`.
	let mut vtable = [MaybeUninit::<*const ()>::uninit(); 11];

	vtable[10].write(unknown_api_versions as *const ());

	let mut object = ISmmApi {
		vtable: vtable.as_ptr().cast(),
	};

	// SAFETY: The object's vtable has `GetApiVersions`, the only method the
	// detection calls before it knows the version.
	assert!(unsafe { MetamodApiBinding::detect(NonNull::from(&mut object).cast()) }.is_none());
}

unsafe extern "C" fn server_factory(_this: *mut ISmmApi, synthetic: bool) -> CreateInterfaceFn {
	LAST_SYNTHETIC.set(Some(synthetic));
	Some(create_server_interface)
}

unsafe extern "C" fn source_hook_versions(
	_this: *mut ISmmApi,
	interface: *mut c_int,
	implementation: *mut c_int,
) {
	// SAFETY: Metamod's caller passes its variables.
	unsafe {
		interface.write(7);
		implementation.write(8);
	}
}

unsafe extern "C" fn stable_engine_build(_this: *mut ISmmApi) -> c_int {
	SOURCE_ENGINE_TF2
}

unsafe extern "C" fn unknown_api_versions(
	_this: *mut ISmmApi,
	major: *mut c_int,
	minor: *mut c_int,
	plugin_current: *mut c_int,
	plugin_minimum: *mut c_int,
) {
	// SAFETY: Metamod's caller passes its variables.
	unsafe {
		major.write(3);
		minor.write(0);
		plugin_current.write(19);
		plugin_minimum.write(19);
	}
}
