//! Bounded, read-only copies from Source's virtual filesystem.
//!
//! The VFS chooses the search-path result. Returned bytes have no resolved
//! source provenance, atomic-read guarantee, or association with the world
//! currently loaded by the engine. A BSP read here does not establish which
//! external lump overrides the engine used.

use sdk_raw::vcall;
use std::ffi::{CStr, c_void};
use std::ptr::NonNull;

interface! {
	/// The callback-scoped `IBaseFileSystem` read interface.
	///
	/// Native calls must finish on the server's main thread in the callback
	/// that supplied this handle. Only the returned owned bytes may cross
	/// callback or thread boundaries.
	///
	/// ```compile_fail
	/// use source_sdk_2013::interfaces::BaseFileSystem;
	/// fn require_send<T: Send>() {}
	/// require_send::<BaseFileSystem<'static>>();
	/// ```
	///
	/// ```compile_fail
	/// use source_sdk_2013::interfaces::BaseFileSystem;
	/// fn require_sync<T: Sync>() {}
	/// require_sync::<BaseFileSystem<'static>>();
	/// ```
	///
	/// ```compile_fail
	/// use source_sdk_2013::{Server, interfaces::BaseFileSystem};
	/// fn escape(server: Server<'_>) -> BaseFileSystem<'static> {
	///     server.base_file_system().unwrap()
	/// }
	/// ```
	#[doc(alias("IBaseFileSystem"))]
	pub struct BaseFileSystem(sys::IBaseFileSystem) = Engine sdk_raw::interfaces::base_file_system::VERSION;
}

#[cfg(test)]
mod tests;

/// Budgets for one [`BaseFileSystem::read_file`] call.
///
/// These bound the output length and native read-call count. They do not
/// bound the VFS's internal allocations or how long a native call blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadLimits {
	/// Maximum output bytes, also capped at `i32::MAX` by the reader.
	pub bytes: usize,
	/// Maximum native `Read` calls, including unsuccessful calls.
	pub read_calls: usize,
}

impl Default for ReadLimits {
	/// Allows up to 64 MiB and 4096 native read calls.
	fn default() -> Self {
		Self {
			bytes: 64 * 1024 * 1024,
			read_calls: 4096,
		}
	}
}

/// A read refused by a budget or unsuccessful native operation.
///
/// Errors never contain partially read bytes. `Open` supplies no detailed
/// failure reason, `Size` supplies no separate error indicator, and `Close`
/// returns no acknowledgement. Native faults and process aborts are outside
/// this error contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReadError {
	/// The VFS returned a null file handle; the cause is unknown.
	#[error("the virtual filesystem could not open the requested file")]
	Open,
	/// The handle's advertised size exceeds the effective byte budget.
	#[error("file size {bytes} exceeds the byte limit {limit}")]
	ByteLimit {
		/// The size returned by the native handle-based `Size` call.
		bytes: usize,
		/// The smaller of the caller's budget and `i32::MAX`.
		limit: usize,
	},
	/// Fallible reservation for the initialized output storage failed.
	#[error("could not allocate storage for {bytes} file bytes")]
	Allocation {
		/// The requested output length.
		bytes: usize,
	},
	/// Another native read would exceed the call budget.
	#[error("the read-call limit {limit} was reached after {completed} bytes")]
	ReadLimit {
		/// The caller's read-call budget.
		limit: usize,
		/// The number of bytes already read, which are discarded.
		completed: usize,
	},
	/// A zero-byte read occurred before the advertised size was reached.
	#[error("the file ended after {completed} bytes; expected {expected}")]
	UnexpectedEof {
		/// The size sampled from the opened handle.
		expected: usize,
		/// The number of bytes already read, which are discarded.
		completed: usize,
	},
	/// The native read returned a negative byte count.
	#[error("the native read returned {returned} after {completed} bytes")]
	ReadFailed {
		/// The negative native return value.
		returned: i32,
		/// The number of bytes already read, which are discarded.
		completed: usize,
	},
	/// The native read reported more bytes than requested.
	#[error("the native read returned {returned} for a request of {requested} bytes")]
	InvalidReadCount {
		/// The size of the initialized writable destination given to `Read`.
		requested: usize,
		/// The impossible positive native return value.
		returned: i32,
	},
}

