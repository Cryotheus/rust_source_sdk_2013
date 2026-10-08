//! Tests of discovering primary vtables through run-time type information.

use super::*;
use crate::util::Section;

const BASE: usize = 0x10000;

#[test]
fn batched_itanium_searches_find_each_class_as_alone() {
	let mut image = fixture();
	image.sections[0].bytes[0x100..0x10d].copy_from_slice(b"10CKickIssue\0");
	image.sections[0].bytes[0x120..0x12b].copy_from_slice(b"9CBanIssue\0");
	word(&mut image, 0x188, BASE + 0x100);
	word(&mut image, 0x1a8, BASE + 0x120);
	word(&mut image, 0x208, BASE + 0x180);
	word(&mut image, 0x210 + 9 * 8, 0x20010);
	word(&mut image, 0x288, BASE + 0x1a0);
	word(&mut image, 0x290 + 9 * 8, 0x20020);
	let classes = ["CKickIssue", "CBanIssue", "CMissing", "", "CKick\0Issue"];
	assert_eq!(
		image.itanium_all(&classes, 9),
		[
			vec![BASE + 0x210],
			vec![BASE + 0x290],
			vec![],
			vec![],
			vec![]
		]
	);
	assert_eq!(image.itanium("CBanIssue", 9), [BASE + 0x290]);
	#[cfg(target_os = "linux")]
	assert_eq!(
		image.primary_vtables(&classes, 9),
		[Some(BASE + 0x210), Some(BASE + 0x290), None, None, None]
	);
}

#[test]
fn batched_msvc_searches_find_each_class_as_alone() {
	let mut image = fixture();
	image.sections[0].bytes[0x110..0x121].copy_from_slice(b".?AVCKickIssue@@\0");
	image.sections[0].bytes[0x130..0x140].copy_from_slice(b".?AVCBanIssue@@\0");
	for (offset, value) in [
		(0x180, 1_u32),
		(0x18c, 0x100),
		(0x194, 0x180),
		(0x1c0, 1),
		(0x1cc, 0x120),
		(0x1d4, 0x1c0),
	] {
		image.sections[0].bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
	}
	word(&mut image, 0x200, BASE + 0x180);
	word(&mut image, 0x208 + 8 * 8, 0x20010);
	word(&mut image, 0x300, BASE + 0x1c0);
	word(&mut image, 0x308 + 8 * 8, 0x20020);
	let classes = ["CKickIssue", "CBanIssue", "CMissing", ""];
	assert_eq!(
		image.msvc_all(&classes, 8),
		[vec![BASE + 0x208], vec![BASE + 0x308], vec![], vec![]]
	);
	assert_eq!(image.msvc("CBanIssue", 8), [BASE + 0x308]);
	#[cfg(target_os = "windows")]
	assert_eq!(
		image.primary_vtables(&classes, 8),
		[Some(BASE + 0x208), Some(BASE + 0x308), None, None]
	);
}

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
fn matches_any_tags_each_aligned_match_with_its_pattern() {
	let mut image = fixture();
	image.sections[0].bytes[0x40..0x44].copy_from_slice(b"abab");
	image.sections[0].bytes[0x51..0x53].copy_from_slice(b"ab");
	assert_eq!(
		image.matches_any(&[b"ab", b"aba", b""], 1),
		[
			(BASE + 0x40, 0),
			(BASE + 0x40, 1),
			(BASE + 0x42, 0),
			(BASE + 0x51, 0)
		]
	);
	assert_eq!(
		image.matches_any(&[b"ab"], 2),
		[(BASE + 0x40, 0), (BASE + 0x42, 0)]
	);
	assert_eq!(image.matches(b"ab", 4), [BASE + 0x40]);
	assert!(image.matches_any(&[b"ab"], 0).is_empty());
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
fn msvc_secondary_tables_carry_their_subobject_offsets() {
	let mut image = fixture();
	image.sections[0].bytes[0x110..0x121].copy_from_slice(b".?AVCKickIssue@@\0");
	// The primary locator, then that of a base 0x78 bytes into the object.
	for (offset, value) in [
		(0x180, 1_u32),
		(0x18c, 0x100),
		(0x194, 0x180),
		(0x1c0, 1),
		(0x1c4, 0x78),
		(0x1cc, 0x100),
		(0x1d4, 0x1c0),
	] {
		image.sections[0].bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
	}
	word(&mut image, 0x200, BASE + 0x180);
	word(&mut image, 0x208 + 8 * 8, 0x20010);
	word(&mut image, 0x300, BASE + 0x1c0);
	word(&mut image, 0x308 + 8 * 8, 0x20020);
	assert_eq!(image.msvc("CKickIssue", 8), [BASE + 0x208]);
	assert_eq!(
		image.msvc_tables("CKickIssue", 8),
		[(0, BASE + 0x208), (0x78, BASE + 0x308)]
	);
	#[cfg(target_os = "windows")]
	{
		assert_eq!(image.primary_vtable("CKickIssue", 8), Some(BASE + 0x208));
		assert_eq!(
			image.vtables("CKickIssue", 8),
			[(0, BASE + 0x208), (0x78, BASE + 0x308)]
		);
	}
	// A construction displacement makes the locator not a complete object's.
	image.sections[0].bytes[0x1c8..0x1cc].copy_from_slice(&1_u32.to_le_bytes());
	assert_eq!(image.msvc_tables("CKickIssue", 8), [(0, BASE + 0x208)]);
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
