//! Tests of the scan for the `god` command's gate.

use super::*;

const BASE: usize = 0x180000000;

/// Where the gate starts in the callback, as in TF2's `CC_God_f`.
const GATE_AT: usize = 0x3b;

const GLOBALS: usize = BASE + 0x2100;

/// Where the `god` command's callback starts, in [`code`].
const GOD: usize = BASE + 0x100;

/// Where `IsPVEModeActive`'s code starts, after the callback, in [`code`].
const IS_PVE_MODE_ACTIVE_AT: usize = BASE + 0x400;

const RULES: usize = BASE + 0x2000;

#[test]
fn a_patched_gate_is_refused() {
	let mut code = code();
	code.bytes[GATE_AT + PATCH_OFFSET] = REPLACEMENT;
	assert!(matches!(find(code), Err(GodError::UnsupportedGame)));
}

/// A code section starting with the callback, whose gate starts at
/// [`GATE_AT`] and calls `IsPVEModeActive`, which follows it.
fn code() -> Section {
	let mut bytes = vec![0xcc; 0x400];
	put_gate(&mut bytes, GATE_AT);

	for (out, input) in bytes[IS_PVE_MODE_ACTIVE_AT - GOD..]
		.iter_mut()
		.zip(IS_PVE_MODE_ACTIVE)
	{
		*out = match input {
			SignaturePattern::Exact(byte) => byte,
			SignaturePattern::Any => 0x26,
		};
	}

	Section {
		address: GOD,
		bytes,
		executable: true,
		writable: false,
	}
}

#[test]
fn every_gate_in_the_callback_is_validated() {
	let mut code = code();
	put_gate(&mut code.bytes, GATE_AT + GATE_LEN);
	assert!(matches!(find(code.clone()), Err(GodError::AmbiguousGate)));

	// A gate with the wrong reference must not hide the valid one or make it
	// ambiguous.
	put_relative(&mut code.bytes, GATE_AT + 3, GOD, GOD);
	assert_eq!(find(code).unwrap(), GOD + GATE_AT + GATE_LEN);
}

/// Finds the gate in the callback at [`GOD`] in `code`, in an image whose
/// only data section holds [`GLOBALS`] and [`RULES`].
fn find(code: Section) -> Result<usize, GodError> {
	find_gate(&image(code), GOD)
}

#[test]
fn finds_the_gate() {
	assert_eq!(find(code()).unwrap(), GOD + GATE_AT);
}

#[test]
fn finds_the_gate_in_tf2s_windows_build() {
	// `CC_God_f` and `CTFGameRules::IsPVEModeActive`, as one build of TF2's
	// 64-bit Windows server.dll loads them at its preferred base, and the data
	// section holding `g_pGameRules` and `gpGlobals`. Other builds move them.
	const CC_GOD_F: usize = 0x180290d20;
	const IS_PVE_MODE_ACTIVE_CODE: usize = 0x180570ad0;

	#[rustfmt::skip]
	let god = vec![
		0x48, 0x83, 0xec, 0x48, 0x48, 0x8b, 0x05, 0x35, 0x76, 0xaf, 0x00, 0x48, 0x8b, 0x48, 0x38, 0x83,
		0x79, 0x58, 0x00, 0x0f, 0x84, 0x8f, 0x00, 0x00, 0x00, 0x48, 0x89, 0x5c, 0x24, 0x40, 0xe8, 0xdd,
		0x87, 0x05, 0x00, 0x48, 0x8b, 0xd8, 0x48, 0x85, 0xc0, 0x74, 0x78, 0x48, 0x8b, 0x10, 0x48, 0x8b,
		0xc8, 0xff, 0x92, 0x98, 0x02, 0x00, 0x00, 0x84, 0xc0, 0x74, 0x68, 0x48, 0x8b, 0x0d, 0xe6, 0x73,
		0xb4, 0x00, 0x48, 0x85, 0xc9, 0x74, 0x16, 0xe8, 0x64, 0xfd, 0x2d, 0x00, 0x84, 0xc0, 0x75, 0x0d,
		0x48, 0x8b, 0x05, 0xe9, 0x5e, 0xb4, 0x00, 0x80, 0x78, 0x65, 0x00, 0x75, 0x46, 0xba, 0x00, 0x80,
		0x00, 0x00, 0x48, 0x8b, 0xcb, 0xe8, 0x36, 0x57, 0xfd, 0xff, 0x33, 0xc0, 0x4c, 0x8d, 0x05, 0x2d,
		0x91, 0x69, 0x00, 0x48, 0x89, 0x44, 0x24, 0x30, 0x45, 0x33, 0xc9, 0xf7, 0x83, 0xf4, 0x01, 0x00,
		0x00, 0x00, 0x80, 0x00, 0x00, 0x48, 0x8b, 0xcb, 0x48, 0x89, 0x44, 0x24, 0x28, 0x8d, 0x50, 0x02,
		0x48, 0x89, 0x44, 0x24, 0x20, 0x74, 0x07, 0x4c, 0x8d, 0x05, 0x12, 0x91, 0x69, 0x00, 0xe8, 0x8d,
		0xbe, 0x17, 0x00, 0x48, 0x8b, 0x5c, 0x24, 0x40, 0x48, 0x83, 0xc4, 0x48, 0xc3, 0xcc, 0xcc, 0xcc,
	];

	#[rustfmt::skip]
	let is_pve_mode_active = vec![
		0x80, 0xb9, 0x26, 0x0d, 0x00, 0x00, 0x00, 0x0f, 0x95, 0xc0, 0xc3, 0xcc, 0xcc, 0xcc, 0xcc, 0xcc,
	];

	let image = Image {
		base: 0x180000000,
		sections: vec![
			Section {
				address: CC_GOD_F,
				bytes: god,
				executable: true,
				writable: false,
			},
			Section {
				address: IS_PVE_MODE_ACTIVE_CODE,
				bytes: is_pve_mode_active,
				executable: true,
				writable: false,
			},
			Section {
				address: 0x180d80000,
				bytes: vec![0; 0x60000],
				executable: false,
				writable: true,
			},
		],
	};

	assert_eq!(find_gate(&image, CC_GOD_F).unwrap(), CC_GOD_F + 0x3b);
	assert_eq!(image.sections[0].bytes[0x3b + PATCH_OFFSET], ORIGINAL);
}

