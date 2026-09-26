//! Source's bit buffers, as `bf_write` and `bf_read` in `tier1/bitbuf.cpp`
//! encode them.
//!
//! Bits are packed least significant first into little-endian 32-bit words,
//! which is the same as packing each byte least significant bit first. A
//! multi-bit field is stored low bit first. Net messages, user messages, and
//! game events are all encoded this way.

use crate::math::{QAngle, Vector};
use std::error::Error;
use std::ffi::{CStr, CString, c_char, c_int};
use std::fmt::{self, Display, Formatter};
use std::mem::offset_of;
use std::ptr::NonNull;

/// Bits in the integer part of a coordinate (`COORD_INTEGER_BITS`).
pub const COORD_INTEGER_BITS: u32 = 14;

/// Bits in the fraction of a coordinate (`COORD_FRACTIONAL_BITS`).
pub const COORD_FRACTIONAL_BITS: u32 = 5;

/// Bits in the fraction of a normal's component (`NORMAL_FRACTIONAL_BITS`).
pub const NORMAL_FRACTIONAL_BITS: u32 = 11;

const COORD_DENOMINATOR: i32 = 1 << COORD_FRACTIONAL_BITS;
const COORD_RESOLUTION: f32 = 1.0 / COORD_DENOMINATOR as f32;
const NORMAL_DENOMINATOR: i32 = (1 << NORMAL_FRACTIONAL_BITS) - 1;
// A double, as in `coordsize.h`, which components are compared against.
const NORMAL_RESOLUTION: f64 = 1.0 / NORMAL_DENOMINATOR as f64;

/// The most bytes a 32-bit variable-length integer takes.
const MAX_VAR_INT32_BYTES: usize = 5;

/// A growable buffer of bits, written the way `bf_write` writes them.
#[doc(alias = "bf_write")]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct BitWriter {
	/// Storage, whole words at a time. Bits past `len` are always zero.
	words: Vec<u32>,
	len: usize,
}

impl BitWriter {
	pub const fn new() -> Self {
		Self {
			words: Vec::new(),
			len: 0,
		}
	}

	/// An empty buffer with room for `bits` bits.
	pub fn with_capacity(bits: usize) -> Self {
		Self {
			words: Vec::with_capacity(bits.div_ceil(32)),
			len: 0,
		}
	}

	/// Copies `bits` bits from little-endian words, as a `bf_write` stores them.
	///
	/// # Panics
	///
	/// If `words` holds fewer than `bits` bits.
	pub fn from_words(words: &[u32], bits: usize) -> Self {
		assert!(
			bits <= words.len() * 32,
			"{bits} bits exceed the words given"
		);

		let mut writer = Self::with_capacity(bits);
		writer.write_words(words, bits);
		writer
	}

	/// The number of bits written.
	pub const fn len(&self) -> usize {
		self.len
	}

	pub const fn is_empty(&self) -> bool {
		self.len == 0
	}

	/// The number of bytes the bits written span.
	pub const fn byte_len(&self) -> usize {
		self.len.div_ceil(8)
	}

	pub fn clear(&mut self) {
		self.words.clear();
		self.len = 0;
	}

	/// The bits written, as little-endian words. Bits past [`len`](Self::len)
	/// in the last word are zero.
	pub fn as_words(&self) -> &[u32] {
		&self.words
	}

	/// The bits written, as bytes. Bits past [`len`](Self::len) in the last
	/// byte are zero.
	pub fn to_bytes(&self) -> Vec<u8> {
		let mut bytes: Vec<u8> = self
			.words
			.iter()
			.flat_map(|word| word.to_le_bytes())
			.collect();

		bytes.truncate(self.byte_len());
		bytes
	}

