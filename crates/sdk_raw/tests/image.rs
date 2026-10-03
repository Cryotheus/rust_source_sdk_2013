//! Tests of scanning image snapshots, and of decoding the x86 operands their
//! code refers to other code with.

use source_sdk_2013_raw::sig;
use source_sdk_2013_raw::util::{Image, Section, relative};

#[test]
fn relative_operands_are_signed_checked_and_bounded() {
	assert_eq!(
		relative(0x1000, &[0xe8, 0xf0, 0xff, 0xff, 0xff], 1),
		Some(0xff5)
	);
	assert_eq!(relative(0, &[0xe8, 0xf0, 0xff, 0xff, 0xff], 1), None);
	assert_eq!(relative(0x1000, &[0xe8, 0, 0], 1), None);
	assert_eq!(relative(usize::MAX, &[0; 4], 0), None);
}

#[test]
fn ranges_alignment_and_permissions_are_checked() {
	let mut image = Image {
		base: 1,
		sections: vec![Section {
			address: 1,
			bytes: vec![0; 33],
			executable: true,
			writable: false,
		}],
	};
	image.sections[0].bytes[15] = 0xff;
	assert_eq!(image.unique(&sig![0xff], 16), Some(16));
	assert!(!image.contains(1, 1, false, true));
	assert!(!image.contains(33, 2, false, false));
	assert!(!image.contains(usize::MAX, 2, false, false));
	assert!(image.matches(&[0xff], 1).is_empty());
	image.sections[0].executable = false;
	assert_eq!(image.matches(&[0xff], 16), [16]);
	assert!(image.matches(&[], 1).is_empty());
	assert!(image.matches(&[0xff], 0).is_empty());
	image.sections[0].address = usize::MAX;
	assert!(image.read(usize::MAX, 1).is_none());
	assert!(image.matches(&[0xff], 1).is_empty());
}

#[test]
fn signatures_reject_ambiguity_and_calls_outside_executable_sections() {
	let mut image = Image {
		base: 0x1000,
		sections: vec![Section {
			address: 0x1000,
			bytes: vec![0x90; 256],
			executable: true,
			writable: false,
		}],
	};
	image.sections[0].bytes[0x10..0x13].copy_from_slice(&[0x48, 0x89, 0xff]);
	assert!(image.unique(&sig![], 16).is_none());
	assert!(image.unique(&sig![0x48 ? 0xff], 0).is_none());
	assert_eq!(image.unique(&sig![0x48 ? 0xff], 16), Some(0x1010));
	image.sections[0].bytes[0x20..0x23].copy_from_slice(&[0x48, 0x89, 0xff]);
	assert!(image.unique(&sig![0x48 ? 0xff], 16).is_none());
	image.sections[0].bytes[0x30..0x35].copy_from_slice(&[0xe8, 0, 0, 0, 0]);
	assert_eq!(image.call(0x1030), Some(0x1035));
	image.sections[0].bytes[0x31..0x35].copy_from_slice(&0x1000_i32.to_le_bytes());
	assert!(image.call(0x1030).is_none());
}
