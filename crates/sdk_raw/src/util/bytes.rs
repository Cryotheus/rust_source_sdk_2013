//! Bounded little-endian readers for the supported 64-bit executable formats.

/// Resolve an x86 rel32 operand relative to the byte following that operand.
pub fn relative(address: usize, bytes: &[u8], operand: usize) -> Option<usize> {
	let displacement = u32_at(bytes, operand)? as i32;
	address
		.checked_add(operand)?
		.checked_add(4)?
		.checked_add_signed(displacement as isize)
}

/// Read a little-endian 16-bit integer without indexing outside the slice.
pub fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
	Some(u16::from_le_bytes(
		bytes.get(offset..offset.checked_add(2)?)?.try_into().ok()?,
	))
}

/// Read a little-endian 32-bit integer without indexing outside the slice.
pub fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
	Some(u32::from_le_bytes(
		bytes.get(offset..offset.checked_add(4)?)?.try_into().ok()?,
	))
}

/// Read a little-endian 64-bit address on a supported 64-bit target.
pub fn word_at(bytes: &[u8], offset: usize) -> Option<usize> {
	Some(usize::from_le_bytes(
		bytes.get(offset..offset.checked_add(8)?)?.try_into().ok()?,
	))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn relative_operands_are_signed_checked_and_bounded() {
		assert_eq!(
			relative(0x1000, &[0xe8, 0xf0, 0xff, 0xff, 0xff], 1),
			Some(0xff5)
		);
		assert_eq!(relative(0, &[0xe8, 0xf0, 0xff, 0xff, 0xff], 1), None);
		assert_eq!(relative(0x1000, &[0xe8, 0, 0], 1), None);
		assert_eq!(relative(usize::MAX, &[0; 4], 0), None);
		assert_eq!(u16_at(&[0; 8], usize::MAX), None);
		assert_eq!(u32_at(&[0; 8], usize::MAX), None);
		assert_eq!(word_at(&[0; 7], 0), None);
	}
}