	/// Reads the bits written from the start.
	pub fn reader(&self) -> BitReader<'_> {
		BitReader::new(&self.words, self.len)
	}

	/// `WriteOneBit`.
	pub fn write_bit(&mut self, bit: bool) {
		self.push(u32::from(bit), 1);
	}

	/// Writes the low `bits` bits of `value`, as `WriteUBitLong` does.
	///
	/// # Panics
	///
	/// If `bits` exceeds 32, or `value` does not fit in `bits` bits.
	#[doc(alias = "WriteUBitLong")]
	pub fn write_ubits(&mut self, value: u32, bits: u32) {
		assert!(bits <= 32, "cannot write {bits} bits at once");
		assert!(
			bits == 32 || value >> bits == 0,
			"{value} does not fit in {bits} bits"
		);

		self.push(value, bits);
	}

	/// Writes `value` in two's complement in `bits` bits, as `WriteSBitLong`
	/// does.
	///
	/// # Panics
	///
	/// If `bits` is 0 or exceeds 32, or `value` does not fit in `bits` bits.
	#[doc(alias = "WriteSBitLong")]
	pub fn write_sbits(&mut self, value: i32, bits: u32) {
		assert!((1..=32).contains(&bits), "cannot write {bits} signed bits");

		let shift = 32 - bits;

		assert!(
			(value << shift) >> shift == value,
			"{value} does not fit in {bits} signed bits"
		);

		self.push(value as u32 & mask(bits), bits);
	}

	/// `WriteByte`.
	pub fn write_u8(&mut self, value: u8) {
		self.push(value.into(), 8);
	}

	/// `WriteChar`.
	pub fn write_i8(&mut self, value: i8) {
		self.write_sbits(value.into(), 8);
	}

	/// `WriteWord`.
	pub fn write_u16(&mut self, value: u16) {
		self.push(value.into(), 16);
	}

	/// `WriteShort`.
	pub fn write_i16(&mut self, value: i16) {
		self.write_sbits(value.into(), 16);
	}

	/// `WriteUBitLong` of all 32 bits.
	pub fn write_u32(&mut self, value: u32) {
		self.push(value, 32);
	}

	/// `WriteLong`.
	pub fn write_i32(&mut self, value: i32) {
		self.push(value as u32, 32);
	}

	/// The raw bits of a float, as `WriteFloat` and `WriteBitFloat` write them.
	#[doc(alias = "WriteFloat")]
	pub fn write_f32(&mut self, value: f32) {
		self.push(value.to_bits(), 32);
	}

	/// `WriteBytes`.
	pub fn write_bytes(&mut self, bytes: &[u8]) {
		for &byte in bytes {
			self.push(byte.into(), 8);
		}
	}

	/// A string and its terminator, as `WriteString` writes them.
	#[doc(alias = "WriteString")]
	pub fn write_cstr(&mut self, value: &CStr) {
		self.write_bytes(value.to_bytes_with_nul());
	}

	/// Appends everything written to `other`.
	pub fn write_bits(&mut self, other: &BitWriter) {
		self.write_words(&other.words, other.len);
	}

	/// An angle in degrees, as a fraction of a turn in `bits` bits, as
	/// `WriteBitAngle` writes it.
	///
	/// # Panics
	///
	/// If `bits` is 0 or exceeds 32.
	#[doc(alias = "WriteBitAngle")]
	pub fn write_bit_angle(&mut self, degrees: f32, bits: u32) {
		assert!(
			(1..=32).contains(&bits),
			"cannot write an angle in {bits} bits"
		);

		let turn = (1u64 << bits) as f64;
		let fraction = (f64::from(degrees) / 360.0 * turn) as i64;

		self.push((fraction as u32) & mask(bits), bits);
	}

	/// A world coordinate: flags for its integer and fraction, a sign, then
	/// each part present, as `WriteBitCoord` writes it.
	#[doc(alias = "WriteBitCoord")]
	pub fn write_bit_coord(&mut self, value: f32) {
		let negative = value <= -COORD_RESOLUTION;
		let integer = value.abs() as u32;
		let fraction = ((value * COORD_DENOMINATOR as f32) as i32).unsigned_abs()
			& (COORD_DENOMINATOR as u32 - 1);

		self.write_bit(integer != 0);
		self.write_bit(fraction != 0);

		if integer != 0 || fraction != 0 {
			self.write_bit(negative);

			if integer != 0 {
				// Integers from 1 are stored from 0, so the largest fits.
				self.push((integer - 1) & mask(COORD_INTEGER_BITS), COORD_INTEGER_BITS);
			}

			if fraction != 0 {
				self.push(fraction, COORD_FRACTIONAL_BITS);
			}
		}
	}

	/// A flag for each nonzero component, then each such component as a
	/// coordinate, as `WriteBitVec3Coord` writes them.
	#[doc(alias = "WriteBitVec3Coord")]
	pub fn write_bit_vec3_coord(&mut self, value: Vector) {
		let components = [value.x, value.y, value.z];
		let present = components.map(|component| component.abs() >= COORD_RESOLUTION);

		for flag in present {
			self.write_bit(flag);
		}

		for (component, present) in components.into_iter().zip(present) {
			if present {
				self.write_bit_coord(component);
			}
		}
	}

	/// Angles as coordinates, as `WriteBitAngles` writes them.
	#[doc(alias = "WriteBitAngles")]
	pub fn write_bit_angles(&mut self, value: QAngle) {
		self.write_bit_vec3_coord(Vector::new(value.pitch, value.yaw, value.roll));
	}

	/// A component of a unit vector: a sign, then the magnitude in
	/// [`NORMAL_FRACTIONAL_BITS`] bits, as `WriteBitNormal` writes it.
	#[doc(alias = "WriteBitNormal")]
	pub fn write_bit_normal(&mut self, value: f32) {
		let negative = f64::from(value) <= -NORMAL_RESOLUTION;
		let fraction = ((value * NORMAL_DENOMINATOR as f32) as i32)
			.unsigned_abs()
			.min(NORMAL_DENOMINATOR as u32);

		self.write_bit(negative);
		self.push(fraction, NORMAL_FRACTIONAL_BITS);
	}

	/// A unit vector: its x and y components when nonzero, then the sign of z,
	/// as `WriteBitVec3Normal` writes it.
	#[doc(alias = "WriteBitVec3Normal")]
	pub fn write_bit_vec3_normal(&mut self, value: Vector) {
		let x = f64::from(value.x.abs()) >= NORMAL_RESOLUTION;
		let y = f64::from(value.y.abs()) >= NORMAL_RESOLUTION;

		self.write_bit(x);
		self.write_bit(y);

		if x {
			self.write_bit_normal(value.x);
		}

		if y {
			self.write_bit_normal(value.y);
		}

		self.write_bit(f64::from(value.z) <= -NORMAL_RESOLUTION);
	}

	/// Seven bits at a time, low first, each byte flagging whether more
	/// follow, as `WriteVarInt32` writes it.
	#[doc(alias = "WriteVarInt32")]
	pub fn write_var_u32(&mut self, mut value: u32) {
		while value > 0x7f {
			self.push((value & 0x7f) | 0x80, 8);
			value >>= 7;
		}

		self.push(value, 8);
	}

	/// A two-bit length selector, then the value in 4, 8, 12, or 32 bits, as
	/// `WriteUBitVar` writes it.
	#[doc(alias = "WriteUBitVar")]
	pub fn write_ubit_var(&mut self, value: u32) {
		let (selector, bits) = match value {
			0..0x10 => (0, 4),
			0x10..0x100 => (1, 8),
			0x100..0x1000 => (2, 12),
			_ => (3, 32),
		};

		self.push(selector, 2);
		self.push(value, bits);
	}

	/// Appends the low `bits` bits of `value`, which must fit.
	fn push(&mut self, value: u32, bits: u32) {
		if bits == 0 {
			return;
		}

		let offset = (self.len % 32) as u32;
		let index = self.len / 32;

		self.len += bits as usize;
		self.words.resize(self.len.div_ceil(32), 0);
		self.words[index] |= value << offset;

		if offset + bits > 32 {
			self.words[index + 1] |= value >> (32 - offset);
		}
	}

	fn write_words(&mut self, words: &[u32], bits: usize) {
		let whole = bits / 32;

		for &word in &words[..whole] {
			self.push(word, 32);
		}

		let rest = (bits % 32) as u32;

		if rest != 0 {
			self.push(words[whole] & mask(rest), rest);
		}
	}
}

