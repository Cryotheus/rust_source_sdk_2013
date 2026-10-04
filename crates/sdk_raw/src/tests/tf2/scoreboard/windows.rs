//! Validation of the Windows signatures, operands, calls, and run-time type
//! information that locate the game statistics, on a synthetic image built
//! from bytes of retail `server.dll` and against a retail `server.dll`.

use super::*;
use crate::tf2::scoreboard::{GameStatsError, ModuleKey, lookup};
use crate::util::Section;

const BASE: usize = 0x180000000;
const CALC: usize = TEXT + 0x100;
const DATA: usize = BASE + 0x20000;
const FIND: usize = TEXT + 0x600;
const FIND_BODY: usize = TEXT + 0x400;
const INCREMENT: usize = TEXT + 0xa00;
const INSTANCE: usize = DATA + 0x40;
const MINIGAME_NAME: usize = RDATA + 0x10;
const RDATA: usize = BASE + 0x10000;

/// `CTFGameStats::FindPlayerStats` of retail `server.dll`, up to its first
/// return of a block.
const RETAIL_FIND_PLAYER_STATS: [u8; 44] = [
	0x4c, 0x8b, 0xc1, 0x48, 0x85, 0xd2, 0x75, 0x03, 0x33, 0xc0, 0xc3, 0x48, 0x8b, 0x42, 0x30, 0x48,
	0x85, 0xc0, 0x74, 0x18, 0x0f, 0xbf, 0x48, 0x06, 0x48, 0x63, 0xc1, 0x48, 0x69, 0xc0, 0x94, 0x07,
	0x00, 0x00, 0x48, 0x05, 0xd8, 0x00, 0x00, 0x00, 0x49, 0x03, 0xc0, 0xc3,
];

/// `CTFGameStats::IncrementStat`'s additions to a block, from retail
/// `server.dll`: stride 0x794, array 0xd8, then the life, round (0xb4), three
/// map (0x21c, 0x224, 0x22c), and session (0x168) blocks, out of order.
const RETAIL_INCREMENT_STAT: [u8; 55] = [
	0x48, 0x69, 0xd0, 0x94, 0x07, 0x00, 0x00, 0x48, 0x81, 0xc2, 0xd8, 0x00, 0x00, 0x00, 0x48, 0x03,
	0xd5, 0x01, 0x1c, 0xb2, 0x01, 0x9c, 0xb2, 0xb4, 0x00, 0x00, 0x00, 0x01, 0x9c, 0xb2, 0x1c, 0x02,
	0x00, 0x00, 0x01, 0x9c, 0xb2, 0x24, 0x02, 0x00, 0x00, 0x01, 0x9c, 0xb2, 0x68, 0x01, 0x00, 0x00,
	0x01, 0x9c, 0xb2, 0x2c, 0x02, 0x00, 0x00,
];

const SCORE: usize = FIND + 0x2cb;
const TEXT: usize = BASE + 0x1000;
const TYPE_NAME: usize = RDATA + 0x100;

/// Adds a locator at `locator` of the subobject at `offset`, and a vtable of
/// it at `table`, whose first entry is executable.
fn add_vtable(image: &mut Image, locator: usize, offset: usize, table: usize) {
	let rva = |address: usize| ((address - BASE) as u32).to_le_bytes();

	put(image, locator, &1_u32.to_le_bytes());
	put(image, locator + 4, &(offset as u32).to_le_bytes());
	put(image, locator + 12, &rva(TYPE_NAME - 16));
	put(image, locator + 20, &rva(locator));
	put(image, table - 8, &locator.to_le_bytes());
	put(image, table, &TEXT.to_le_bytes());
}

#[test]
fn agreeing_code_operands_and_vtables_locate_the_singleton() {
	assert_eq!(
		resolve_image(&fixture()),
		Some(Addresses {
			calc_player_score: CALC,
			find_player_stats: FIND_BODY,
			instance: NonZeroUsize::new(INSTANCE).unwrap(),
			player_stats: Some(0xd8),
			size: 0xd8 + 102 * 0x794,
		})
	);
}

/// The bytes `signature` matches, with each wildcard 0.
fn filled(signature: &[SignaturePattern]) -> Vec<u8> {
	signature
		.iter()
		.map(|byte| match byte {
			SignaturePattern::Exact(byte) => *byte,
			SignaturePattern::Any => 0,
		})
		.collect()
}

