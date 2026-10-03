//! The error buffer Metamod passes a plugin's `Load`, which may be
//! uninitialized, written as C copies a string into it.

use metamod_source::ErrorBuffer;
use std::ffi::c_char;
use std::mem::MaybeUninit;

#[test]
fn error_buffer_initializes_and_terminates_uninitialized_memory() {
	let mut storage = [MaybeUninit::<c_char>::uninit(); 5];
	let lifetime = ();

	// SAFETY: The storage is writable for its length, outlives the buffer, and
	// is accessed only through it until it is written.
	let mut buffer =
		unsafe { ErrorBuffer::from_raw(storage.as_mut_ptr().cast(), storage.len(), &lifetime) };

	buffer.write(c"longer than the buffer");

	// SAFETY: The message filled the storage, its last byte with the
	// terminator.
	let initialized = storage.map(|byte| unsafe { byte.assume_init() as u8 });

	assert_eq!(&initialized, b"long\0");
}
