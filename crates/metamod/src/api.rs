//! Version-aware access to Metamod's C++ API object.

use crate::sys::api::{
	CreateInterfaceFn, ISmmApi, ISmmApiVtable1226, ISmmApiVtable1469, ISmmApiVtablePrefix,
	ISmmApiVtableSuffix, MetamodVersionInfo1226, MetamodVersionInfo1469, MetamodVersionInfoPrefix,
};
use std::error::Error;
use std::ffi::{CStr, c_int, c_void};
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::marker::PhantomData;
use std::ptr::NonNull;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoaderVersionInfo {
	pub version: MetamodVersion,
	pub source_engine: c_int,
}

impl LoaderVersionInfo {
	/// Reads only fields belonging to the version selected by the shared prefix.
	///
	/// # Safety
	///
	/// `info` must point to a live `MetamodVersionInfo` supplied by Metamod.
	pub unsafe fn from_raw(info: *const c_void) -> Option<Self> {
		let info = NonNull::new(info.cast_mut())?.cast::<MetamodVersionInfoPrefix>();
		let prefix = unsafe { info.as_ptr().read() };

		let (version, pl_min, pl_max, source_engine) = match (prefix.api_major, prefix.api_minor) {
			(2, 0) => {
				let info = info.cast::<MetamodVersionInfo1226>();

				(
					MetamodVersion::Stable1226,
					unsafe { (&raw const (*info.as_ptr()).pl_min).read() },
					unsafe { (&raw const (*info.as_ptr()).pl_max).read() },
					unsafe { (&raw const (*info.as_ptr()).source_engine).read() },
				)
			}

			(2, 1) => {
				let info = info.cast::<MetamodVersionInfo1469>();

				(
					MetamodVersion::Dev1469,
					unsafe { (&raw const (*info.as_ptr()).pl_min).read() },
					unsafe { (&raw const (*info.as_ptr()).pl_max).read() },
					unsafe { (&raw const (*info.as_ptr()).source_engine).read() },
				)
			}

			_ => return None,
		};

		let expected = match version {
			MetamodVersion::Stable1226 => (14, 16),
			MetamodVersion::Dev1469 => (18, 18),
		};
		(pl_min == expected.0 && pl_max == expected.1).then_some(Self {
			version,
			source_engine,
		})
	}
}

/// Version-agnostic access to a live C++ `ISmmAPI` object during a callback.
#[derive(Debug, Clone, Copy)]
pub struct MetamodApi<'callback> {
	binding: MetamodApiBinding,
	_lifetime: PhantomData<&'callback ()>,
	_not_send_or_sync: PhantomData<*mut ()>,
}