/// A synthetic module whose code, strings, run-time type information, and
/// constructed singleton are laid out as retail `server.dll`'s.
fn fixture() -> Image {
	let mut image = Image {
		base: BASE,
		sections: vec![
			Section {
				address: TEXT,
				bytes: vec![0xcc; 0x1000],
				executable: true,
				writable: false,
			},
			Section {
				address: RDATA,
				bytes: vec![0; 0x1000],
				executable: false,
				writable: false,
			},
			Section {
				address: DATA,
				bytes: vec![0; 0x40 + GAME_STATS_SIZE],
				executable: false,
				writable: true,
			},
		],
	};

	// `CalcPlayerScore`, which loads the attribute's name.
	put(&mut image, CALC, &filled(CALC_PLAYER_SCORE));
	put(
		&mut image,
		CALC + CALC_PLAYER_SCORE_MINIGAME,
		&[0x48, 0x8d, 0x15],
	);
	put_relative(
		&mut image,
		CALC + CALC_PLAYER_SCORE_MINIGAME + 3,
		MINIGAME_NAME,
	);
	put(&mut image, MINIGAME_NAME, MINIGAME_ATTRIBUTE);

	// `UpdateConnectedPlayer` finds the block in the singleton, then scores it.
	put(&mut image, FIND, &filled(FIND_PLAYER_STATS_CALL));
	put_relative(&mut image, FIND + FIND_PLAYER_STATS_CALL_INSTANCE, INSTANCE);
	put_relative(&mut image, FIND + FIND_PLAYER_STATS_CALL_AT + 1, FIND_BODY);
	put(&mut image, SCORE, &filled(CALC_ACCUMULATED_SCORE));
	put_relative(&mut image, SCORE + CALC_ACCUMULATED_SCORE_CALL + 1, CALC);
	put(&mut image, FIND_BODY, &RETAIL_FIND_PLAYER_STATS);
	put(&mut image, INCREMENT, &RETAIL_INCREMENT_STAT);

	// The class's type descriptor, then a locator and a vtable for each of its
	// subobjects, whose pointers the constructed singleton holds.
	put(&mut image, TYPE_NAME, b".?AVCTFGameStats@@\0");

	for (index, offset) in GAME_STATS_VTABLES.into_iter().enumerate() {
		let locator = RDATA + 0x200 + index * 0x20;

		add_vtable(&mut image, locator, offset, table(offset));
		put(&mut image, INSTANCE + offset, &table(offset).to_le_bytes());
	}

	image
}

#[test]
fn layout_operands_must_be_the_declared_layout() {
	// `IncrementStat`'s stride, array, and the round and session blocks among
	// its five block displacements.
	for (at, value) in [(3, 0x798_u32), (10, 0xe0), (23, 0x21c), (44, 0x224)] {
		let mut image = fixture();

		put(&mut image, INCREMENT + at, &value.to_le_bytes());
		assert!(
			resolve_image(&image).is_none(),
			"IncrementStat operand {at}"
		);
	}

	// The session and round blocks in the other order.
	let mut image = fixture();

	put(&mut image, INCREMENT + 23, &0x168_u32.to_le_bytes());
	put(&mut image, INCREMENT + 44, &0xb4_u32.to_le_bytes());
	assert!(resolve_image(&image).is_none());

	// `FindPlayerStats`'s stride and array.
	for at in [FIND_PLAYER_STATS_STRIDE, FIND_PLAYER_STATS_BASE] {
		let mut image = fixture();

		put(&mut image, FIND_BODY + at, &0x100_u32.to_le_bytes());
		assert!(
			resolve_image(&image).is_none(),
			"FindPlayerStats operand {at}"
		);
	}
}

fn put(image: &mut Image, address: usize, bytes: &[u8]) {
	let section = image
		.sections
		.iter_mut()
		.find(|section| (section.address..section.address + section.bytes.len()).contains(&address))
		.unwrap();
	let offset = address - section.address;

	section.bytes[offset..offset + bytes.len()].copy_from_slice(bytes);
}

/// Writes at `operand` the `rip`-relative displacement of `target`, for an
/// instruction that ends with the displacement.
fn put_relative(image: &mut Image, operand: usize, target: usize) {
	let displacement = i32::try_from(target as isize - (operand + 4) as isize).unwrap();

	put(image, operand, &displacement.to_le_bytes());
}