/// Reads bits the way `bf_read` does.
#[doc(alias = "bf_read")]
#[derive(Debug, Clone)]
pub struct BitReader<'a> {
	words: &'a [u32],
	len: usize,
	position: usize,
}

/// A read past the end of a [`BitReader`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overflow;

impl Display for Overflow {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.write_str("read past the end of the bit buffer")
	}
}

impl Error for Overflow {}

impl<'a> BitReader<'a> {
	/// Reads `len` bits of `words`.
	///
	/// # Panics
	///
	/// If `words` holds fewer than `len` bits.
	pub fn new(words: &'a [u32], len: usize) -> Self {
		assert!(len <= words.len() * 32, "{len} bits exceed the words given");

		Self {
			words,
			len,
			position: 0,
		}
	}

	/// The number of bits read.
	pub const fn position(&self) -> usize {
		self.position
	}

	/// The number of bits left.
	pub const fn remaining(&self) -> usize {
		self.len - self.position
	}

	/// `ReadOneBit`.
	pub fn read_bit(&mut self) -> Result<bool, Overflow> {
		Ok(self.read_ubits(1)? != 0)
	}

	/// `ReadUBitLong`.
	///
	/// # Panics
	///
	/// If `bits` exceeds 32.
	#[doc(alias = "ReadUBitLong")]
	pub fn read_ubits(&mut self, bits: u32) -> Result<u32, Overflow> {
		assert!(bits <= 32, "cannot read {bits} bits at once");

		if bits == 0 {
			return Ok(0);
		}

		if self.remaining() < bits as usize {
			return Err(Overflow);
		}

		let offset = (self.position % 32) as u32;
		let index = self.position / 32;
		let mut value = self.words[index] >> offset;

		if offset + bits > 32 {
			value |= self.words[index + 1] << (32 - offset);
		}

		self.position += bits as usize;
		Ok(value & mask(bits))
	}

