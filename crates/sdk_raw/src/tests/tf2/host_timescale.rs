//! Tests of the scan for the Windows engine's `host_timescale` gate.

use super::*;
use crate::util::Section;

const BASE: usize = 0x180000000;
const CHEATS: usize = BASE + 0x2100;
const DEMO: usize = BASE + 0x2200;
const TIMESCALE: usize = BASE + 0x2000;

#[test]
fn every_candidate_in_a_section_is_validated() {
	let mut code = section(0x100);
	code.bytes.extend(section(0x200).bytes);
	assert!(matches!(
		find(vec![code.clone()]),
		Err(GateError::AmbiguousGate)
	));

	// A signature match with the wrong variable must not hide the valid
	// candidate later in the same section or make it ambiguous.
	put_relative(&mut code.bytes, 3, CHEATS, code.address);
	assert_eq!(find(vec![code]).unwrap(), BASE + 0x200 + PATCH_OFFSET);
}

/// Finds the gate among `code` sections, in an image whose only data
/// section holds `CHEATS`, `DEMO`, and `TIMESCALE`.
fn find(mut sections: Vec<Section>) -> Result<usize, GateError> {
	sections.push(Section {
		address: BASE + 0x2000,
		bytes: vec![0; 0x1000],
		executable: false,
		writable: true,
	});

	find_gate(
		&Image {
			base: BASE,
			sections,
		},
		TIMESCALE,
		CHEATS,
	)
}

#[test]
fn parents_must_be_in_data_sections() {
	let image = Image {
		base: BASE,
		sections: vec![section(0x100)],
	};
	assert!(matches!(
		find_gate(&image, TIMESCALE, CHEATS),
		Err(GateError::UnsupportedEngine)
	));
	let code = BASE + 0x100;
	assert!(matches!(
		find_gate(&image, code, code),
		Err(GateError::UnsupportedEngine)
	));
}

/// Points the rel32 operand at `at`, in an instruction ending after it,
/// of code at `address`, to `target`.
fn put_relative(bytes: &mut [u8], at: usize, target: usize, address: usize) {
	let end = address + at + 4;
	let delta = i32::try_from(target as i128 - end as i128).unwrap();
	bytes[at..at + 4].copy_from_slice(&delta.to_le_bytes());
}

#[test]
fn rejection_branches_and_replacement_destination_are_checked() {
	for at in [13, 47] {
		let mut code = section(0x100);
		put_relative(&mut code.bytes, at, code.address + 199, code.address);
		assert!(matches!(
			find(vec![code]),
			Err(GateError::UnsupportedEngine)
		));
	}
	let mut code = section(0x100);
	code.bytes[PATCH_OFFSET + 1] = 0x1d;
	assert!(matches!(
		find(vec![code]),
		Err(GateError::UnsupportedEngine)
	));
}

#[test]
fn relocation_targets_must_be_the_registered_variables() {
	for (at, wrong) in [
		(3, CHEATS),
		(20, TIMESCALE),
		(54, CHEATS),
		(33, BASE + 0x100),
	] {
		let mut code = section(0x100);
		put_relative(&mut code.bytes, at, wrong, code.address);
		assert!(matches!(
			find(vec![code]),
			Err(GateError::UnsupportedEngine)
		));
	}
}

/// A 256-byte code section at `offset` from the image base, starting with
/// the gate.
fn section(offset: usize) -> Section {
	let address = BASE + offset;
	let mut bytes = vec![0x90; 256];
	for (out, input) in bytes.iter_mut().zip(PATTERN) {
		*out = match input {
			SignaturePattern::Exact(byte) => byte,
			SignaturePattern::Any => 0,
		};
	}
	put_relative(&mut bytes, 3, TIMESCALE, address);
	put_relative(&mut bytes, 20, CHEATS, address);
	put_relative(&mut bytes, 54, TIMESCALE, address);
	put_relative(&mut bytes, 33, DEMO, address);
	// Literal, so the fixture pins REJECT_OFFSET independently.
	put_relative(&mut bytes, 13, address + 198, address);
	put_relative(&mut bytes, 47, address + 198, address);
	Section {
		address,
		bytes,
		executable: true,
		writable: false,
	}
}

#[test]
fn selects_only_the_verified_gate() {
	assert_eq!(
		find(vec![section(0x100)]).unwrap(),
		BASE + 0x100 + PATCH_OFFSET
	);
	assert!(matches!(
		find(vec![section(0x100), section(0x500)]),
		Err(GateError::AmbiguousGate)
	));
}

#[test]
fn truncated_or_already_patched_code_is_refused() {
	let mut code = section(0x100);
	code.bytes.truncate(PATTERN_LEN);
	assert!(matches!(
		find(vec![code]),
		Err(GateError::UnsupportedEngine)
	));
	let mut code = section(0x100);
	code.bytes[PATCH_OFFSET] = REPLACEMENT;
	assert!(matches!(
		find(vec![code]),
		Err(GateError::UnsupportedEngine)
	));
}
