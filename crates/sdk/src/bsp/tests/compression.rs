use super::super::{Bsp, Compression, Error, Invalid, Limits, Resource, entity_lump};
use super::{append_lump, put_i32, three_regions};

const ENTITY_EOS: &[u8] = &[
	76, 90, 77, 65, 5, 0, 0, 0, 15, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 0, 82, 105, 60, 201, 115, 7,
	255, 255, 253, 17, 224, 0,
];

const ENTITY_MARKERLESS: &[u8] = &[
	76, 90, 77, 65, 5, 0, 0, 0, 10, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 0, 82, 105, 60, 195, 7, 0, 0,
];

const LEAVES: &[u8] = &[
	76, 90, 77, 65, 64, 0, 0, 0, 34, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 104, 190, 77, 2, 71, 119, 226,
	100, 155, 117, 46, 113, 138, 105, 40, 129, 228, 186, 151, 70, 21, 57, 79, 248, 31, 139, 255,
	255, 238, 79, 0, 0,
];

const LEAVES_V0: &[u8] = &[
	76, 90, 77, 65, 112, 0, 0, 0, 35, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 104, 190, 77, 2, 71, 119,
	226, 100, 155, 117, 46, 113, 138, 105, 39, 168, 140, 165, 109, 125, 31, 88, 114, 79, 54, 69,
	179, 255, 255, 243, 187, 0, 0,
];

const MODEL: &[u8] = &[
	76, 90, 77, 65, 48, 0, 0, 0, 19, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 97, 255, 253, 62, 29, 5, 247,
	6, 173, 113, 93, 255, 255, 248, 126, 32, 0,
];

const NODE: &[u8] = &[
	76, 90, 77, 65, 32, 0, 0, 0, 24, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 104, 190, 81, 105, 67, 97, 18,
	29, 140, 97, 111, 212, 2, 156, 18, 139, 255, 255, 90, 0, 0, 0,
];

// Independent wire vectors: Python/liblzma raw LZMA1 with explicit EOS, 4 KiB
// dictionary and lc/lp/pb 3/0/2. No production encoder is used by these tests.
const PLANE: &[u8] = &[
	76, 90, 77, 65, 20, 0, 0, 0, 16, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 97, 254, 245, 254, 225, 115,
	8, 207, 255, 255, 222, 1, 0, 0,
];

#[test]
fn all_required_lumps_decode_from_independent_compressed_vectors() {
	for version in [19, 20] {
		let mut bytes = header(version);
		for (id, lump_version, stored) in
			[(1, 0, PLANE), (5, 0, NODE), (10, 1, LEAVES), (14, 0, MODEL)]
		{
			compressed(&mut bytes, id, lump_version, stored);
		}
		let bsp = Bsp::decode(&bytes, Limits::default()).unwrap();
		assert_eq!(
			(
				bsp.planes().len(),
				bsp.nodes().len(),
				bsp.leaves().len(),
				bsp.models().len()
			),
			(1, 1, 2, 1)
		);
		assert_eq!(bsp.locate([0.0, 0.0, 0.0]).unwrap(), 0);
		assert_eq!(bsp.locate([-1.0, 0.0, 0.0]).unwrap(), 1);
		assert_eq!(
			(
				bsp.leaves()[1].contents,
				bsp.leaves()[1].area,
				bsp.leaves()[1].flags
			),
			(1, 2, 3)
		);
		assert_eq!(bsp.world_partitions().unwrap().partitions().len(), 2);
		let limits = Limits {
			expanded_bytes: 163,
			..Limits::default()
		};
		assert_eq!(
			Bsp::decode(&bytes, limits).unwrap_err(),
			Error::Budget {
				resource: Resource::ExpandedBytes,
				limit: 163
			}
		);
	}
}

fn compressed(bytes: &mut Vec<u8>, id: usize, version: i32, stored: &[u8]) -> usize {
	let start = append_lump(bytes, id, stored);
	put_i32(bytes, 8 + id * 16 + 8, version);
	let expanded = i32::from_le_bytes(stored[4..8].try_into().unwrap());
	put_i32(bytes, 8 + id * 16 + 12, expanded);
	start
}

