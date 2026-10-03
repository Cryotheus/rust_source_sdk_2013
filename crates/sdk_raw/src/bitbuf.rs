//! Hand-written ABI of the engine's bit buffers, `bf_write` and `bf_read`,
//! from `public/tier1/bitbuf.h`.
//!
//! The generated bindings of both classes are opaque, since the header does
//! not parse. The engine reads and writes their fields inline, so these
//! mirrors match its own copies.
//!
//! The buffers pack bits least significant first into little-endian 32-bit
//! words, which is the same as packing each byte least significant bit first.
//!
//! The mirrors hold the classes' C++ `bool`s as bytes: they are also copied
//! out of engine objects whose layout is learned at run time, where a byte
//! need not be a valid `bool`.

use std::ffi::{CStr, c_char, c_int};
use std::mem::offset_of;
use std::ptr::NonNull;

const _: () = {
	assert!(size_of::<BfWrite>() == 32);
	assert!(offset_of!(BfWrite, data_bytes) == 8);
	assert!(offset_of!(BfWrite, data_bits) == 12);
	assert!(offset_of!(BfWrite, cur_bit) == 16);
	assert!(offset_of!(BfWrite, overflow) == 20);
	assert!(offset_of!(BfWrite, assert_on_overflow) == 21);
	assert!(offset_of!(BfWrite, debug_name) == 24);

	assert!(size_of::<BfRead>() == 32);
	assert!(offset_of!(BfRead, data_bytes) == 8);
	assert!(offset_of!(BfRead, data_bits) == 12);
	assert!(offset_of!(BfRead, cur_bit) == 16);
	assert!(offset_of!(BfRead, overflow) == 20);
	assert!(offset_of!(BfRead, assert_on_overflow) == 21);
	assert!(offset_of!(BfRead, debug_name) == 24);
};

/// A layout mirror of the engine's `bf_read`.
#[doc(alias("bf_read"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct BfRead {
	/// The buffer (`m_pData`).
	pub data: *const u8,

	/// The size of the buffer in bytes (`m_nDataBytes`).
	pub data_bytes: c_int,

	/// The number of bits that may be read (`m_nDataBits`).
	pub data_bits: c_int,

	/// The number of bits read (`m_iCurBit`).
	pub cur_bit: c_int,

	/// Whether a read did not fit (`m_bOverflow`), a C++ `bool`.
	pub overflow: u8,

	/// Whether the engine asserts when a read does not fit
	/// (`m_bAssertOnOverflow`), a C++ `bool`.
	pub assert_on_overflow: u8,

	/// Named in the engine's messages about overflow (`m_pDebugName`).
	pub debug_name: *const c_char,
}

impl BfRead {
	/// The mirror of the engine's buffer `buffer` points to.
	pub const fn from_sys(buffer: NonNull<sys::bf_read>) -> NonNull<Self> {
		buffer.cast()
	}

	/// The engine's type for the buffer, for the calls that take one.
	pub fn as_raw(&mut self) -> *mut sys::bf_read {
		(&raw mut *self).cast()
	}

	/// Copies the `bits` bits that follow the reader's position, or returns
	/// `None` if its buffer does not hold them.
	///
	/// # Safety
	///
	/// [`data`](Self::data), unless null, must point to at least
	/// [`data_bytes`](Self::data_bytes) readable bytes, which nothing writes
	/// to during the call.
	pub unsafe fn unread_bits(&self, bits: c_int) -> Option<Bits> {
		let len = usize::try_from(bits).ok()?;
		let position = usize::try_from(self.cur_bit).ok()?;
		let end = position.checked_add(len)?;

		if self.data.is_null()
			|| end > usize::try_from(self.data_bits).ok()?
			|| end > usize::try_from(self.data_bytes).ok()?.checked_mul(8)?
		{
			return None;
		}

		let mut words = vec![0; len.div_ceil(32)];

		for (index, bit) in (position..end).enumerate() {
			// SAFETY: `end` is within the buffer's bytes, checked above, which
			// the caller guarantees are readable.
			let byte = unsafe { self.data.add(bit / 8).read() };

			words[index / 32] |= u32::from(byte >> (bit % 8) & 1) << (index % 32);
		}

		Some(Bits { words, len })
	}
}

