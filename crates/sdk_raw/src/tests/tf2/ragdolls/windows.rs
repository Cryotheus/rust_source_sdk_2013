//! Tests of the Windows resolution of `CreateServerRagdoll`: its signature,
//! and the agreement of its callers in `CBaseCombatCharacter`'s vtable, in a
//! synthetic image and against a retail `server.dll`.

use super::*;
use crate::util::{Section, SignaturePattern};

const BASE: usize = 0x180000000;

/// Where the synthetic `BecomeRagdoll` starts, directly followed by
/// `BecomeRagdollBoogie`, as in the retail module.
const BECOME_RAGDOLL: usize = CODE + 0x400;

/// Where the synthetic `BecomeRagdollBoogie` starts.
const BOOGIE: usize = BECOME_RAGDOLL + 0x200;

/// Where the synthetic image's executable section starts.
const CODE: usize = BASE + 0x10000;

/// Where the synthetic image's data section starts.
const DATA: usize = BASE + 0x1000;

/// Where the synthetic `CBaseCombatCharacter` vtable starts.
const TABLE: usize = DATA + 0x400;

/// Where the synthetic `CreateServerRagdoll` starts.
const TARGET: usize = CODE + 0x800;

#[test]
fn callers_in_the_vtable_must_agree_with_the_signature() {
	let mut image = fixture();

	assert_eq!(resolve_image(&image), Some(TARGET));

	// A second copy of the prologue makes the signature ambiguous.
	write_signature(&mut image, CODE + 0xc00);
	assert_eq!(resolve_image(&image), None);

	// Redirecting one caller elsewhere breaks the agreement.
	let mut image = fixture();

	write_call(&mut image, BOOGIE + 0x94, CODE + 0xc00);
	assert_eq!(resolve_image(&image), None);

	// A call in the next function does not count for `BecomeRagdoll`, even
	// within its window.
	let mut image = fixture();

	write_call(&mut image, BECOME_RAGDOLL + 0x1a3, CODE + 0xc00);
	write_call(&mut image, BOOGIE + 0x10, TARGET);
	assert_eq!(resolve_image(&image), None);

	// Both slots holding the same caller is no agreement either.
	let mut image = fixture();

	write_word(&mut image, TABLE + BECOME_RAGDOLL_SLOT * 8, BOOGIE);
	assert_eq!(resolve_with_table(&image, TABLE), None);
}

/// A module whose `CBaseCombatCharacter` vtable, found through its run-time
/// type information, holds callers each calling `CreateServerRagdoll` where
/// the retail ones do.
fn fixture() -> Image {
	let mut image = Image {
		base: BASE,
		sections: vec![
			Section {
				address: DATA,
				bytes: vec![0; 0x1000],
				executable: false,
				writable: false,
			},
			Section {
				address: CODE,
				bytes: vec![0xcc; 0x1000],
				executable: true,
				writable: false,
			},
		],
	};

	// The type descriptor's name, and the primary complete-object locator.
	let descriptor = DATA + 0x100;
	let locator = DATA + 0x180;
	let name = b".?AVCBaseCombatCharacter@@\0";
	let offset = descriptor + 16 - DATA;

	image.sections[0].bytes[offset..offset + name.len()].copy_from_slice(name);

	for (at, value) in [
		(locator, 1),
		(locator + 0xc, descriptor - BASE),
		(locator + 0x14, locator - BASE),
	] {
		let offset = at - DATA;
		let value = u32::try_from(value).unwrap();

		image.sections[0].bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
	}

	write_word(&mut image, TABLE - 8, locator);

	for slot in 0..=BECOME_RAGDOLL_BOOGIE_SLOT {
		write_word(&mut image, TABLE + slot * 8, CODE);
	}

	write_word(&mut image, TABLE + BECOME_RAGDOLL_SLOT * 8, BECOME_RAGDOLL);
	write_word(&mut image, TABLE + BECOME_RAGDOLL_BOOGIE_SLOT * 8, BOOGIE);
	write_signature(&mut image, TARGET);
	write_call(&mut image, BECOME_RAGDOLL + 0x1a3, TARGET);
	write_call(&mut image, BOOGIE + 0x94, TARGET);

	image
}

#[test]
#[ignore = "set TF2_SERVER_IMAGE to an authorized retail server.dll for binary validation"]
fn retail_create_server_ragdoll_callers() {
	let bytes =
		std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE")).unwrap();

	let image = crate::util::pe::from_file(&bytes).unwrap();
	let target = resolve_image(&image).expect("a unique prologue and agreeing callers");

	assert!(image.executable(target));
}

/// Writes `call target` at `at`.
fn write_call(image: &mut Image, at: usize, target: usize) {
	let displacement = i32::try_from(target as isize - (at + 5) as isize).unwrap();
	let offset = at - CODE;

	image.sections[1].bytes[offset] = CALL;
	image.sections[1].bytes[offset + 1..offset + 5].copy_from_slice(&displacement.to_le_bytes());
}

/// Writes [`CREATE_SERVER_RAGDOLL`] at `at`, with zeros for its wildcards.
fn write_signature(image: &mut Image, at: usize) {
	let offset = at - CODE;

	for (index, byte) in CREATE_SERVER_RAGDOLL.iter().enumerate() {
		image.sections[1].bytes[offset + index] = match *byte {
			SignaturePattern::Exact(byte) => byte,
			SignaturePattern::Any => 0,
		};
	}
}

/// Writes an address into the data section at `at`.
fn write_word(image: &mut Image, at: usize, value: usize) {
	let offset = at - DATA;

	image.sections[0].bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}