#[test]
fn compressed_and_raw_layouts_have_identical_geometry_and_queries() {
	let raw_plane: &[u8] = &[
		0, 0, 128, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	];
	let raw_node: &[u8] = &[
		0, 0, 0, 0, 255, 255, 255, 255, 254, 255, 255, 255, 192, 255, 192, 255, 192, 255, 64, 0,
		64, 0, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	];
	let raw_leaves: &[u8] = &[
		0, 0, 0, 0, 255, 255, 1, 6, 192, 255, 192, 255, 192, 255, 64, 0, 64, 0, 64, 0, 0, 0, 0, 0,
		0, 0, 0, 0, 255, 255, 210, 4, 1, 0, 0, 0, 255, 255, 2, 6, 192, 255, 192, 255, 192, 255, 64,
		0, 64, 0, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 210, 4,
	];
	let raw_model: &[u8] = &[
		0, 0, 128, 194, 0, 0, 128, 194, 0, 0, 128, 194, 0, 0, 128, 66, 0, 0, 128, 66, 0, 0, 128,
		66, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
	];
	let raw_leaves_v0: &[u8] = &[
		0, 0, 0, 0, 255, 255, 1, 6, 192, 255, 192, 255, 192, 255, 64, 0, 64, 0, 64, 0, 0, 0, 0, 0,
		0, 0, 0, 0, 255, 255, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171,
		171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 1, 0, 0, 0, 255, 255, 2, 6,
		192, 255, 192, 255, 192, 255, 64, 0, 64, 0, 64, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 255, 171,
		171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171, 171,
		171, 171, 171, 171, 171, 171, 171,
	];
	for version in [19, 20] {
		for leaf_version in [0, 1] {
			let mut plain = header(version);
			let mut encoded = header(version);
			let (leaf_raw, leaf_encoded) = if leaf_version == 0 {
				(raw_leaves_v0, LEAVES_V0)
			} else {
				(raw_leaves, LEAVES)
			};
			for (id, lump_version, bytes, stored) in [
				(1, 0, raw_plane, PLANE),
				(5, 0, raw_node, NODE),
				(10, leaf_version, leaf_raw, leaf_encoded),
				(14, 0, raw_model, MODEL),
			] {
				append_lump(&mut plain, id, bytes);
				put_i32(&mut plain, 8 + id * 16 + 8, lump_version);
				compressed(&mut encoded, id, lump_version, stored);
			}
			let plain = Bsp::decode(&plain, Limits::default()).unwrap();
			let encoded = Bsp::decode(&encoded, Limits::default()).unwrap();
			assert_eq!(plain.planes(), encoded.planes());
			assert_eq!(plain.nodes(), encoded.nodes());
			assert_eq!(plain.leaves(), encoded.leaves());
			assert_eq!(plain.models(), encoded.models());
			for point in [[0.0, 0.0, 0.0], [-1.0, 0.0, 0.0], [1.0, -50.0, 30.0]] {
				assert_eq!(plain.locate(point), encoded.locate(point));
			}
			let plain = plain.world_partitions().unwrap();
			let encoded = encoded.world_partitions().unwrap();
			assert_eq!(plain.partitions().len(), encoded.partitions().len());
			for id in 0..plain.partitions().len() {
				assert_eq!(plain.partitions()[id].leaf, encoded.partitions()[id].leaf);
				assert_eq!(
					plain.constraints(id).unwrap(),
					encoded.constraints(id).unwrap()
				);
			}
		}
	}
}

#[test]
fn compression_limits_are_checked_before_codec_allocation() {
	let bytes = entity(ENTITY_MARKERLESS);
	let defaults = Limits::default();
	for (limits, resource, limit) in [
		(
			Limits {
				expanded_bytes: 4,
				..defaults
			},
			Resource::ExpandedBytes,
			4,
		),
		(
			Limits {
				dictionary_bytes: 4095,
				..defaults
			},
			Resource::DictionaryBytes,
			4095,
		),
		(
			Limits {
				probability_bytes: 12287,
				..defaults
			},
			Resource::ProbabilityBytes,
			12287,
		),
		(
			Limits {
				codec_bytes: 39,
				..defaults
			},
			Resource::CodecBytes,
			39,
		),
		(
			Limits {
				work: 64,
				..defaults
			},
			Resource::Work,
			64,
		),
	] {
		assert_eq!(
			entity_lump(&bytes, limits).unwrap_err(),
			Error::Budget { resource, limit }
		);
	}
	let exact = Limits {
		input_bytes: bytes.len(),
		expanded_bytes: 5,
		dictionary_bytes: 4096,
		probability_bytes: 12288,
		codec_bytes: 40,
		work: 65,
		..defaults
	};
	assert_eq!(entity_lump(&bytes, exact).unwrap(), [0, 1, 2, 255, 0]);
	let mut bytes = bytes;
	put_i32(&mut bytes, 1049, 0);
	assert_eq!(entity_lump(&bytes, exact).unwrap(), [0, 1, 2, 255, 0]);
	assert_eq!(
		entity_lump(
			&bytes,
			Limits {
				dictionary_bytes: 4095,
				..exact
			}
		)
		.unwrap_err(),
		Error::Budget {
			resource: Resource::DictionaryBytes,
			limit: 4095
		}
	);
	bytes[1048] = 224; // lc=8, lp=4: six MiB literal table, despite tiny output.
	assert_eq!(
		entity_lump(&bytes, defaults).unwrap_err(),
		Error::Budget {
			resource: Resource::ProbabilityBytes,
			limit: defaults.probability_bytes
		}
	);
}