/// A layout mirror of the engine's `bf_write`.
#[doc(alias("bf_write"))]
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct BfWrite {
	/// The storage (`m_pData`).
	pub data: *mut u32,

	/// The size of the storage in bytes (`m_nDataBytes`).
	pub data_bytes: c_int,

	/// The most bits that may be written (`m_nDataBits`).
	pub data_bits: c_int,

	/// The number of bits written (`m_iCurBit`).
	pub cur_bit: c_int,

	/// Whether a write did not fit (`m_bOverflow`), a C++ `bool`.
	pub overflow: u8,

	/// Whether the engine asserts when a write does not fit
	/// (`m_bAssertOnOverflow`), a C++ `bool`.
	pub assert_on_overflow: u8,

	/// Named in the engine's messages about overflow (`m_pDebugName`).
	pub debug_name: *const c_char,
}

impl BfWrite {
	/// The [`debug_name`](Self::debug_name) of the buffers this module makes,
	/// which the engine names in its messages about overflow.
	pub const DEBUG_NAME: &'static CStr = c"source_sdk_2013";

	/// Appends the first `bits` bits of `words` to the buffer `this` points
	/// to, as `WriteBits` would, or marks it overflowed and returns false if
	/// they do not fit. The rest of the buffer's storage keeps its bits.
	///
	/// # Panics
	///
	/// If `words` holds fewer than `bits` bits.
	///
	/// # Safety
	///
	/// `this` must point to a live `bf_write` whose `data`, unless null, holds
	/// at least `data_bytes` writable bytes, 4-byte aligned, which nothing
	/// else accesses during the call. Only the whole 32-bit words within
	/// `data_bytes` are accessed: bits that would fall in a trailing partial
	/// word do not fit.
	pub unsafe fn append(this: NonNull<Self>, words: &[u32], bits: usize) -> bool {
		assert!(
			bits.div_ceil(32) <= words.len(),
			"{bits} bits exceed the words given"
		);

		let this = this.as_ptr();

		// SAFETY: The caller guarantees the buffer is live.
		let (data, bytes, limit, cur_bit, overflow) = unsafe {
			(
				(&raw const (*this).data).read(),
				(&raw const (*this).data_bytes).read(),
				(&raw const (*this).data_bits).read(),
				(&raw const (*this).cur_bit).read(),
				(&raw const (*this).overflow).read(),
			)
		};

		let fits = (|| {
			let start = usize::try_from(cur_bit).ok()?;
			let end = start.checked_add(bits)?;
			let limit = usize::try_from(limit)
				.ok()?
				.min((usize::try_from(bytes).ok()? / 4).checked_mul(32)?);

			(overflow == 0 && !data.is_null() && end <= limit).then_some((start, end))
		})();

		let Some((start, end)) = fits else {
			// SAFETY: As above.
			unsafe { (&raw mut (*this).overflow).write(1) };
			return false;
		};

		let mut position = start;

		while position < end {
			let offset = (position % 32) as u32;
			let chunk = (32 - offset).min(u32::try_from(end - position).unwrap_or(u32::MAX));
			let value = bits_at(words, position - start, chunk);
			let chunk_mask = mask(chunk) << offset;

			// SAFETY: `position` is below `end`, which is within the whole words
			// of the storage the caller guarantees, so its word is too.
			unsafe {
				let word = data.add(position / 32);
				word.write((word.read() & !chunk_mask) | ((value << offset) & chunk_mask));
			}

			position += chunk as usize;
		}

		// SAFETY: As above. `end` fit in `data_bits`, a `c_int`.
		unsafe { (&raw mut (*this).cur_bit).write(end as c_int) };
		true
	}

	/// A buffer for the engine to write up to `words.len() * 32` bits into,
	/// from the start.
	///
	/// The buffer points to `words` without borrowing them: they must stay
	/// alive, and unused otherwise, while the engine writes to it.
	///
	/// # Panics
	///
	/// If the buffer holds more than `c_int::MAX` bits.
	pub fn empty(words: &mut [u32]) -> Self {
		let (data_bytes, data_bits) = sizes(words.len());

		Self {
			data: words.as_mut_ptr(),
			data_bytes,
			data_bits,
			cur_bit: 0,
			overflow: 0,
			assert_on_overflow: 0,
			debug_name: Self::DEBUG_NAME.as_ptr(),
		}
	}

	/// The mirror of the engine's buffer `buffer` points to.
	pub const fn from_sys(buffer: NonNull<sys::bf_write>) -> NonNull<Self> {
		buffer.cast()
	}