	/// `ReadSBitLong`.
	///
	/// # Panics
	///
	/// If `bits` is 0 or exceeds 32.
	#[doc(alias = "ReadSBitLong")]
	pub fn read_sbits(&mut self, bits: u32) -> Result<i32, Overflow> {
		assert!((1..=32).contains(&bits), "cannot read {bits} signed bits");

		let shift = 32 - bits;

		Ok(((self.read_ubits(bits)? << shift) as i32) >> shift)
	}

	pub fn read_u8(&mut self) -> Result<u8, Overflow> {
		Ok(self.read_ubits(8)? as u8)
	}

	pub fn read_i8(&mut self) -> Result<i8, Overflow> {
		Ok(self.read_sbits(8)? as i8)
	}

	pub fn read_u16(&mut self) -> Result<u16, Overflow> {
		Ok(self.read_ubits(16)? as u16)
	}

	pub fn read_i16(&mut self) -> Result<i16, Overflow> {
		Ok(self.read_sbits(16)? as i16)
	}

	pub fn read_u32(&mut self) -> Result<u32, Overflow> {
		self.read_ubits(32)
	}

	pub fn read_i32(&mut self) -> Result<i32, Overflow> {
		Ok(self.read_ubits(32)? as i32)
	}

	pub fn read_f32(&mut self) -> Result<f32, Overflow> {
		Ok(f32::from_bits(self.read_ubits(32)?))
	}

	pub fn read_bytes(&mut self, len: usize) -> Result<Vec<u8>, Overflow> {
		if self.remaining() < len * 8 {
			return Err(Overflow);
		}

		(0..len).map(|_| self.read_u8()).collect()
	}

	/// Reads up to and including a terminator, as `ReadString` does.
	#[doc(alias = "ReadString")]
	pub fn read_cstring(&mut self) -> Result<CString, Overflow> {
		let mut bytes = Vec::new();

		loop {
			match self.read_u8()? {
				0 => break,
				byte => bytes.push(byte),
			}
		}

		// SAFETY: The loop stopped before the first NUL.
		Ok(unsafe { CString::from_vec_unchecked(bytes) })
	}

	/// Reads `bits` bits into a new buffer.
	pub fn read_bits(&mut self, bits: usize) -> Result<BitWriter, Overflow> {
		if self.remaining() < bits {
			return Err(Overflow);
		}

		let mut writer = BitWriter::with_capacity(bits);

		for _ in 0..bits / 32 {
			writer.push(self.read_ubits(32)?, 32);
		}

		let rest = (bits % 32) as u32;
		writer.push(self.read_ubits(rest)?, rest);

		Ok(writer)
	}

	/// `ReadBitAngle`.
	pub fn read_bit_angle(&mut self, bits: u32) -> Result<f32, Overflow> {
		let turn = (1u64 << bits) as f64;

		Ok((f64::from(self.read_ubits(bits)?) * 360.0 / turn) as f32)
	}