fn entity(stored: &[u8]) -> Vec<u8> {
	let mut bytes = header(20);
	compressed(&mut bytes, 0, 0, stored);
	bytes
}

#[test]
fn entity_bytes_are_owned_arbitrary_and_keep_terminal_nuls() {
	for version in [19, 20] {
		let mut bytes = header(version);
		assert!(entity_lump(&bytes, Limits::default()).unwrap().is_empty());
		append_lump(&mut bytes, 0, &[255, 0, 123, 0, 0]);
		let output = entity_lump(&bytes, Limits::default()).unwrap();
		bytes.fill(0);
		assert_eq!(output, [255, 0, 123, 0, 0]);
	}
	for stored in [ENTITY_EOS, ENTITY_MARKERLESS] {
		assert_eq!(
			entity_lump(&entity(stored), Limits::default()).unwrap(),
			[0, 1, 2, 255, 0]
		);
	}
}

#[test]
fn entity_version_and_all_directory_ranges_are_checked() {
	let mut bytes = entity(ENTITY_EOS);
	put_i32(&mut bytes, 16, 1);
	assert_eq!(
		entity_lump(&bytes, Limits::default()).unwrap_err(),
		Error::UnsupportedLumpVersion {
			lump: 0,
			version: 1
		}
	);
	put_i32(&mut bytes, 16, 0);
	put_i32(&mut bytes, 8 + 63 * 16, -1);
	assert_eq!(
		entity_lump(&bytes, Limits::default()).unwrap_err(),
		Error::Invalid(Invalid::LumpRange { lump: 63 })
	);
}

#[test]
fn eos_output_size_must_match_in_both_directions() {
	for length in [1, 4, 6, 10] {
		let mut stored = ENTITY_EOS.to_vec();
		put_i32(&mut stored, 4, length);
		assert!(entity_lump(&entity(&stored), Limits::default()).is_err());
	}
}

#[test]
fn every_truncated_or_extended_payload_is_rejected() {
	for stored in [ENTITY_EOS, ENTITY_MARKERLESS] {
		for length in 17..stored.len() {
			let mut shortened = stored[..length].to_vec();
			put_i32(&mut shortened, 8, (length - 17) as i32);
			assert!(
				entity_lump(&entity(&shortened), Limits::default()).is_err(),
				"accepted prefix {length}"
			);
		}
		for trailer in [&[0][..], &[255][..], &[0, 0, 0, 0][..]] {
			let mut extended = stored.to_vec();
			extended.extend_from_slice(trailer);
			let encoded = (extended.len() - 17) as i32;
			put_i32(&mut extended, 8, encoded);
			assert!(entity_lump(&entity(&extended), Limits::default()).is_err());
		}
	}
}

#[test]
fn expanded_record_size_and_record_budget_fail_before_decompression() {
	let mut data = three_regions();
	let mut stored = PLANE.to_vec();
	put_i32(&mut stored, 4, 21);
	compressed(&mut data.bytes, 1, 0, &stored);
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
		Error::Invalid(Invalid::RecordSize { lump: 1 })
	);
	put_i32(&mut stored, 4, 20_000_000);
	compressed(&mut data.bytes, 1, 0, &stored);
	let limits = Limits {
		records: 10,
		..Limits::default()
	};
	assert_eq!(
		Bsp::decode(&data.bytes, limits).unwrap_err(),
		Error::Budget {
			resource: Resource::Records,
			limit: 10
		}
	);
}

fn header(version: i32) -> Vec<u8> {
	let mut bytes = vec![0; 1036];
	bytes[..4].copy_from_slice(b"VBSP");
	put_i32(&mut bytes, 4, version);
	bytes
}