impl BaseFileSystem<'_> {
	/// Opens `path` in `rb` mode and copies the handle's sampled size into
	/// owned, initialized storage, subject to `limits`.
	///
	/// `path_id` is passed unchanged to `Open`; `None` passes null and lets
	/// the VFS search all its paths. The reader does not canonicalize paths,
	/// guess a game directory, choose alternative paths, or use the OS
	/// filesystem. The VFS may resolve an archive or loose file.
	///
	/// Reads use chunks of at most 64 KiB. Positive short reads are accepted;
	/// zero, negative, and oversized counts before completion are errors.
	/// A non-null handle whose size is zero returns an empty vector without
	/// reading. Each successful open receives exactly one `Close` attempt
	/// before returning, including budget and allocation failures.
	///
	/// Concurrent file changes can produce a mixed image or leave an appended
	/// suffix unread. This method neither detects those cases nor proves that
	/// a map file represents the loaded world. Hashes computed from the result
	/// identify only these returned bytes. Native calls may block, so callers
	/// should arrange an appropriate callback and release engine borrows first.
	pub fn read_file(
		self,
		path: &CStr,
		path_id: Option<&CStr>,
		limits: ReadLimits,
	) -> Result<Vec<u8>, ReadError> {
		self.read_file_with_allocator(path, path_id, limits, allocate)
	}

	fn read_file_with_allocator(
		self,
		path: &CStr,
		path_id: Option<&CStr>,
		limits: ReadLimits,
		allocate: impl FnOnce(usize) -> Result<Vec<u8>, ReadError>,
	) -> Result<Vec<u8>, ReadError> {
		// SAFETY: The interface is live on this callback's main thread. Both
		// C strings remain valid throughout the Open call.
		let handle = unsafe {
			vcall!(self.as_ptr() => IBaseFileSystem_Open(
				path.as_ptr(), c"rb".as_ptr(), path_id.map_or(std::ptr::null(), CStr::as_ptr)
			))
		};
		let handle = NonNull::new(handle).ok_or(ReadError::Open)?;
		// Install unique ownership before Size, budgets, or allocation can exit.
		let file = File { fs: self, handle };
		// SAFETY: Open returned a live handle owned by file. The generated
		// named field selects the handle overload on both supported ABIs.
		let length =
			unsafe { vcall!(self.as_ptr() => IBaseFileSystem_Size(file.handle.as_ptr())) } as usize;
		let limit = limits.bytes.min(i32::MAX as usize);
		if length > limit {
			return Err(ReadError::ByteLimit {
				bytes: length,
				limit,
			});
		}
		let mut bytes = allocate(length)?;
		let mut completed = 0;
		let mut calls = 0;
		while completed < length {
			if calls == limits.read_calls {
				return Err(ReadError::ReadLimit {
					limit: limits.read_calls,
					completed,
				});
			}
			calls += 1;
			let requested = (length - completed).min(64 * 1024);
			let destination = bytes[completed..completed + requested].as_mut_ptr();
			// SAFETY: The handle stays live and this initialized slice is writable
			// for requested bytes. The request fits c_int. Read must respect the
			// supplied destination capacity; validating its return cannot repair
			// an out-of-bounds write by a broken native implementation.
			let returned = unsafe {
				vcall!(self.as_ptr() => IBaseFileSystem_Read(
					destination.cast(), requested as i32, file.handle.as_ptr()
				))
			};
			if returned < 0 {
				return Err(ReadError::ReadFailed {
					returned,
					completed,
				});
			}
			if returned == 0 {
				return Err(ReadError::UnexpectedEof {
					expected: length,
					completed,
				});
			}
			if returned as usize > requested {
				return Err(ReadError::InvalidReadCount {
					requested,
					returned,
				});
			}
			completed += returned as usize;
		}
		Ok(bytes)
	}
}

fn allocate(length: usize) -> Result<Vec<u8>, ReadError> {
	let mut bytes = Vec::new();
	bytes
		.try_reserve_exact(length)
		.map_err(|_| ReadError::Allocation { bytes: length })?;
	// Capacity is already reserved; resize initializes every byte before FFI.
	bytes.resize(length, 0);
	Ok(bytes)
}

/// Private unique ownership; no native handle escapes read_file.
struct File<'s> {
	fs: BaseFileSystem<'s>,
	handle: NonNull<c_void>,
}

impl Drop for File<'_> {
	fn drop(&mut self) {
		// SAFETY: This guard owns the live handle. Its interface and handle are
		// used and closed within the same main-thread callback as Open.
		unsafe { vcall!(self.fs.as_ptr() => IBaseFileSystem_Close(self.handle.as_ptr())) };
	}
}
