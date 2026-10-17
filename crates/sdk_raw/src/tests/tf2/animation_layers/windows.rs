//! Tests of the Windows resolution of `RestartGesture`: its signature in a
//! synthetic image, and against a retail `server.dll`.

use super::*;
use crate::util::Section;

const BASE: usize = 0x180000000;

/// Where the synthetic image's executable section starts.
const CODE: usize = BASE + 0x10000;

#[test]
fn an_entry_a_detour_left_is_still_found() {
	let mut image = fixture();

	write_signature(&mut image, CODE + 0x100);
	image.sections[0].bytes[0x100..0x105].copy_from_slice(&[0xe9, 0x55, 0xc4, 0xa3, 0xff]);
	assert_eq!(resolve_image(&image), Some(CODE + 0x100));

	// The jump covers only the first instruction.
	image.sections[0].bytes[0x105] ^= 0xff;
	assert_eq!(resolve_image(&image), None);
}

fn fixture() -> Image {
	Image {
		base: BASE,
		sections: vec![Section {
			address: CODE,
			bytes: vec![0xcc; 0x1000],
			executable: true,
			writable: false,
		}],
	}
}

#[test]
#[ignore = "set TF2_SERVER_IMAGE to an authorized retail server.dll for binary validation"]
fn retail_restart_gesture_signature() {
	let bytes =
		std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE")).unwrap();

	let image = crate::util::pe::from_file(&bytes).unwrap();
	let target = resolve_image(&image).expect("a unique start");

	assert!(image.executable(target));
}

#[test]
fn the_signature_must_match_one_function_entry() {
	let mut image = fixture();

	write_signature(&mut image, CODE + 0x100);
	assert_eq!(resolve_image(&image), Some(CODE + 0x100));

	// A second copy makes the signature ambiguous.
	write_signature(&mut image, CODE + 0x300);
	assert_eq!(resolve_image(&image), None);

	// A copy that is not at a function's entry does not count.
	let mut image = fixture();

	write_signature(&mut image, CODE + 0x108);
	assert_eq!(resolve_image(&image), None);
}

/// Writes [`RESTART_GESTURE`] at `at`, with zeros for its wildcards.
fn write_signature(image: &mut Image, at: usize) {
	let offset = at - CODE;

	for (index, byte) in RESTART_GESTURE.iter().enumerate() {
		image.sections[0].bytes[offset + index] = match *byte {
			SignaturePattern::Exact(byte) => byte,
			SignaturePattern::Any => 0,
		};
	}
}