#[test]
fn independently_authored_markerless_zero_suffixes_use_declared_size() {
	// These literal-only range streams end at zero arithmetic code with no EOS.
	// In particular unknown-size decoding of zeros32 first stops at 31 bytes.
	assert_eq!(
		entity_lump(
			&entity(&[
				76, 90, 77, 65, 5, 0, 0, 0, 10, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 0, 82, 105, 60,
				195, 7, 0, 0
			]),
			Limits::default()
		)
		.unwrap(),
		&[0, 1, 2, 255, 0]
	);
	assert_eq!(
		entity_lump(
			&entity(&[
				76, 90, 77, 65, 20, 0, 0, 0, 21, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
				0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
			]),
			Limits::default()
		)
		.unwrap(),
		&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
	);
	assert_eq!(
		entity_lump(
			&entity(&[
				76, 90, 77, 65, 32, 0, 0, 0, 26, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
				0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
			]),
			Limits::default()
		)
		.unwrap(),
		&[
			0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
			0, 0, 0
		]
	);
	assert_eq!(
		entity_lump(
			&entity(&[
				76, 90, 77, 65, 48, 0, 0, 0, 31, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
				0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
			]),
			Limits::default()
		)
		.unwrap(),
		&[
			0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
			0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
		]
	);
	assert_eq!(
		entity_lump(
			&entity(&[
				76, 90, 77, 65, 56, 0, 0, 0, 33, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
				0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
			]),
			Limits::default()
		)
		.unwrap(),
		&[
			0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
			0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
		]
	);
	assert_eq!(
		entity_lump(
			&entity(&[
				76, 90, 77, 65, 20, 0, 0, 0, 22, 0, 0, 0, 93, 0, 16, 0, 0, 0, 0, 0, 21, 189, 178,
				225, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
			]),
			Limits::default()
		)
		.unwrap(),
		&[
			0, 0, 128, 63, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
		]
	);
}

#[test]
fn invalid_completion_never_falls_back_to_known_size() {
	// Independently inspected: known-size decoding alone produces the five
	// expected bytes and consumes all input but leaves arithmetic code == 1.
	// Unknown-size completion must fail, even though the output looks correct.
	let mut stored = ENTITY_MARKERLESS.to_vec();
	*stored.last_mut().unwrap() = 1;
	malformed(&entity(&stored), Compression::Stream);
}

fn malformed(bytes: &[u8], reason: Compression) {
	assert_eq!(
		entity_lump(bytes, Limits::default()).unwrap_err(),
		Error::Invalid(Invalid::Compression { lump: 0, reason })
	);
}

#[test]
fn mixed_raw_and_compressed_lumps_preserve_geometry_validation() {
	let mut data = three_regions();
	compressed(&mut data.bytes, 1, 0, PLANE);
	// The original second node refers to plane 1, absent from this one-plane lump.
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
		Error::Invalid(Invalid::Reference { lump: 5, record: 1 })
	);
	let mut data = three_regions();
	compressed(&mut data.bytes, 14, 0, MODEL);
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default())
			.unwrap()
			.leaves()
			.len(),
		3
	);
}

#[test]
#[ignore = "set SOURCE_BSP_ARENA_BYRE to the externally SHA-256-verified pinned public fixture"]
fn pinned_arena_byre_counts_tree_points_and_entities() {
	let path = std::env::var_os("SOURCE_BSP_ARENA_BYRE").expect("fixture path is required");
	let bytes = std::fs::read(path).expect("read local verified fixture; never download");
	assert_eq!(bytes.len(), 16843394);
	let bsp = Bsp::decode(&bytes, Limits::default()).unwrap();
	assert_eq!((bsp.version(), bsp.revision()), (20, 2059));
	assert_eq!(
		(
			bsp.planes().len(),
			bsp.nodes().len(),
			bsp.leaves().len(),
			bsp.models().len()
		),
		(12348, 1803, 1825, 21)
	);
	let parts = bsp.world_partitions().unwrap();
	assert_eq!(parts.partitions().len(), 1558);
	assert_eq!(
		parts.partitions().iter().map(|part| part.leaf).min(),
		Some(1)
	);
	assert_eq!(
		parts.partitions().iter().map(|part| part.leaf).max(),
		Some(1558)
	);
	assert_eq!(
		(0..parts.partitions().len())
			.map(|id| parts.constraints(id).unwrap().len())
			.max(),
		Some(21)
	);
	for (point, leaf, cluster, area) in [
		([-1984.0, -32.0, -208.0], 1216, 724, 5),
		([-1984.0, -160.0, -208.0], 1232, 737, 5),
		([-5120.0, 0.0, -1217.83], 1450, 843, 6),
	] {
		assert_eq!(bsp.locate(point).unwrap(), leaf);
		assert_eq!(
			(
				bsp.leaves()[leaf].contents,
				bsp.leaves()[leaf].cluster,
				bsp.leaves()[leaf].area
			),
			(0, cluster, area)
		);
	}
	let entities = entity_lump(&bytes, Limits::default()).unwrap();
	assert_eq!(entities.len(), 129933);
	assert_eq!(entities.last(), Some(&0));
}