impl MetamodApi<'_> {
	pub fn version(&self) -> MetamodVersion {
		self.binding.version()
	}

	pub fn supports(&self, feature: MetamodFeature) -> bool {
		matches!(
			(self.version(), feature),
			(
				MetamodVersion::Stable1226,
				MetamodFeature::SourceHookVersions
			) | (MetamodVersion::Dev1469, MetamodFeature::DetourInterface)
		)
	}

	/// Calls Metamod's logger without creating a reference to its C++ object.
	pub fn log_cstr(&self, plugin: NonNull<c_void>, message: &CStr) {
		let prefix = self.binding.vtable.prefix();
		let log_message = unsafe { (&raw const (*prefix.as_ptr()).log_message).read() };

		unsafe {
			log_message(
				self.binding.this.as_ptr(),
				plugin.as_ptr(),
				c"%s".as_ptr(),
				message.as_ptr(),
			)
		};
	}

	pub fn source_engine_build(&self) -> c_int {
		let suffix = self.binding.vtable.suffix();
		let get_source_engine_build =
			unsafe { (&raw const (*suffix.as_ptr()).get_source_engine_build).read() };

		unsafe { get_source_engine_build(self.binding.this.as_ptr()) }
	}

	/// The engine's own `CreateInterface`, bypassing the wrapper through which
	/// Metamod lets plugins substitute interfaces.
	pub fn engine_factory(&self) -> CreateInterfaceFn {
		let prefix = self.binding.vtable.prefix();
		let get_engine_factory =
			unsafe { (&raw const (*prefix.as_ptr()).get_engine_factory).read() };

		unsafe { get_engine_factory(self.binding.this.as_ptr(), false) }
	}

	/// The game server's own `CreateInterface`, bypassing the wrapper through
	/// which Metamod lets plugins substitute interfaces.
	pub fn server_factory(&self) -> CreateInterfaceFn {
		let prefix = self.binding.vtable.prefix();
		let get_server_factory =
			unsafe { (&raw const (*prefix.as_ptr()).get_server_factory).read() };

		unsafe { get_server_factory(self.binding.this.as_ptr(), false) }
	}

	/// Looks up an engine interface. The caller must validate and bind the
	/// returned opaque pointer before dereferencing it.
	///
	/// Metamod falls back to newer versions when the exact one is missing,
	/// so the pointer may not match the requested version's layout.
	pub fn find_engine_interface<T>(&self, name: &CStr) -> Option<NonNull<T>> {
		let prefix = self.binding.vtable.prefix();
		let get_engine_factory =
			unsafe { (&raw const (*prefix.as_ptr()).get_engine_factory).read() };
		let factory = unsafe { get_engine_factory(self.binding.this.as_ptr(), true) };
		self.find_interface(factory, name)
	}

	/// Looks up a game-server interface. The caller must validate and bind the
	/// returned opaque pointer before dereferencing it.
	///
	/// As with [`Self::find_engine_interface`], a newer version may be returned.
	pub fn find_server_interface<T>(&self, name: &CStr) -> Option<NonNull<T>> {
		let prefix = self.binding.vtable.prefix();
		let get_server_factory =
			unsafe { (&raw const (*prefix.as_ptr()).get_server_factory).read() };
		let factory = unsafe { get_server_factory(self.binding.this.as_ptr(), true) };
		self.find_interface(factory, name)
	}

	fn find_interface<T>(&self, factory: CreateInterfaceFn, name: &CStr) -> Option<NonNull<T>> {
		let suffix = self.binding.vtable.suffix();
		let interface_match = unsafe { (&raw const (*suffix.as_ptr()).interface_match).read() };
		let factory = factory?;
		let interface = unsafe {
			interface_match(self.binding.this.as_ptr(), Some(factory), name.as_ptr(), -1)
		};

		NonNull::new(interface.cast())
	}

	/// `ISmmAPI::RegisterConCommandBase`, which links a command or variable and
	/// tracks it for `plugin`. Metamod's result is always `true`.
	///
	/// # Safety
	///
	/// `plugin` must be the plugin object Metamod loaded, and `command` a live
	/// `ConCommandBase`, which Metamod passes to the engine's `ICvar`.
	#[cfg(feature = "sdk")]
	pub(crate) unsafe fn register_con_command_base(
		&self,
		plugin: NonNull<c_void>,
		command: NonNull<c_void>,
	) -> bool {
		let prefix = self.binding.vtable.prefix();
		let register = unsafe { (&raw const (*prefix.as_ptr()).register_con_command_base).read() };

		unsafe {
			register(
				self.binding.this.as_ptr(),
				plugin.as_ptr(),
				command.as_ptr(),
			)
		}
	}

	/// `ISmmAPI::UnregisterConCommandBase`, the reverse of
	/// [`Self::register_con_command_base`].
	///
	/// # Safety
	///
	/// As for [`Self::register_con_command_base`].
	#[cfg(feature = "sdk")]
	pub(crate) unsafe fn unregister_con_command_base(
		&self,
		plugin: NonNull<c_void>,
		command: NonNull<c_void>,
	) {
		let prefix = self.binding.vtable.prefix();
		let unregister =
			unsafe { (&raw const (*prefix.as_ptr()).unregister_con_command_base).read() };

		unsafe {
			unregister(
				self.binding.this.as_ptr(),
				plugin.as_ptr(),
				command.as_ptr(),
			)
		};
	}

	/// Returns `None` on dev builds, which removed `GetShVersions`.
	pub fn source_hook_versions(&self) -> Option<SourceHookVersions> {
		let VersionedVtable::Stable1226(vtable) = self.binding.vtable else {
			return None;
		};

		let get_sh_versions = unsafe { (&raw const (*vtable.as_ptr()).get_sh_versions).read() };
		let (mut interface, mut implementation) = (0, 0);

		unsafe {
			get_sh_versions(
				self.binding.this.as_ptr(),
				&mut interface,
				&mut implementation,
			)
		};

		Some(SourceHookVersions {
			interface,
			implementation,
		})
	}

	/// Returns the dev-build detour interface, or an explicit unsupported error.
	/// The returned pointer is opaque and must not be dereferenced without its
	/// own interface contract.
	pub fn detour_interface(
		&self,
		plugin_id: c_int,
	) -> Result<Option<NonNull<c_void>>, UnsupportedFeature> {
		let VersionedVtable::Dev1469(vtable) = self.binding.vtable else {
			return Err(UnsupportedFeature(MetamodFeature::DetourInterface));
		};

		let get_detour_interface =
			unsafe { (&raw const (*vtable.as_ptr()).get_detour_interface).read() };

		Ok(NonNull::new(unsafe {
			get_detour_interface(self.binding.this.as_ptr(), plugin_id)
		}))
	}
}

