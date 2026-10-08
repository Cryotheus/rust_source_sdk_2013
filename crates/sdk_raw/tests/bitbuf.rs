//! Tests of the engine's bit buffers, `bf_write` and `bf_read`.

use source_sdk_2013_raw::bitbuf::{BfRead, BfWrite};
use std::ptr::{NonNull, null};

#[test]
fn engine_buffers_access_only_whole_words() {
	// Two words back the buffer, but it claims only five bytes, so its
	// second word is not whole and must never be accessed.
	let mut storage = [0, 0xDEAD_BEEF];
	let mut buffer = BfWrite::empty(&mut storage);
	let this = NonNull::from(&mut buffer);

	// SAFETY: The buffer describes `storage`, which only it accesses, and
	// claims fewer bytes than `storage` holds.
	unsafe {
		(&raw mut (*this.as_ptr()).data_bytes).write(5);
		(&raw mut (*this.as_ptr()).data_bits).write(40);

		assert!(BfWrite::append(this, &[u32::MAX], 32));
		assert!(!BfWrite::append(this, &[u32::MAX], 8));
		assert_eq!((&raw const (*this.as_ptr()).cur_bit).read(), 32);

		(&raw mut (*this.as_ptr()).overflow).write(0);
		assert_eq!(BfWrite::read_back(this).unwrap().as_words(), [u32::MAX]);

		(&raw mut (*this.as_ptr()).cur_bit).write(33);
		assert_eq!(BfWrite::read_back(this), None);
	}

	assert_eq!(storage, [u32::MAX, 0xDEAD_BEEF]);
}

#[test]
fn engine_buffers_overflow_instead_of_growing() {
	let mut storage = [0u32; 1];
	let mut buffer = BfWrite::empty(&mut storage);
	let this = NonNull::from(&mut buffer);

	// SAFETY: The buffer describes `storage`, which only it accesses.
	unsafe {
		assert!(BfWrite::append(this, &[0], 20));
		assert!(!BfWrite::append(this, &[0], 20));
		assert_eq!((&raw const (*this.as_ptr()).overflow).read(), 1);
		assert_eq!(BfWrite::read_back(this), None);
	}
}

#[test]
fn engine_buffers_take_bits_across_words() {
	let mut storage = [0u32; 3];
	let mut buffer = BfWrite::empty(&mut storage);
	let this = NonNull::from(&mut buffer);
	let words = [0xABCD_EF12, 0b101];

	// SAFETY: The buffer describes `storage`, which only it accesses.
	let written = unsafe {
		(&raw mut (*this.as_ptr()).cur_bit).write(3);

		assert!(BfWrite::append(this, &words, 35));
		BfWrite::read_back(this).unwrap()
	};

	assert_eq!(written.len(), 38);
	assert_eq!(
		written.as_words(),
		[0xABCD_EF12 << 3, 0xABCD_EF12 >> 29 | 0b101 << 3]
	);
}

#[test]
fn engine_buffers_take_bits_without_disturbing_their_contents() {
	let mut storage = [u32::MAX; 2];
	let mut buffer = BfWrite::empty(&mut storage);
	let this = NonNull::from(&mut buffer);

	// SAFETY: The buffer describes `storage`, which only it accesses.
	let written = unsafe {
		// Starting past bits already written, which must survive.
		(&raw mut (*this.as_ptr()).cur_bit).write(5);

		assert!(BfWrite::append(this, &[0], 30));
		assert_eq!((&raw const (*this.as_ptr()).cur_bit).read(), 35);

		BfWrite::read_back(this).unwrap()
	};

	assert_eq!(written.len(), 35);
	assert_eq!(written.as_words(), [0b11111, 0]);
	assert_eq!(storage[1] >> 3, u32::MAX >> 3);
}

#[test]
fn readers_copy_the_bits_after_their_position() {
	let data = [0b1010_0000_u8, 0b0000_0111, 0xff];
	let mut reader = BfRead {
		data: data.as_ptr(),
		data_bytes: 3,
		data_bits: 20,
		cur_bit: 5,
		overflow: 0,
		assert_on_overflow: 0,
		debug_name: null(),
	};

	// SAFETY: The reader describes `data`.
	unsafe {
		let bits = reader.unread_bits(6).unwrap();

		assert_eq!(bits.len(), 6);
		assert_eq!(bits.as_words(), [0b11_1101]);
		assert_eq!(reader.unread_bits(15).unwrap().len(), 15);
		assert!(reader.unread_bits(16).is_none());
		assert!(reader.unread_bits(-1).is_none());

		reader.data_bits = 24;
		assert_eq!(reader.unread_bits(19).unwrap().len(), 19);

		reader.data_bytes = 2;
		assert!(reader.unread_bits(19).is_none());

		reader.data = null();
		assert!(reader.unread_bits(0).is_none());
	}
}

#[test]
fn written_buffers_expose_the_bits() {
	let words = [5];
	let buffer = BfWrite::written(&words, 6);

	assert_eq!(buffer.cur_bit, 6);
	assert_eq!(buffer.data_bits, 32);
	assert_eq!(buffer.data_bytes, 4);

	// SAFETY: The buffer describes `words`.
	assert_eq!(unsafe { buffer.data.read() }, 5);
}