#[test]
#[ignore = "set SOURCE_BSP_CTF_2FORT to the externally SHA-256-verified pinned public fixture"]
fn pinned_ctf_2fort_counts_tree_points_and_entities() {
	let path = std::env::var_os("SOURCE_BSP_CTF_2FORT").expect("fixture path is required");
	let bytes = std::fs::read(path).expect("read local verified fixture; never download");
	assert_eq!(bytes.len(), 22751863);
	let bsp = Bsp::decode(&bytes, Limits::default()).unwrap();
	assert_eq!((bsp.version(), bsp.revision()), (20, 4067));
	assert_eq!(
		(
			bsp.planes().len(),
			bsp.nodes().len(),
			bsp.leaves().len(),
			bsp.models().len()
		),
		(14474, 7822, 7971, 148)
	);
	let parts = bsp.world_partitions().unwrap();
	assert_eq!(parts.partitions().len(), 6337);
	assert_eq!(
		parts.partitions().iter().map(|part| part.leaf).min(),
		Some(1)
	);
	assert_eq!(
		parts.partitions().iter().map(|part| part.leaf).max(),
		Some(6337)
	);
	assert_eq!(
		(0..parts.partitions().len())
			.map(|id| parts.constraints(id).unwrap().len())
			.max(),
		Some(33)
	);
	for (point, leaf, cluster, area) in [
		([-1888.0, -1280.0, 276.0], 5974, 2370, 17),
		([1888.0, 1368.0, 276.0], 1579, 617, 8),
		([-4544.0, 256.0, 48.0], 3208, 1287, 11),
	] {
		assert_eq!(bsp.locate(point).unwrap(), leaf);
		assert_eq!(
			(
				bsp.leaves()[leaf].contents,
				bsp.leaves()[leaf].cluster,
				bsp.leaves()[leaf].area
			),
			(0, cluster, area)
		);
	}
	let entities = entity_lump(&bytes, Limits::default()).unwrap();
	assert_eq!(entities.len(), 314566);
	assert_eq!(entities.last(), Some(&0));
}

#[test]
fn raw_file_and_leaf_versions_are_independent() {
	for file_version in [19, 20] {
		for leaf_version in [0, 1] {
			let mut data = three_regions();
			put_i32(&mut data.bytes, 4, file_version);
			if leaf_version == 1 {
				let mut leaves = Vec::new();
				for at in [data.leaves, data.leaves + 56, data.leaves + 112] {
					leaves.extend_from_slice(&data.bytes[at..at + 30]);
					leaves.extend_from_slice(&[0xa5, 0x5a]);
				}
				append_lump(&mut data.bytes, 10, &leaves);
				put_i32(&mut data.bytes, 8 + 10 * 16 + 8, 1);
			}
			let bsp = Bsp::decode(&data.bytes, Limits::default()).unwrap();
			assert_eq!(bsp.version(), file_version);
			assert_eq!(bsp.leaves().len(), 3);
			assert_eq!(bsp.locate([1.0, 1.0, 0.0]).unwrap(), 0);
			assert_eq!(bsp.locate([-1.0, -1.0, 6.0]).unwrap(), 1);
			assert_eq!(bsp.locate([-1.0, -1.0, 4.0]).unwrap(), 2);
		}
	}
}

#[test]
fn selected_compression_header_fields_have_typed_errors() {
	let mut bytes = entity(ENTITY_EOS);
	put_i32(&mut bytes, 20, -1);
	malformed(&bytes, Compression::ExpandedSize);
	let mut bytes = entity(ENTITY_EOS);
	put_i32(&mut bytes, 12, 16);
	malformed(&bytes, Compression::Header);
	let mut bytes = entity(ENTITY_EOS);
	bytes[1036] = b'Z';
	assert_eq!(
		entity_lump(&bytes, Limits::default()).unwrap_err(),
		Error::UnsupportedCompression { lump: 0 }
	);
	let mut bytes = entity(ENTITY_EOS);
	put_i32(&mut bytes, 1040, 6);
	malformed(&bytes, Compression::SizeMismatch);
	for encoded in [0, 1, 1000] {
		let mut bytes = entity(ENTITY_EOS);
		put_i32(&mut bytes, 1044, encoded);
		malformed(&bytes, Compression::PayloadLength);
	}
	for (offset, value) in [(1048, 225), (1053, 1)] {
		let mut bytes = entity(ENTITY_EOS);
		bytes[offset] = value;
		malformed(&bytes, Compression::Properties);
	}
}
