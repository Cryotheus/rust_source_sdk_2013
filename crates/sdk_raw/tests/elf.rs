//! Tests of parsing ELF modules and their symbols.

use source_sdk_2013_raw::util::elf::Elf;

fn fixture() -> Vec<u8> {
	let mut bytes = vec![0; 512];
	bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
	bytes[16..18].copy_from_slice(&3_u16.to_le_bytes());
	bytes[18..20].copy_from_slice(&62_u16.to_le_bytes());
	bytes[40..48].copy_from_slice(&64_usize.to_le_bytes());
	bytes[58..60].copy_from_slice(&64_u16.to_le_bytes());
	bytes[60..62].copy_from_slice(&4_u16.to_le_bytes());
	// Executable section #1 at virtual address 0x1000, file offset 400.
	bytes[132..136].copy_from_slice(&1_u32.to_le_bytes());
	bytes[136..144].copy_from_slice(&4_usize.to_le_bytes());
	bytes[144..152].copy_from_slice(&0x1000_usize.to_le_bytes());
	bytes[152..160].copy_from_slice(&400_usize.to_le_bytes());
	bytes[160..168].copy_from_slice(&4_usize.to_le_bytes());
	bytes[400..404].copy_from_slice(&[0x31, 0xc0, 0xc3, 0x90]);
	// Symbol table #2, pointing at string table #3.
	bytes[196..200].copy_from_slice(&2_u32.to_le_bytes());
	bytes[216..224].copy_from_slice(&320_usize.to_le_bytes());
	bytes[224..232].copy_from_slice(&24_usize.to_le_bytes());
	bytes[232..236].copy_from_slice(&3_u32.to_le_bytes());
	bytes[248..256].copy_from_slice(&24_usize.to_le_bytes());
	bytes[324] = 2;
	bytes[326..328].copy_from_slice(&1_u16.to_le_bytes());
	bytes[328..336].copy_from_slice(&0x1000_usize.to_le_bytes());
	bytes[336..344].copy_from_slice(&3_usize.to_le_bytes());
	bytes[260..264].copy_from_slice(&3_u32.to_le_bytes());
	bytes[280..288].copy_from_slice(&380_usize.to_le_bytes());
	bytes[288..296].copy_from_slice(&5_usize.to_le_bytes());
	bytes[380..385].copy_from_slice(b"test\0");
	bytes
}

#[test]
fn parser_rejects_truncation_and_wrong_abi() {
	let mut bytes = fixture();
	assert!(Elf::new(&[]).is_none());
	assert!(Elf::new(&bytes[..100]).is_none());
	assert!(Elf::new(&bytes).is_some());
	bytes[4] = 1;
	assert!(Elf::new(&bytes).is_none());
}

#[test]
fn symbols_require_unique_executable_bounded_definitions() {
	let mut bytes = fixture();
	assert_eq!(
		Elf::new(&bytes).unwrap().symbol(b"test"),
		Some((0x1000, [0x31, 0xc0, 0xc3].as_slice()))
	);
	assert!(Elf::new(&bytes).unwrap().symbol(b"absent").is_none());
	bytes[136..144].copy_from_slice(&0_usize.to_le_bytes());
	assert!(Elf::new(&bytes).unwrap().symbol(b"test").is_none());
	bytes = fixture();
	bytes[336..344].copy_from_slice(&5_usize.to_le_bytes());
	assert!(Elf::new(&bytes).unwrap().symbol(b"test").is_none());
	bytes = fixture();
	let duplicate = bytes[320..344].to_vec();
	bytes[344..368].copy_from_slice(&duplicate);
	bytes[224..232].copy_from_slice(&48_usize.to_le_bytes());
	assert!(Elf::new(&bytes).unwrap().symbol(b"test").is_none());
}