	/// `ReadBitCoord`.
	pub fn read_bit_coord(&mut self) -> Result<f32, Overflow> {
		let has_integer = self.read_bit()?;
		let has_fraction = self.read_bit()?;

		if !has_integer && !has_fraction {
			return Ok(0.0);
		}

		let negative = self.read_bit()?;
		let integer = match has_integer {
			true => self.read_ubits(COORD_INTEGER_BITS)? + 1,
			false => 0,
		};
		let fraction = match has_fraction {
			true => self.read_ubits(COORD_FRACTIONAL_BITS)?,
			false => 0,
		};
		let value = integer as f32 + fraction as f32 * COORD_RESOLUTION;

		Ok(if negative { -value } else { value })
	}

	/// `ReadBitVec3Coord`.
	pub fn read_bit_vec3_coord(&mut self) -> Result<Vector, Overflow> {
		let present = [self.read_bit()?, self.read_bit()?, self.read_bit()?];
		let mut components = [0.0; 3];

		for (component, present) in components.iter_mut().zip(present) {
			if present {
				*component = self.read_bit_coord()?;
			}
		}

		Ok(Vector::new(components[0], components[1], components[2]))
	}

	/// `ReadBitNormal`.
	pub fn read_bit_normal(&mut self) -> Result<f32, Overflow> {
		let negative = self.read_bit()?;
		let fraction = f64::from(self.read_ubits(NORMAL_FRACTIONAL_BITS)?);
		let value = (fraction / f64::from(NORMAL_DENOMINATOR)) as f32;

		Ok(if negative { -value } else { value })
	}

	/// `ReadVarInt32`.
	pub fn read_var_u32(&mut self) -> Result<u32, Overflow> {
		let mut value = 0u32;

		for index in 0..MAX_VAR_INT32_BYTES {
			let byte = self.read_u8()?;

			value |= u32::from(byte & 0x7f) << (7 * index);

			if byte & 0x80 == 0 {
				break;
			}
		}

		Ok(value)
	}

	/// `ReadUBitVar`.
	pub fn read_ubit_var(&mut self) -> Result<u32, Overflow> {
		let bits = match self.read_ubits(2)? {
			0 => 4,
			1 => 8,
			2 => 12,
			_ => 32,
		};

		self.read_ubits(bits)
	}
}

/// The low `bits` bits set.
const fn mask(bits: u32) -> u32 {
	match bits {
		32.. => u32::MAX,
		_ => (1 << bits) - 1,
	}
}

/// A layout mirror of the engine's `bf_write`, from `public/tier1/bitbuf.h`.
///
/// The generated binding is opaque, since the header does not parse. The
/// engine reads and writes these fields inline, so they match its own copy.
#[repr(C)]
#[derive(Debug)]
pub(crate) struct RawBfWrite {
	pub(crate) data: *mut u32,
	pub(crate) data_bytes: c_int,
	pub(crate) data_bits: c_int,
	pub(crate) cur_bit: c_int,
	pub(crate) overflow: bool,
	pub(crate) assert_on_overflow: bool,
	pub(crate) debug_name: *const c_char,
}

const _: () = {
	assert!(size_of::<RawBfWrite>() == 32);
	assert!(offset_of!(RawBfWrite, data_bytes) == 8);
	assert!(offset_of!(RawBfWrite, data_bits) == 12);
	assert!(offset_of!(RawBfWrite, cur_bit) == 16);
	assert!(offset_of!(RawBfWrite, overflow) == 20);
	assert!(offset_of!(RawBfWrite, debug_name) == 24);
};

/// Named in the engine's messages about overflowed buffers.
const DEBUG_NAME: &CStr = c"source_sdk_2013";

impl RawBfWrite {
	/// A buffer for the engine to write up to `words.len() * 32` bits into,
	/// from the start.
	///
	/// # Panics
	///
	/// If the buffer holds more than `c_int::MAX` bits.
	pub(crate) fn empty(words: &mut [u32]) -> Self {
		let bytes = c_int::try_from(words.len() * 4).expect("bit buffer too large");
		let bits = bytes.checked_mul(8).expect("bit buffer too large");

		Self {
			data: words.as_mut_ptr(),
			data_bytes: bytes,
			data_bits: bits,
			cur_bit: 0,
			overflow: false,
			assert_on_overflow: false,
			debug_name: DEBUG_NAME.as_ptr(),
		}
	}