#[test]
fn gates_past_the_start_of_the_callback_are_ignored() {
	let mut code = code();
	code.bytes[GATE_AT..GATE_AT + GATE_LEN].fill(0xcc);
	put_gate(&mut code.bytes, GATE_SEARCH_LEN);
	assert!(matches!(find(code.clone()), Err(GodError::UnsupportedGame)));

	code.bytes[GATE_SEARCH_LEN..GATE_SEARCH_LEN + GATE_LEN].fill(0xcc);
	put_gate(&mut code.bytes, GATE_SEARCH_LEN - 1);
	assert_eq!(find(code).unwrap(), GOD + GATE_SEARCH_LEN - 1);
}

/// An image of `code` and a data section holding [`GLOBALS`] and [`RULES`].
fn image(code: Section) -> Image {
	Image {
		base: BASE,
		sections: vec![
			code,
			Section {
				address: BASE + 0x2000,
				bytes: vec![0; 0x1000],
				executable: false,
				writable: true,
			},
		],
	}
}

/// Writes the gate at offset `at` of the callback, whose code starts at
/// [`GOD`], calling `IsPVEModeActive` at [`IS_PVE_MODE_ACTIVE_AT`].
fn put_gate(bytes: &mut [u8], at: usize) {
	for (out, input) in bytes[at..].iter_mut().zip(GATE) {
		*out = match input {
			SignaturePattern::Exact(byte) => byte,
			SignaturePattern::Any => 0,
		};
	}

	put_relative(bytes, at + 3, RULES, GOD);
	put_relative(bytes, at + 13, IS_PVE_MODE_ACTIVE_AT, GOD);
	put_relative(bytes, at + 24, GLOBALS, GOD);
}

/// Points the rel32 operand at `at`, in an instruction ending after it, of
/// code at `address`, to `target`.
fn put_relative(bytes: &mut [u8], at: usize, target: usize, address: usize) {
	let end = address + at + 4;
	let delta = i32::try_from(target as i128 - end as i128).unwrap();
	bytes[at..at + 4].copy_from_slice(&delta.to_le_bytes());
}

#[test]
fn references_must_be_the_rules_and_globals_in_data() {
	for at in [3, 24] {
		let mut code = code();
		put_relative(&mut code.bytes, GATE_AT + at, GOD, GOD);
		assert!(matches!(find(code), Err(GodError::UnsupportedGame)));
	}
}

#[test]
fn the_callback_must_be_code() {
	assert!(matches!(
		find_gate(&image(code()), RULES),
		Err(GodError::UnsupportedGame)
	));
	assert!(matches!(
		find_gate(&image(code()), BASE + 0x3000),
		Err(GodError::UnsupportedGame)
	));
}

#[test]
fn the_gate_must_call_is_pve_mode_active() {
	for target in [IS_PVE_MODE_ACTIVE_AT + 1, RULES, BASE + 0x3000] {
		let mut code = code();
		put_relative(&mut code.bytes, GATE_AT + 13, target, GOD);
		assert!(matches!(find(code), Err(GodError::UnsupportedGame)));
	}
}

#[test]
fn truncated_code_is_refused() {
	let mut code = code();
	code.bytes.truncate(GATE_AT + GATE_LEN - 1);
	assert!(matches!(find(code), Err(GodError::UnsupportedGame)));

	let mut code = self::code();
	code.bytes
		.truncate(IS_PVE_MODE_ACTIVE_AT - GOD + IS_PVE_MODE_ACTIVE_LEN - 1);
	assert!(matches!(find(code), Err(GodError::UnsupportedGame)));
}