/// An ABI-selected raw handle suitable for storage between callbacks.
///
/// This does not promise the C++ object remains live. Convert it to a
/// callback-scoped [`MetamodApi`] only while Metamod owns that object.
#[derive(Debug, Clone, Copy)]
pub struct MetamodApiBinding {
	this: NonNull<ISmmApi>,
	vtable: VersionedVtable,
}

impl MetamodApiBinding {
	/// Detects the ABI through the common `ISmmAPI` vtable prefix.
	///
	/// # Safety
	///
	/// `this` must identify a live Metamod API object whose common prefix
	/// matches one of the supported 64-bit builds.
	pub unsafe fn detect(this: NonNull<c_void>) -> Option<Self> {
		let this = this.cast::<ISmmApi>();
		let prefix =
			NonNull::new(unsafe { (&raw const (*this.as_ptr()).vtable).read().cast_mut() })?;
		let get_api_versions = unsafe { (&raw const (*prefix.as_ptr()).get_api_versions).read() };
		let (mut major, mut minor, mut plugin_current, mut plugin_minimum) = (0, 0, 0, 0);
		unsafe {
			get_api_versions(
				this.as_ptr(),
				&mut major,
				&mut minor,
				&mut plugin_current,
				&mut plugin_minimum,
			)
		};

		let vtable = match MetamodVersion::from_api_versions(
			major,
			minor,
			plugin_current,
			plugin_minimum,
		)? {
			MetamodVersion::Stable1226 => VersionedVtable::Stable1226(prefix.cast()),
			MetamodVersion::Dev1469 => VersionedVtable::Dev1469(prefix.cast()),
		};
		Some(Self { this, vtable })
	}

	/// Ties this raw binding to a live Metamod callback.
	///
	/// # Safety
	///
	/// The C++ API object must remain live throughout `lifetime`, and all
	/// calls through the returned view must occur on Metamod's server thread.
	#[allow(
		clippy::needless_lifetimes,
		reason = "Prevents mistakes if function signature changes `self` to `&self`"
	)]
	pub unsafe fn for_callback<'callback, T: ?Sized>(
		self,
		_lifetime: &'callback T,
	) -> MetamodApi<'callback> {
		MetamodApi {
			binding: self,
			_lifetime: PhantomData,
			_not_send_or_sync: PhantomData,
		}
	}

	pub fn version(&self) -> MetamodVersion {
		self.vtable.version()
	}
}

/// A feature whose availability differs between supported Metamod ABIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetamodFeature {
	SourceHookVersions,
	DetourInterface,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetamodReleaseChannel {
	Dev,
	Stable,
}

