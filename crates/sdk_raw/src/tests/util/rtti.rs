//! Tests of discovering primary vtables through run-time type information.

use super::*;
use crate::util::Section;

const BASE: usize = 0x10000;

fn fixture() -> Image {
	Image {
		base: BASE,
		sections: vec![
			Section {
				address: BASE,
				bytes: vec![0; 1024],
				executable: false,
				writable: false,
			},
			Section {
				address: 0x20000,
				bytes: vec![0; 0x1000],
				executable: true,
				writable: false,
			},
		],
	}
}

#[test]
fn itanium_excludes_secondary_and_truncated_tables() {
	let mut image = fixture();
	image.sections[0].bytes[0x100..0x10d].copy_from_slice(b"10CKickIssue\0");
	word(&mut image, 0x188, BASE + 0x100);
	word(&mut image, 0x208, BASE + 0x180);
	word(&mut image, 0x210 + 9 * 8, 0x20010);
	assert_eq!(image.itanium("CKickIssue", 9), [BASE + 0x210]);
	word(&mut image, 0x200, usize::MAX - 7);
	assert!(image.itanium("CKickIssue", 9).is_empty());
	word(&mut image, 0x200, 0);
	image.sections[0].bytes.truncate(0x210 + 9 * 8);
	assert!(image.itanium("CKickIssue", 9).is_empty());
}

#[test]
fn malformed_locator_and_slot_arithmetic_cannot_wrap() {
	let mut image = fixture();
	assert!(!image.valid_table(BASE, usize::MAX));
	image.base = usize::MAX - 2047;
	image.sections[0].address = image.base;
	image.sections[0].bytes[0x110..0x121].copy_from_slice(b".?AVCKickIssue@@\0");
	for (offset, value) in [(0x180, 1_u32), (0x18c, 0x100), (0x194, u32::MAX)] {
		image.sections[0].bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
	}
	assert!(image.msvc("CKickIssue", 8).is_empty());
}

#[test]
fn msvc_requires_unique_primary_locator_and_executable_slot() {
	let mut image = fixture();
	image.sections[0].bytes[0x110..0x121].copy_from_slice(b".?AVCKickIssue@@\0");
	for (offset, value) in [(0x180, 1_u32), (0x18c, 0x100), (0x194, 0x180)] {
		image.sections[0].bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
	}
	word(&mut image, 0x200, BASE + 0x180);
	word(&mut image, 0x208 + 8 * 8, 0x20010);
	assert_eq!(image.msvc("CKickIssue", 8), [BASE + 0x208]);
	word(&mut image, 0x280, BASE + 0x180);
	word(&mut image, 0x288 + 8 * 8, 0x20020);
	assert_eq!(image.msvc("CKickIssue", 8).len(), 2);
	#[cfg(target_os = "windows")]
	assert!(image.primary_vtable("CKickIssue", 8).is_none());
	word(&mut image, 0x208 + 8 * 8, 0x30000);
	assert_eq!(image.msvc("CKickIssue", 8), [BASE + 0x288]);
}

#[test]
fn retained_hook_trampolines_may_be_outside_the_game_image() {
	unsafe extern "C" fn trampoline() {}
	let mut image = fixture();
	word(&mut image, 0x100 + 8 * 8, trampoline as *const () as usize);
	assert!(image.valid_table(BASE + 0x100, 8));
	// An ordinary data allocation is never a valid replacement for code.
	let data = [0_u8; 32];
	word(&mut image, 0x100 + 8 * 8, data.as_ptr() as usize);
	assert!(!image.valid_table(BASE + 0x100, 8));
}

#[test]
fn vtable_records_cannot_use_executable_snapshot_bytes() {
	let mut image = fixture();
	image.sections[1].bytes[8 * 8..9 * 8].copy_from_slice(&0x20010_usize.to_le_bytes());
	assert!(!image.valid_table(0x20000, 8));
	word(&mut image, 0x100 + 8 * 8, 0x20010);
	assert!(image.valid_table(BASE + 0x100, 8));
}

fn word(image: &mut Image, offset: usize, value: usize) {
	image.sections[0].bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