	/// A full buffer holding what `writer` wrote, for the engine to read, as
	/// `INetChannel::SendData` does.
	///
	/// The engine only reads through the pointer, but `bf_write` stores it
	/// mutably.
	///
	/// # Panics
	///
	/// If the writer holds more than `c_int::MAX` bits.
	pub(crate) fn written(writer: &BitWriter) -> Self {
		let bytes = c_int::try_from(writer.words.len() * 4).expect("bit buffer too large");
		let bits = bytes.checked_mul(8).expect("bit buffer too large");

		Self {
			data: writer.words.as_ptr().cast_mut(),
			data_bytes: bytes,
			data_bits: bits,
			cur_bit: c_int::try_from(writer.len).expect("bit buffer too large"),
			overflow: false,
			assert_on_overflow: false,
			debug_name: DEBUG_NAME.as_ptr(),
		}
	}

	/// Copies what the engine wrote into `raw`, or `None` if it overflowed or
	/// its fields are inconsistent.
	///
	/// # Safety
	///
	/// `raw` must point to a live `bf_write` whose `data` holds at least
	/// `data_bytes` readable bytes, 4-byte aligned, which nothing writes to
	/// during the call.
	pub(crate) unsafe fn read_back(raw: NonNull<RawBfWrite>) -> Option<BitWriter> {
		let raw = raw.as_ptr();

		// SAFETY: The caller guarantees the buffer is live.
		let (data, bytes, bits, cur_bit, overflow) = unsafe {
			(
				(&raw const (*raw).data).read(),
				(&raw const (*raw).data_bytes).read(),
				(&raw const (*raw).data_bits).read(),
				(&raw const (*raw).cur_bit).read(),
				(&raw const (*raw).overflow).read(),
			)
		};

		let bytes = usize::try_from(bytes).ok()?;
		let cur_bit = usize::try_from(cur_bit).ok()?;

		if overflow
			|| data.is_null()
			|| cur_bit > usize::try_from(bits).ok()?
			|| cur_bit > bytes * 8
		{
			return None;
		}

		// SAFETY: The caller guarantees `data` holds `bytes` bytes, and
		// `cur_bit` is within them.
		let words = unsafe { std::slice::from_raw_parts(data, cur_bit.div_ceil(32)) };

		Some(BitWriter::from_words(words, cur_bit))
	}

