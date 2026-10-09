//! Tests of the bit streams' encodings, which must match `bf_write` and
//! `bf_read` bit for bit.

use source_sdk_2013::bitbuf::{BitWriter, NORMAL_FRACTIONAL_BITS, Overflow};
use source_sdk_2013::math::Vector;

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

/// The bits `writer` holds, in the order they were written.
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
fn normals_clamp_to_the_fraction_bits() {
	// The step between the magnitudes a component stores
	// (`NORMAL_RESOLUTION`).
	let resolution = 1.0 / f64::from((1 << NORMAL_FRACTIONAL_BITS) - 1);
	let mut writer = BitWriter::new();

	writer.write_bit_normal(1.0);
	writer.write_bit_normal(-1.5);
	writer.write_bit_vec3_normal(Vector::new(0.0, 0.5, -0.5));

	let mut reader = writer.reader();

	assert_eq!(reader.read_bit_normal(), Ok(1.0));
	assert_eq!(reader.read_bit_normal(), Ok(-1.0));
	assert_eq!(reader.read_bit(), Ok(false));
	assert_eq!(reader.read_bit(), Ok(true));
	assert!(f64::from((reader.read_bit_normal().unwrap() - 0.5).abs()) < resolution);
	assert_eq!(reader.read_bit(), Ok(true));
	assert_eq!(reader.remaining(), 0);
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
fn skipped_bits_are_passed_over_whole_or_not_at_all() {
	let mut writer = BitWriter::new();

	writer.write_ubits(0b101, 3);
	writer.write_u32(0xDEAD_BEEF);
	writer.write_ubits(0b11, 2);

	let mut reader = writer.reader();

	assert_eq!(reader.skip(3), Ok(()));
	assert_eq!(reader.skip(35), Err(Overflow));
	assert_eq!(reader.position(), 3);
	assert_eq!(reader.skip(32), Ok(()));
	assert_eq!(reader.read_ubits(2), Ok(0b11));
	assert_eq!(reader.skip(0), Ok(()));
	assert_eq!(reader.skip(1), Err(Overflow));
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
