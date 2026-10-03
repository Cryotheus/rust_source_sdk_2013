//! Version-aware access to Metamod's C++ API object.

#[cfg(test)]
#[path = "tests/api.rs"]
mod tests;

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

	/// The engine's own `CreateInterface`, bypassing the wrapper through which
	/// Metamod lets plugins substitute interfaces.
	pub fn engine_factory(&self) -> CreateInterfaceFn {
		let prefix = self.binding.vtable.prefix();
		let get_engine_factory =
			unsafe { (&raw const (*prefix.as_ptr()).get_engine_factory).read() };

		unsafe { get_engine_factory(self.binding.this.as_ptr(), false) }
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

	fn find_interface<T>(&self, factory: CreateInterfaceFn, name: &CStr) -> Option<NonNull<T>> {
		let suffix = self.binding.vtable.suffix();
		let interface_match = unsafe { (&raw const (*suffix.as_ptr()).interface_match).read() };
		let factory = factory?;
		let interface = unsafe {
			interface_match(self.binding.this.as_ptr(), Some(factory), name.as_ptr(), -1)
		};

		NonNull::new(interface.cast())
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

	/// `ISmmAPI::MetaFactory`: one of Metamod's own interfaces by name, such as
	/// SourceHook's on the stable channel. The caller must validate and bind the
	/// returned opaque pointer before dereferencing it.
	pub fn meta_interface(&self, name: &CStr) -> Option<NonNull<c_void>> {
		let suffix = self.binding.vtable.suffix();
		let meta_factory = unsafe { (&raw const (*suffix.as_ptr()).meta_factory).read() };

		NonNull::new(unsafe {
			meta_factory(
				self.binding.this.as_ptr(),
				name.as_ptr(),
				std::ptr::null_mut(),
				std::ptr::null_mut(),
			)
		})
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

	/// The game server's own `CreateInterface`, bypassing the wrapper through
	/// which Metamod lets plugins substitute interfaces.
	pub fn server_factory(&self) -> CreateInterfaceFn {
		let prefix = self.binding.vtable.prefix();
		let get_server_factory =
			unsafe { (&raw const (*prefix.as_ptr()).get_server_factory).read() };

		unsafe { get_server_factory(self.binding.this.as_ptr(), false) }
	}

	pub fn source_engine_build(&self) -> c_int {
		let suffix = self.binding.vtable.suffix();
		let get_source_engine_build =
			unsafe { (&raw const (*suffix.as_ptr()).get_source_engine_build).read() };

		unsafe { get_source_engine_build(self.binding.this.as_ptr()) }
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

	pub fn supports(&self, feature: MetamodFeature) -> bool {
		matches!(
			(self.version(), feature),
			(
				MetamodVersion::Stable1226,
				MetamodFeature::SourceHookVersions
			) | (MetamodVersion::Dev1469, MetamodFeature::DetourInterface)
		)
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

	pub fn version(&self) -> MetamodVersion {
		self.binding.version()
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

/// A supported Metamod:Source, by the build its layouts were taken from.
///
/// Metamod reports only its API versions, which later builds share while
/// they keep the layouts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetamodVersion {
	/// 1.12, the stable channel, from build 1226.
	Stable1226,

	/// 2.0, the dev channel, from build 1469. Build 1472 has the same layouts.
	Dev1469,
}

impl MetamodVersion {
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

	/// The build the version's layouts were taken from, not necessarily the
	/// running one: Metamod does not report its build.
	pub fn build_number(&self) -> u32 {
		match self {
			Self::Stable1226 => 1226,
			Self::Dev1469 => 1469,
		}
	}

	pub fn plugin_api_version(self) -> c_int {
		match self {
			Self::Stable1226 => 16,
			Self::Dev1469 => 18,
		}
	}

	pub fn release_channel(&self) -> MetamodReleaseChannel {
		match self {
			Self::Stable1226 => MetamodReleaseChannel::Stable,
			Self::Dev1469 => MetamodReleaseChannel::Dev,
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