	/// Appends `bits` to the buffer at `raw`, as `WriteBits` would, or marks it
	/// overflowed and returns false if they do not fit.
	///
	/// # Safety
	///
	/// `raw` must point to a live `bf_write` whose `data` holds at least
	/// `data_bytes` writable bytes, 4-byte aligned, which nothing else accesses
	/// during the call.
	pub(crate) unsafe fn append(raw: NonNull<RawBfWrite>, bits: &BitWriter) -> bool {
		let raw = raw.as_ptr();

		// SAFETY: The caller guarantees the buffer is live.
		let (data, bytes, limit, cur_bit, overflow) = unsafe {
			(
				(&raw const (*raw).data).read(),
				(&raw const (*raw).data_bytes).read(),
				(&raw const (*raw).data_bits).read(),
				(&raw const (*raw).cur_bit).read(),
				(&raw const (*raw).overflow).read(),
			)
		};

		let fits = (|| {
			let start = usize::try_from(cur_bit).ok()?;
			let end = start.checked_add(bits.len())?;
			let limit = usize::try_from(limit)
				.ok()?
				.min(usize::try_from(bytes).ok()? * 8);

			(!overflow && !data.is_null() && end <= limit).then_some((start, end))
		})();

		let Some((start, end)) = fits else {
			// SAFETY: As above.
			unsafe { (&raw mut (*raw).overflow).write(true) };
			return false;
		};

		let mut reader = bits.reader();
		let mut position = start;

		while position < end {
			let offset = (position % 32) as u32;
			let chunk = (32 - offset).min((end - position) as u32);
			let value = reader
				.read_ubits(chunk)
				.expect("the writer holds these bits");
			let chunk_mask = mask(chunk) << offset;

			// SAFETY: `position` is below `end`, which is within the buffer the
			// caller guarantees, so its word is too.
			unsafe {
				let word = data.add(position / 32);
				word.write((word.read() & !chunk_mask) | ((value << offset) & chunk_mask));
			}

			position += chunk as usize;
		}

		// SAFETY: As above. `end` fit in `data_bits`, a `c_int`.
		unsafe { (&raw mut (*raw).cur_bit).write(end as c_int) };
		true
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn bits(writer: &BitWriter) -> String {
		let mut reader = writer.reader();

		(0..writer.len())
			.map(|_| match reader.read_bit().unwrap() {
				true => '1',
				false => '0',
			})
			.collect()
	}

	#[test]
	fn fields_are_packed_low_bit_first_across_words() {
		let mut writer = BitWriter::new();

		writer.write_ubits(0b101, 3);
		writer.write_ubits(0xABCD_EF12, 32);
		writer.write_bit(true);

		assert_eq!(writer.len(), 36);
		assert_eq!(
			writer.as_words(),
			&[0xABCD_EF12 << 3 | 0b101, 0xABCD_EF12 >> 29 | 1 << 3]
		);
		assert_eq!(writer.to_bytes(), vec![0x95, 0x78, 0x6f, 0x5e, 0x0d]);

		let mut reader = writer.reader();

		assert_eq!(reader.read_ubits(3), Ok(0b101));
		assert_eq!(reader.read_u32(), Ok(0xABCD_EF12));
		assert_eq!(reader.read_bit(), Ok(true));
		assert_eq!(reader.read_bit(), Err(Overflow));
	}

	#[test]
	fn signed_fields_use_twos_complement() {
		let mut writer = BitWriter::new();

		writer.write_sbits(-1, 4);
		writer.write_i8(-128);
		writer.write_i16(-2);
		writer.write_i32(i32::MIN);

		assert_eq!(bits(&writer)[..4], *"1111");

		let mut reader = writer.reader();

		assert_eq!(reader.read_sbits(4), Ok(-1));
		assert_eq!(reader.read_i8(), Ok(-128));
		assert_eq!(reader.read_i16(), Ok(-2));
		assert_eq!(reader.read_i32(), Ok(i32::MIN));
	}

	#[test]
	#[should_panic(expected = "does not fit")]
	fn oversized_values_are_refused() {
		BitWriter::new().write_ubits(8, 3);
	}

	#[test]
	#[should_panic(expected = "does not fit")]
	fn oversized_signed_values_are_refused() {
		BitWriter::new().write_sbits(8, 4);
	}

	#[test]
	fn strings_end_with_their_terminator() {
		let mut writer = BitWriter::new();

		writer.write_bit(true);
		writer.write_cstr(c"sv_cheats");
		writer.write_cstr(c"");

		assert_eq!(writer.len(), 1 + 10 * 8 + 8);

		let mut reader = writer.reader();

		assert_eq!(reader.read_bit(), Ok(true));
		assert_eq!(reader.read_cstring().as_deref(), Ok(c"sv_cheats"));
		assert_eq!(reader.read_cstring().as_deref(), Ok(c""));
		assert_eq!(reader.remaining(), 0);
	}

	#[test]
	fn angles_are_fractions_of_a_turn() {
		let mut writer = BitWriter::new();

		writer.write_bit_angle(90.0, 16);
		writer.write_bit_angle(-90.0, 16);
		writer.write_bit_angle(359.0, 8);

		let mut reader = writer.reader();

		assert_eq!(reader.read_u16(), Ok(16384));
		assert_eq!(reader.read_u16(), Ok(49152));
		assert_eq!(reader.read_u8(), Ok(255));

		let mut reader = writer.reader();

		assert_eq!(reader.read_bit_angle(16), Ok(90.0));
		assert_eq!(reader.read_bit_angle(16), Ok(270.0));
	}

	#[test]
	fn coordinates_match_the_engines_encoding() {
		let cases: [(f32, &str); 5] = [
			(0.0, "00"),
			// Integer flag, no fraction, positive, 1 stored as 0.
			(1.0, "10000000000000000"),
			// No integer, fraction flag, negative, 16/32.
			(-0.5, "01100001"),
			// Both, positive, 3 stored as 2, then 8/32.
			(3.25, "1100100000000000000010"),
			// Below the resolution rounds to zero.
			(0.01, "00"),
		];

		for (value, expected) in cases {
			let mut writer = BitWriter::new();

			writer.write_bit_coord(value);
			assert_eq!(bits(&writer), expected, "{value}");
		}

		let mut writer = BitWriter::new();
		let position = Vector::new(-1024.5, 0.0, 12.03125);

		writer.write_bit_vec3_coord(position);
		assert_eq!(bits(&writer)[..3], *"101");
		assert_eq!(writer.reader().read_bit_vec3_coord(), Ok(position));
	}

	#[test]
	fn normals_clamp_to_the_fraction_bits() {
		let mut writer = BitWriter::new();

		writer.write_bit_normal(1.0);
		writer.write_bit_normal(-1.5);
		writer.write_bit_vec3_normal(Vector::new(0.0, 0.5, -0.5));

		let mut reader = writer.reader();

		assert_eq!(reader.read_bit_normal(), Ok(1.0));
		assert_eq!(reader.read_bit_normal(), Ok(-1.0));
		assert_eq!(reader.read_bit(), Ok(false));
		assert_eq!(reader.read_bit(), Ok(true));
		assert!(f64::from((reader.read_bit_normal().unwrap() - 0.5).abs()) < NORMAL_RESOLUTION);
		assert_eq!(reader.read_bit(), Ok(true));
		assert_eq!(reader.remaining(), 0);
	}

	#[test]
	fn variable_length_integers_round_trip() {
		let values = [0, 0x7f, 0x80, 0x3fff, 0x4000, u32::MAX];
		let mut writer = BitWriter::new();

		for value in values {
			writer.write_var_u32(value);
			writer.write_ubit_var(value);
		}

		let mut reader = writer.reader();

		for value in values {
			assert_eq!(reader.read_var_u32(), Ok(value));
			assert_eq!(reader.read_ubit_var(), Ok(value));
		}

		let mut writer = BitWriter::new();

		writer.write_var_u32(300);
		assert_eq!(writer.to_bytes(), vec![0xac, 0x02]);
	}

	#[test]
	fn buffers_append_at_any_offset() {
		let mut inner = BitWriter::new();

		inner.write_ubits(0x1_2345, 17);
		inner.write_cstr(c"hi");

		let mut outer = BitWriter::new();

		outer.write_ubits(0b11, 2);
		outer.write_bits(&inner);

		let mut reader = outer.reader();

		assert_eq!(reader.read_ubits(2), Ok(0b11));
		assert_eq!(reader.read_bits(inner.len()), Ok(inner.clone()));

		let copy = BitWriter::from_words(outer.as_words(), outer.len());

		assert_eq!(copy, outer);
	}

	#[test]
	fn engine_buffers_take_bits_without_disturbing_their_contents() {
		let mut storage = [u32::MAX; 2];
		let mut raw = RawBfWrite::empty(&mut storage);
		let raw_pointer = NonNull::from(&mut raw);
		let mut bits = BitWriter::new();

		bits.write_ubits(0, 30);

		// Starting past bits already written, which must survive.
		unsafe { (&raw mut (*raw_pointer.as_ptr()).cur_bit).write(5) };

		assert!(unsafe { RawBfWrite::append(raw_pointer, &bits) });
		assert_eq!(
			unsafe { (&raw const (*raw_pointer.as_ptr()).cur_bit).read() },
			35
		);

		let written = unsafe { RawBfWrite::read_back(raw_pointer) }.unwrap();
		let mut reader = written.reader();

		assert_eq!(reader.read_ubits(5), Ok(0b11111));
		assert_eq!(reader.read_ubits(30), Ok(0));
		assert_eq!(storage[1] >> 3, u32::MAX >> 3);
	}

	#[test]
	fn engine_buffers_overflow_instead_of_growing() {
		let mut storage = [0u32; 1];
		let mut raw = RawBfWrite::empty(&mut storage);
		let raw_pointer = NonNull::from(&mut raw);
		let mut bits = BitWriter::new();

		bits.write_ubits(0, 20);

		assert!(unsafe { RawBfWrite::append(raw_pointer, &bits) });
		assert!(!unsafe { RawBfWrite::append(raw_pointer, &bits) });
		assert!(unsafe { (&raw const (*raw_pointer.as_ptr()).overflow).read() });
		assert_eq!(unsafe { RawBfWrite::read_back(raw_pointer) }, None);
	}

	#[test]
	fn written_buffers_expose_the_writers_bits() {
		let mut writer = BitWriter::new();

		writer.write_ubits(5, 6);

		let raw = RawBfWrite::written(&writer);

		assert_eq!(raw.cur_bit, 6);
		assert_eq!(raw.data_bits, 32);
		assert_eq!(unsafe { raw.data.read() }, 5);
	}
}