impl Display for MetamodReleaseChannel {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		match self {
			Self::Dev => f.write_str("dev"),
			Self::Stable => f.write_str("stable"),
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetamodVersion {
	Stable1226,
	Dev1469,
}

impl MetamodVersion {
	pub fn build_number(&self) -> u32 {
		match self {
			Self::Stable1226 => 1226,
			Self::Dev1469 => 1469,
		}
	}

	pub fn release_channel(&self) -> MetamodReleaseChannel {
		match self {
			Self::Stable1226 => MetamodReleaseChannel::Stable,
			Self::Dev1469 => MetamodReleaseChannel::Dev,
		}
	}

	pub fn plugin_api_version(self) -> c_int {
		match self {
			Self::Stable1226 => 16,
			Self::Dev1469 => 18,
		}
	}

	fn from_api_versions(
		major: c_int,
		minor: c_int,
		plugin_current: c_int,
		plugin_minimum: c_int,
	) -> Option<Self> {
		match (major, minor, plugin_current, plugin_minimum) {
			(2, 0, 16, 14) => Some(Self::Stable1226),
			(2, 1, 18, 18) => Some(Self::Dev1469),
			_ => None,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceHookVersions {
	pub interface: c_int,
	pub implementation: c_int,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedFeature(pub MetamodFeature);

impl Display for UnsupportedFeature {
	fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
		write!(
			f,
			"Metamod feature {:?} is unavailable on this build",
			self.0
		)
	}
}

impl Error for UnsupportedFeature {}

#[derive(Debug, Clone, Copy)]
enum VersionedVtable {
	Stable1226(NonNull<ISmmApiVtable1226>),
	Dev1469(NonNull<ISmmApiVtable1469>),
}

impl VersionedVtable {
	fn prefix(self) -> NonNull<ISmmApiVtablePrefix> {
		match self {
			Self::Stable1226(vtable) => vtable.cast(),
			Self::Dev1469(vtable) => vtable.cast(),
		}
	}

	fn suffix(self) -> NonNull<ISmmApiVtableSuffix> {
		let suffix = match self {
			Self::Stable1226(vtable) => unsafe { &raw const (*vtable.as_ptr()).suffix },
			Self::Dev1469(vtable) => unsafe { &raw const (*vtable.as_ptr()).suffix },
		};

		// Both supported layouts contain an initialized suffix at this address.
		unsafe { NonNull::new_unchecked(suffix.cast_mut()) }
	}

	fn version(&self) -> MetamodVersion {
		match self {
			Self::Stable1226(..) => MetamodVersion::Stable1226,
			Self::Dev1469(..) => MetamodVersion::Dev1469,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::sys::api::SOURCE_ENGINE_TF2;
	use std::mem::MaybeUninit;
	use std::ptr::{null, null_mut};

	unsafe extern "C" fn stable_api_versions(
		_this: *mut ISmmApi,
		major: *mut c_int,
		minor: *mut c_int,
		plugin_current: *mut c_int,
		plugin_minimum: *mut c_int,
	) {
		unsafe {
			major.write(2);
			minor.write(0);
			plugin_current.write(16);
			plugin_minimum.write(14);
		}
	}

	unsafe extern "C" fn dev_api_versions(
		_this: *mut ISmmApi,
		major: *mut c_int,
		minor: *mut c_int,
		plugin_current: *mut c_int,
		plugin_minimum: *mut c_int,
	) {
		unsafe {
			major.write(2);
			minor.write(1);
			plugin_current.write(18);
			plugin_minimum.write(18);
		}
	}

	unsafe extern "C" fn unknown_api_versions(
		_this: *mut ISmmApi,
		major: *mut c_int,
		minor: *mut c_int,
		plugin_current: *mut c_int,
		plugin_minimum: *mut c_int,
	) {
		unsafe {
			major.write(3);
			minor.write(0);
			plugin_current.write(19);
			plugin_minimum.write(19);
		}
	}

	unsafe extern "C" fn create_interface(
		_name: *const std::ffi::c_char,
		_return_code: *mut c_int,
	) -> *mut c_void {
		NonNull::<c_void>::dangling().as_ptr()
	}

	thread_local! {
		static LAST_SYNTHETIC: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
	}

	unsafe extern "C" fn engine_factory(_this: *mut ISmmApi, synthetic: bool) -> CreateInterfaceFn {
		LAST_SYNTHETIC.set(Some(synthetic));
		Some(create_interface)
	}

	unsafe extern "C" fn create_server_interface(
		_name: *const std::ffi::c_char,
		_return_code: *mut c_int,
	) -> *mut c_void {
		2_usize as *mut c_void
	}

	unsafe extern "C" fn server_factory(_this: *mut ISmmApi, synthetic: bool) -> CreateInterfaceFn {
		LAST_SYNTHETIC.set(Some(synthetic));
		Some(create_server_interface)
	}

	unsafe extern "C" fn interface_match(
		_this: *mut ISmmApi,
		factory: CreateInterfaceFn,
		name: *const std::ffi::c_char,
		_minimum: c_int,
	) -> *mut c_void {
		let Some(factory) = factory else {
			return null_mut();
		};
		unsafe { factory(name, null_mut()) }
	}

	unsafe extern "C" fn stable_engine_build(_this: *mut ISmmApi) -> c_int {
		SOURCE_ENGINE_TF2
	}

	unsafe extern "C" fn dev_engine_build(_this: *mut ISmmApi) -> c_int {
		19
	}

	unsafe extern "C" fn source_hook_versions(
		_this: *mut ISmmApi,
		interface: *mut c_int,
		implementation: *mut c_int,
	) {
		unsafe {
			interface.write(7);
			implementation.write(8);
		}
	}

	unsafe extern "C" fn detour_interface(_this: *mut ISmmApi, _plugin_id: c_int) -> *mut c_void {
		NonNull::<c_void>::dangling().as_ptr()
	}

	fn mock_vtable(version: MetamodVersion) -> [MaybeUninit<*const ()>; 34] {
		let mut slots = [MaybeUninit::uninit(); 34];

		slots[1].write(engine_factory as *const ());
		slots[4].write(server_factory as *const ());

		match version {
			MetamodVersion::Stable1226 => {
				slots[10].write(stable_api_versions as *const ());
				slots[11].write(source_hook_versions as *const ());
				slots[19].write(interface_match as *const ());
				slots[26].write(stable_engine_build as *const ());
			}

			MetamodVersion::Dev1469 => {
				slots[10].write(dev_api_versions as *const ());
				slots[18].write(interface_match as *const ());
				slots[25].write(dev_engine_build as *const ());
				slots[33].write(detour_interface as *const ());
			}
		}

		slots
	}

	#[test]
	fn dispatches_common_and_optional_methods_for_both_abis() {
		for version in [MetamodVersion::Stable1226, MetamodVersion::Dev1469] {
			let vtable = mock_vtable(version);

			let mut object = ISmmApi {
				vtable: vtable.as_ptr().cast(),
			};

			let binding =
				unsafe { MetamodApiBinding::detect(NonNull::from(&mut object).cast()) }.unwrap();
			let api = unsafe { binding.for_callback(&object) };

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

	#[test]
	fn rejects_unrecognized_api_version_before_version_specific_vtable_access() {
		let mut vtable = [MaybeUninit::<*const ()>::uninit(); 11];

		vtable[10].write(unknown_api_versions as *const ());

		let mut object = ISmmApi {
			vtable: vtable.as_ptr().cast(),
		};

		assert!(unsafe { MetamodApiBinding::detect(NonNull::from(&mut object).cast()) }.is_none());
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
			unsafe { LoaderVersionInfo::from_raw((&raw const stable).cast()) },
			Some(LoaderVersionInfo {
				version: MetamodVersion::Stable1226,
				source_engine: SOURCE_ENGINE_TF2
			})
		);

		assert_eq!(
			unsafe { LoaderVersionInfo::from_raw((&raw const dev).cast()) },
			Some(LoaderVersionInfo {
				version: MetamodVersion::Dev1469,
				source_engine: 19
			})
		);

		assert!(unsafe { LoaderVersionInfo::from_raw(null()) }.is_none());
	}
}