#[test]
#[ignore = "set TF2_SERVER_IMAGE to an authorized retail server.dll for binary validation"]
fn retail_game_stats_resolution() {
	let bytes =
		std::fs::read(std::env::var_os("TF2_SERVER_IMAGE").expect("TF2_SERVER_IMAGE")).unwrap();

	let mut image = crate::util::pe::from_file(&bytes).unwrap();

	// The file leaves the singleton zeroed; construct its vtable pointers as
	// the constructor does, from the tables run-time type information finds.
	let find = image.unique(FIND_PLAYER_STATS_CALL, 1).unwrap();
	let call = image.read(find, FIND_PLAYER_STATS_CALL.len()).unwrap();
	let instance = relative(find, call, FIND_PLAYER_STATS_CALL_INSTANCE).unwrap();
	let tables = image.vtables("CTFGameStats", 0);

	assert_eq!(
		tables.iter().map(|(offset, _)| *offset).collect::<Vec<_>>(),
		GAME_STATS_VTABLES
	);

	for (offset, table) in tables {
		put(&mut image, instance + offset, &table.to_le_bytes());
	}

	let addresses = resolve_image(&image).expect("retail signatures, operands, and vtables");

	assert_eq!(addresses.instance.get(), instance);
	assert!(image.executable(addresses.calc_player_score));
	assert!(image.executable(addresses.find_player_stats));

	// Resolving the same module twice through the cache inspects it once.
	// A file has no loader to find its factory by, so another of its
	// addresses stands in for it.
	let key = ModuleKey {
		base: image.base,
		factory: addresses.calc_player_score,
	};

	assert_eq!(
		lookup(key, || resolve_image(&image)
			.ok_or(GameStatsError::Unresolved)),
		Ok(addresses)
	);
	assert_eq!(
		lookup(key, || panic!("a cache hit inspected the module again")),
		Ok(addresses)
	);

	// Matching signatures alone are insufficient: redirect the scoring call to
	// another executable function and require the cross-check to reject it.
	let score = image.unique(CALC_ACCUMULATED_SCORE, 1).unwrap() + CALC_ACCUMULATED_SCORE_CALL;

	put_relative(&mut image, score + 1, addresses.find_player_stats);
	assert!(resolve_image(&image).is_none());
}

/// The vtable of the subobject at `offset`, in the synthetic image.
fn table(offset: usize) -> usize {
	RDATA + 0x408 + offset * 2
}

#[test]
fn the_function_calls_and_string_must_agree() {
	// `UpdateConnectedPlayer` scoring the block with another function.
	let mut image = fixture();

	put_relative(
		&mut image,
		SCORE + CALC_ACCUMULATED_SCORE_CALL + 1,
		FIND_BODY,
	);
	assert!(resolve_image(&image).is_none());

	// `CalcPlayerScore` reading another attribute.
	let mut image = fixture();

	put(&mut image, MINIGAME_NAME, b"scoreboard_minigamf\0");
	assert!(resolve_image(&image).is_none());

	// The two calls in different functions.
	let mut image = fixture();

	put(&mut image, SCORE, &[0xcc; 15]);
	put(&mut image, FIND + 0x500, &filled(CALC_ACCUMULATED_SCORE));
	put_relative(
		&mut image,
		FIND + 0x500 + CALC_ACCUMULATED_SCORE_CALL + 1,
		CALC,
	);
	assert!(resolve_image(&image).is_none());
}

#[test]
fn the_singleton_must_hold_each_vtable_of_one_class() {
	// A missing secondary vtable.
	let mut image = fixture();

	put(&mut image, INSTANCE + 0x88, &0_usize.to_le_bytes());
	assert!(resolve_image(&image).is_none());

	// The tables in each other's place.
	let mut image = fixture();

	put(&mut image, INSTANCE + 0x78, &table(0x88).to_le_bytes());
	put(&mut image, INSTANCE + 0x88, &table(0x78).to_le_bytes());
	assert!(resolve_image(&image).is_none());

	// A second table for one subobject makes it ambiguous.
	let mut image = fixture();

	add_vtable(&mut image, RDATA + 0x300, 0x78, RDATA + 0x808);
	assert!(resolve_image(&image).is_none());

	// A singleton too small for its array.
	let mut image = fixture();

	image.sections[2].bytes.truncate(0x40 + GAME_STATS_SIZE - 4);
	assert!(resolve_image(&image).is_none());
}
