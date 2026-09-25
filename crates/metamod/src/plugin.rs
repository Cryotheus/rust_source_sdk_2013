//! Reusable pieces of a Rust-backed `ISmmPlugin` implementation.

use crate::MetamodVersion;
pub use crate::sys::plugin::PluginCallbacks;
use crate::sys::plugin::{self as raw, PluginMetadata as RawMetadata};
use std::ffi::{CStr, c_char, c_void};
use std::marker::PhantomData;
use std::mem::MaybeUninit;
use std::ptr::NonNull;
use std::slice;

/// A callback-scoped view of a possibly uninitialized C output buffer.
pub struct ErrorBuffer<'callback> {
	buffer: Option<NonNull<MaybeUninit<c_char>>>,
	capacity: usize,
	_lifetime: PhantomData<&'callback mut [MaybeUninit<c_char>]>,
}

impl<'callback> ErrorBuffer<'callback> {
	/// Creates a view over a buffer supplied by C++.
	///
	/// # Safety
	///
	/// A non-null `buffer` must remain writable for `capacity` `c_char` values
	/// for the lifetime represented by `lifetime`. The memory may be
	/// uninitialized. No other Rust reference may access it while this view is
	/// used. Its allocation must satisfy [`slice::from_raw_parts_mut`]'s size
	/// requirements.
	pub unsafe fn from_raw<T: ?Sized>(
		buffer: *mut c_char,
		capacity: usize,
		_lifetime: &'callback T,
	) -> Self {
		Self {
			buffer: NonNull::new(buffer.cast()),
			capacity,
			_lifetime: PhantomData,
		}
	}

	pub const fn empty() -> Self {
		Self {
			buffer: None,
			capacity: 0,
			_lifetime: PhantomData,
		}
	}

	/// Writes a terminated, possibly truncated message into the buffer.
	pub fn write(&mut self, message: &CStr) {
		let Some(start) = self.buffer else { return };
		if self.capacity == 0 {
			return;
		}

		let bytes = message.to_bytes();
		let copy_length = bytes.len().min(self.capacity - 1);
		let buffer = unsafe { slice::from_raw_parts_mut(start.as_ptr(), self.capacity) };

		for (slot, byte) in buffer.iter_mut().zip(bytes).take(copy_length) {
			slot.write(*byte as c_char);
		}

		buffer[copy_length].write(0);
	}
}

/// Static callbacks and metadata supplied to the reusable C++ shell.
#[derive(Debug)]
pub struct PluginDescriptor {
	pub callbacks: PluginCallbacks,
	pub metadata: PluginMetadata,
}

impl PluginDescriptor {
	/// Selects the appropriate C++ plugin shell before Metamod calls `Load`.
	///
	/// # Safety
	///
	/// Calls must follow Metamod's single-threaded plugin-loader contract; do
	/// not reconfigure a shell concurrently with its callbacks.
	pub unsafe fn create_interface(&'static self, version: MetamodVersion) -> *mut c_void {
		unsafe {
			raw::cpp_metamod_plugin(
				version.plugin_api_version(),
				&raw const self.callbacks,
				self.metadata.as_raw(),
			)
		}
	}

	/// Compares the supplied plugin object with our selected C++ shell.
	pub fn is_selected_plugin(&self, version: MetamodVersion, plugin: NonNull<c_void>) -> bool {
		raw::cpp_metamod_plugin_for_version(version.plugin_api_version()) == plugin.as_ptr()
	}
}

/// Metadata returned by the C++ plugin shell. All strings must be static.
#[derive(Debug)]
pub struct PluginMetadata {
	pub author: &'static CStr,
	pub name: &'static CStr,
	pub description: &'static CStr,
	pub url: &'static CStr,
	pub license: &'static CStr,
	pub version: &'static CStr,
	pub date: &'static CStr,
	pub log_tag: &'static CStr,
}

impl PluginMetadata {
	fn as_raw(&self) -> RawMetadata {
		RawMetadata {
			author: self.author.as_ptr(),
			name: self.name.as_ptr(),
			description: self.description.as_ptr(),
			url: self.url.as_ptr(),
			license: self.license.as_ptr(),
			version: self.version.as_ptr(),
			date: self.date.as_ptr(),
			log_tag: self.log_tag.as_ptr(),
		}
	}
}

/// Generates [`PluginMetadata`] using cargo environment variables.
///
/// The `date` field is left blank.
#[macro_export]
macro_rules! plugin_meta {
	() => {
		$crate::PluginMetadata {
			author: $crate::__private_env_cstr!("CARGO_PKG_AUTHORS"),
			name: $crate::__private_env_cstr!("CARGO_PKG_NAME"),
			description: $crate::__private_env_cstr!("CARGO_PKG_DESCRIPTION"),
			url: const {
				let cstr = $crate::__private_env_cstr!("CARGO_PKG_HOMEPAGE");

				if cstr.is_empty() {
					$crate::__private_env_cstr!("CARGO_PKG_REPOSITORY")
				} else {
					cstr
				}
			},
			license: $crate::__private_env_cstr!("CARGO_PKG_LICENSE"),
			version: $crate::__private_env_cstr!("CARGO_PKG_VERSION"),
			date: c"",
			log_tag: $crate::__private_env_cstr!("CARGO_PKG_NAME"),
		}
	};
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn error_buffer_initializes_and_terminates_uninitialized_memory() {
		let mut storage = [MaybeUninit::<c_char>::uninit(); 5];
		let lifetime = ();
		let mut buffer =
			unsafe { ErrorBuffer::from_raw(storage.as_mut_ptr().cast(), storage.len(), &lifetime) };

		buffer.write(c"longer than the buffer");

		let initialized = storage.map(|byte| unsafe { byte.assume_init() as u8 });

		assert_eq!(&initialized, b"long\0");
	}
}
