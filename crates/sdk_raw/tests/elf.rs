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

/// [`fixture`], with its section #1 a writable `.bss` at virtual address
/// 0x2000, 0x100 bytes long without file bytes, and its symbol a 0x20-byte
/// variable in it.
fn data_fixture() -> Vec<u8> {
	let mut bytes = fixture();
	bytes[132..136].copy_from_slice(&8_u32.to_le_bytes());
	bytes[136..144].copy_from_slice(&3_usize.to_le_bytes());
	bytes[144..152].copy_from_slice(&0x2000_usize.to_le_bytes());
	bytes[152..160].copy_from_slice(&0x10000_usize.to_le_bytes());
	bytes[160..168].copy_from_slice(&0x100_usize.to_le_bytes());
	bytes[324] = 1;
	bytes[328..336].copy_from_slice(&0x2010_usize.to_le_bytes());
	bytes[336..344].copy_from_slice(&0x20_usize.to_le_bytes());
	bytes
}

#[test]
fn data_symbols_require_unique_writable_bounded_definitions() {
	// A variable in `.bss`, whose section has no bytes in the file.
	assert_eq!(
		Elf::new(&data_fixture()).unwrap().data_symbol(b"test"),
		Some((0x2010, 0x20))
	);

	// Functions are not variables, and variables are not functions.
	assert!(Elf::new(&fixture()).unwrap().data_symbol(b"test").is_none());
	assert!(Elf::new(&data_fixture()).unwrap().symbol(b"test").is_none());

	// Read-only, executable, and unallocated sections are refused.
	for flags in [2_usize, 7, 1] {
		let mut bytes = data_fixture();
		bytes[136..144].copy_from_slice(&flags.to_le_bytes());
		assert!(Elf::new(&bytes).unwrap().data_symbol(b"test").is_none());
	}

	// So are variables that overrun their section, and empty ones.
	for size in [0xf1_usize, 0] {
		let mut bytes = data_fixture();
		bytes[336..344].copy_from_slice(&size.to_le_bytes());
		assert!(Elf::new(&bytes).unwrap().data_symbol(b"test").is_none());
	}

	// And duplicate definitions.
	let mut bytes = data_fixture();
	let duplicate = bytes[320..344].to_vec();
	bytes[344..368].copy_from_slice(&duplicate);
	bytes[224..232].copy_from_slice(&48_usize.to_le_bytes());
	assert!(Elf::new(&bytes).unwrap().data_symbol(b"test").is_none());
}