	/// Copies the bits written to the buffer `this` points to, or returns
	/// `None` if it overflowed or its fields are inconsistent.
	///
	/// # Safety
	///
	/// `this` must point to a live `bf_write` whose `data`, unless null, holds
	/// at least `data_bytes` readable bytes, 4-byte aligned, which nothing
	/// writes to during the call. Only the whole 32-bit words within
	/// `data_bytes` are accessed: bits written into a trailing partial word
	/// make the fields inconsistent.
	pub unsafe fn read_back(this: NonNull<Self>) -> Option<Bits> {
		let this = this.as_ptr();

		// SAFETY: The caller guarantees the buffer is live.
		let (data, bytes, limit, cur_bit, overflow) = unsafe {
			(
				(&raw const (*this).data).read(),
				(&raw const (*this).data_bytes).read(),
				(&raw const (*this).data_bits).read(),
				(&raw const (*this).cur_bit).read(),
				(&raw const (*this).overflow).read(),
			)
		};

		let len = usize::try_from(cur_bit).ok()?;

		if overflow != 0
			|| data.is_null()
			|| len > usize::try_from(limit).ok()?
			|| len > (usize::try_from(bytes).ok()? / 4).checked_mul(32)?
		{
			return None;
		}

		// SAFETY: The caller guarantees `data` holds `bytes` readable bytes,
		// and `len` bits are within their whole words.
		let mut words = unsafe { std::slice::from_raw_parts(data, len.div_ceil(32)) }.to_vec();

		if let (Some(last), rest @ 1..) = (words.last_mut(), (len % 32) as u32) {
			*last &= mask(rest);
		}

		Some(Bits { words, len })
	}

	/// A full buffer describing the first `bits` bits of `words`, for the
	/// engine to read, as `INetChannel::SendData` does.
	///
	/// `bf_write` stores its storage as mutable, but the engine must only read
	/// through this buffer's. The buffer points to `words` without borrowing
	/// them: they must stay alive while the engine reads it.
	///
	/// # Panics
	///
	/// If `words` holds fewer than `bits` bits, or more than `c_int::MAX`
	/// bits.
	pub fn written(words: &[u32], bits: usize) -> Self {
		assert!(
			bits.div_ceil(32) <= words.len(),
			"{bits} bits exceed the words given"
		);

		let (data_bytes, data_bits) = sizes(words.len());

		Self {
			data: words.as_ptr().cast_mut(),
			data_bytes,
			data_bits,
			// `bits` is at most `data_bits`, a `c_int`.
			cur_bit: bits as c_int,
			overflow: 0,
			assert_on_overflow: 0,
			debug_name: Self::DEBUG_NAME.as_ptr(),
		}
	}

	/// The engine's type for the buffer, for the calls that take one.
	pub fn as_raw(&mut self) -> *mut sys::bf_write {
		(&raw mut *self).cast()
	}
}

/// Bits copied out of one of the engine's buffers, packed as the buffers pack
/// them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Bits {
	/// The bits, as little-endian words. Bits past `len` are zero.
	words: Vec<u32>,

	/// The number of bits.
	len: usize,
}

impl Bits {
	/// The bits, as little-endian words, low bit first. Bits past
	/// [`len`](Self::len) in the last word are zero.
	pub fn as_words(&self) -> &[u32] {
		&self.words
	}

	/// Whether there are no bits.
	pub const fn is_empty(&self) -> bool {
		self.len == 0
	}

	/// The number of bits.
	pub const fn len(&self) -> usize {
		self.len
	}
}

/// `bits` bits of `words` from bit `position`, which must hold them.
fn bits_at(words: &[u32], position: usize, bits: u32) -> u32 {
	let offset = (position % 32) as u32;
	let index = position / 32;
	let mut value = words[index] >> offset;

	if offset + bits > 32 {
		value |= words[index + 1] << (32 - offset);
	}

	value & mask(bits)
}

/// The low `bits` bits set.
const fn mask(bits: u32) -> u32 {
	match bits {
		32.. => u32::MAX,
		_ => (1 << bits) - 1,
	}
}

/// A buffer's `data_bytes` and `data_bits` for storage of `words` words.
///
/// # Panics
///
/// If the storage holds more than `c_int::MAX` bits.
fn sizes(words: usize) -> (c_int, c_int) {
	let bytes = words
		.checked_mul(size_of::<u32>())
		.and_then(|bytes| c_int::try_from(bytes).ok())
		.expect("bit buffer too large");
	let bits = bytes.checked_mul(8).expect("bit buffer too large");

	(bytes, bits)
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::ptr::null;

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

		// SAFETY: As above.
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

		// SAFETY: As above.
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
}
