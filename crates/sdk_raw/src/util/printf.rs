//! Capture of output that the engine formats with `printf`-style functions.
//!
//! Some engine and game methods print through a callback they are given, such
//! as `IServerGameDLL::Status`. [`capture_printf`] lends them one that
//! formats each call with the C runtime and keeps the text.

use crate::util::cstr::cstring_from_buffer;
use std::cell::RefCell;
use std::ffi::{CString, VaList, c_char, c_int};

/// A `printf`-style output callback, which the engine calls with a format and
/// its arguments.
///
/// The generated bindings take it as
/// `Option<unsafe extern "C" fn(*const c_char, ...)>`, such as
/// `IServerGameDLL::Status`'s parameter.
pub type PrintfCallback = unsafe extern "C" fn(format: *const c_char, ...);

/// The size of the buffer each call of a [`capture_printf`] callback is
/// formatted into, terminator included, so the longest text one call records
/// is a byte shorter.
pub const CAPTURE_LINE_CAPACITY: usize = 1024;

thread_local! {
	/// Output of the innermost [`capture_printf`] in progress on this thread,
	/// or `None` outside of one.
	static CAPTURED: RefCell<Option<Vec<u8>>> = const { RefCell::new(None) };
}

#[cfg(test)]
#[path = "../tests/util/printf.rs"]
mod tests;

/// Restores the capture that was in progress when a [`capture_printf`] began,
/// and returns what the inner one captured.
struct CaptureGuard(Option<Option<Vec<u8>>>);

impl CaptureGuard {
	/// Ends the capture, returning its output.
	fn finish(mut self) -> Vec<u8> {
		let previous = self.0.take().unwrap_or_default();

		CAPTURED.replace(previous).unwrap_or_default()
	}
}

impl Drop for CaptureGuard {
	fn drop(&mut self) {
		// Only reached without `finish`, when the closure unwound.
		if let Some(previous) = self.0.take() {
			CAPTURED.set(previous);
		}
	}
}

unsafe extern "C" {
	/// The C library's `vsnprintf`.
	#[cfg(not(target_os = "windows"))]
	#[link_name = "vsnprintf"]
	fn c_vsnprintf(
		buffer: *mut c_char,
		count: usize,
		format: *const c_char,
		arguments: VaList<'_>,
	) -> c_int;

	/// The UCRT's formatter, which its inline `vsnprintf` calls.
	#[cfg(target_os = "windows")]
	fn __stdio_common_vsprintf(
		options: u64,
		buffer: *mut c_char,
		count: usize,
		format: *const c_char,
		locale: *mut std::ffi::c_void,
		arguments: VaList<'_>,
	) -> c_int;
}

/// The callback [`capture_printf`] lends, appending to [`CAPTURED`].
unsafe extern "C" fn append_captured(format: *const c_char, arguments: ...) {
	if format.is_null() {
		return;
	}

	let mut line = [0 as c_char; CAPTURE_LINE_CAPACITY];

	// SAFETY: The engine passes a `printf` format and matching arguments.
	if unsafe { vsnprintf(&mut line, format, arguments) } < 0 {
		return;
	}

	let line = cstring_from_buffer(&line);

	CAPTURED.with_borrow_mut(|output| {
		if let Some(output) = output {
			output.extend_from_slice(line.as_bytes());
		}
	});
}

/// Runs `f` with a [`PrintfCallback`] that formats each call and appends the
/// text to the output, which it returns alongside `f`'s result.
///
/// Each call's text is truncated to [`CAPTURE_LINE_CAPACITY`] minus one
/// bytes, and ends at its first NUL, so the output never contains one. Calls
/// with a null format, or that the C runtime fails to format, add nothing.
///
/// The output belongs to the innermost capture in progress on the calling
/// thread: a capture nested in `f` collects its own, and the callback adds
/// nothing once every capture has ended.
pub fn capture_printf<R>(f: impl FnOnce(PrintfCallback) -> R) -> (R, CString) {
	let guard = CaptureGuard(Some(CAPTURED.replace(Some(Vec::new()))));
	let result = f(append_captured);
	let output = guard.finish();

	// SAFETY: `append_captured` only appends the bytes before each terminator.
	(result, unsafe { CString::from_vec_unchecked(output) })
}

/// Formats a `printf` call into `buffer` as C99's `vsnprintf` does: the text
/// is truncated to fit, and terminated if the buffer is not empty.
///
/// Returns the length the whole text would have, without its terminator, or a
/// negative value if the C runtime fails to format it.
///
/// # Safety
///
/// `format` must point to a NUL-terminated `printf` format, and `arguments`
/// must be the arguments it reads, of the types it expects.
pub unsafe fn vsnprintf(
	buffer: &mut [c_char],
	format: *const c_char,
	arguments: VaList<'_>,
) -> c_int {
	cfg_select! {
		target_os = "windows" => {
			/// `_CRT_INTERNAL_PRINTF_STANDARD_SNPRINTF_BEHAVIOR`, which makes the
			/// formatter behave as C99's `vsnprintf`.
			const STANDARD_SNPRINTF_BEHAVIOR: u64 = 1 << 1;

			// SAFETY: The caller upholds the contract, and the length is passed.
			unsafe {
				__stdio_common_vsprintf(
					STANDARD_SNPRINTF_BEHAVIOR,
					buffer.as_mut_ptr(),
					buffer.len(),
					format,
					std::ptr::null_mut(),
					arguments,
				)
			}
		}

		_ => {
			// SAFETY: The caller upholds the contract, and the length is passed.
			unsafe { c_vsnprintf(buffer.as_mut_ptr(), buffer.len(), format, arguments) }
		}
	}
}
