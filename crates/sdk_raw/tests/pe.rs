//! Tests of parsing PE32+ images.

use source_sdk_2013_raw::util::pe::from_file;

fn fixture() -> Vec<u8> {
	let mut bytes = vec![0; 0x204];
	bytes[..2].copy_from_slice(b"MZ");
	bytes[60..64].copy_from_slice(&64_u32.to_le_bytes());
	bytes[64..68].copy_from_slice(b"PE\0\0");
	bytes[68..70].copy_from_slice(&0x8664_u16.to_le_bytes());
	bytes[70..72].copy_from_slice(&1_u16.to_le_bytes());
	bytes[84..86].copy_from_slice(&112_u16.to_le_bytes());
	bytes[88..90].copy_from_slice(&0x20b_u16.to_le_bytes());
	bytes[112..120].copy_from_slice(&0x10000_usize.to_le_bytes());
	bytes[144..148].copy_from_slice(&0x2000_u32.to_le_bytes());
	for (offset, value) in [
		(208, 8_u32),
		(212, 0x1000),
		(216, 4),
		(220, 0x200),
		(236, 0x60000000),
	] {
		bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
	}
	bytes[0x200..].copy_from_slice(&[1, 2, 3, 4]);
	bytes
}

#[test]
fn pe_file_checks_headers_ranges_and_zero_fills_virtual_tail() {
	let bytes = fixture();
	let image = from_file(&bytes).unwrap();
	assert_eq!(
		image.read(0x11000, 8),
		Some([1, 2, 3, 4, 0, 0, 0, 0].as_slice())
	);
	assert!(image.executable(0x11000));
	for length in [0, 63, 88, 199, 239, 0x203] {
		assert!(from_file(&bytes[..length]).is_err());
	}
	let mut invalid = bytes.clone();
	invalid[68] = 0;
	assert!(from_file(&invalid).is_err());
	invalid = bytes.clone();
	invalid[212..216].copy_from_slice(&0x2000_u32.to_le_bytes());
	assert!(from_file(&invalid).is_err());
	invalid = bytes;
	invalid[112..120].copy_from_slice(&usize::MAX.to_le_bytes());
	assert!(from_file(&invalid).is_err());
}
