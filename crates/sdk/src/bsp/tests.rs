mod compression;

use super::{Bsp, Child, Coverage, Error, HalfSpace, Invalid, Limits, Resource, Side};

struct Fixture {
	bytes: Vec<u8>,
	planes: usize,
	nodes: usize,
	leaves: usize,
	models: usize,
}

#[test]
fn a_leaf_root_has_no_invented_enclosing_planes() {
	let bsp = decode(&fixture(&[], &[], 1, -1));
	assert_eq!(bsp.locate([f32::MAX; 3]).unwrap(), 0);
	let parts = bsp.world_partitions().unwrap();
	assert_eq!(parts.partitions().len(), 1);
	assert!(parts.constraints(0).unwrap().is_empty());
}

#[test]
fn all_directory_ranges_are_checked_including_unused_lumps() {
	for (id, field, value) in [
		(1, 0, -1),
		(1, 4, -1),
		(1, 0, 1000),
		(63, 0, i32::MAX),
		(63, 4, i32::MAX),
	] {
		let mut data = three_regions();
		put_i32(&mut data.bytes, 8 + id * 16 + field, value);
		assert_eq!(
			Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
			Error::Invalid(Invalid::LumpRange { lump: id })
		);
	}
}

fn append_lump(bytes: &mut Vec<u8>, id: usize, data: &[u8]) -> usize {
	let start = bytes.len();
	put_i32(bytes, 8 + id * 16, start as i32);
	put_i32(bytes, 12 + id * 16, data.len() as i32);
	bytes.extend_from_slice(data);
	start
}

#[test]
fn cycles_fail_even_when_not_reachable_from_the_world_root() {
	let mut data = three_regions();
	put_i32(&mut data.bytes, data.nodes + 32 + 4, 0);
	assert!(matches!(
		Bsp::decode(&data.bytes, Limits::default()),
		Err(Error::Invalid(Invalid::Cycle { .. }))
	));
	put_i32(&mut data.bytes, data.models + 36, -1);
	assert!(matches!(
		Bsp::decode(&data.bytes, Limits::default()),
		Err(Error::Invalid(Invalid::Cycle { .. }))
	));
}

fn decode(fixture: &Fixture) -> Bsp {
	Bsp::decode(&fixture.bytes, Limits::default()).unwrap()
}

#[test]
fn finite_f32_coefficients_are_evaluated_without_f32_product_overflow() {
	let bsp = decode(&fixture(
		&[[f32::MAX, f32::MAX, 0.0, 0.0]],
		&[(0, [-1, -2])],
		2,
		0,
	));
	let point = [f32::MAX, -f32::MAX, 0.0];
	assert_eq!(bsp.locate(point).unwrap(), 0);
	assert!(
		bsp.matches(
			&[HalfSpace {
				plane: 0,
				side: Side::Front
			}],
			point
		)
		.unwrap()
	);
	assert!(
		!bsp.matches(
			&[HalfSpace {
				plane: 0,
				side: Side::Back
			}],
			point
		)
		.unwrap()
	);
}

// Authored wire records use literal widths and offsets from the public format,
// independently of the production decoder's constants.
fn fixture(planes: &[[f32; 4]], nodes: &[(i32, [i32; 2])], leaves: usize, root: i32) -> Fixture {
	let mut bytes = vec![0; 1036];
	bytes[..4].copy_from_slice(b"VBSP");
	put_i32(&mut bytes, 4, 19);
	put_i32(&mut bytes, 1032, 7);

	let mut plane_data = Vec::new();
	for plane in planes {
		for value in plane {
			plane_data.extend_from_slice(&value.to_le_bytes());
		}
		plane_data.extend_from_slice(&0i32.to_le_bytes());
	}
	let planes = append_lump(&mut bytes, 1, &plane_data);

	let mut node_data = vec![0; nodes.len() * 32];
	for (record, (plane, children)) in nodes.iter().enumerate() {
		let at = record * 32;
		put_i32(&mut node_data, at, *plane);
		put_i32(&mut node_data, at + 4, children[0]);
		put_i32(&mut node_data, at + 8, children[1]);
		put_short_bounds(&mut node_data, at + 12);
	}
	let nodes = append_lump(&mut bytes, 5, &node_data);

	let mut leaf_data = vec![0; leaves * 56];
	for record in 0..leaves {
		let at = record * 56;
		put_i16(&mut leaf_data, at + 4, -1);
		put_i16(&mut leaf_data, at + 6, 1);
		put_short_bounds(&mut leaf_data, at + 8);
	}
	let leaves = append_lump(&mut bytes, 10, &leaf_data);

	let mut model_data = vec![0; 48];
	for axis in 0..3 {
		put_float(&mut model_data, axis * 4, -64.0);
		put_float(&mut model_data, 12 + axis * 4, 64.0);
	}
	put_i32(&mut model_data, 36, root);
	let models = append_lump(&mut bytes, 14, &model_data);
	Fixture {
		bytes,
		planes,
		nodes,
		leaves,
		models,
	}
}

#[test]
fn incomplete_headers_and_wrong_identifiers_fail_before_record_reads() {
	let data = three_regions();
	for length in [0, 3, 7, 1035] {
		assert_eq!(
			Bsp::decode(&data.bytes[..length], Limits::default()).unwrap_err(),
			Error::Invalid(Invalid::Header)
		);
	}
	let mut bytes = data.bytes;
	bytes[0] = b'X';
	assert_eq!(
		Bsp::decode(&bytes, Limits::default()).unwrap_err(),
		Error::Invalid(Invalid::Identifier)
	);
}

#[test]
fn incomplete_records_and_missing_world_are_rejected() {
	let mut data = three_regions();
	put_i32(&mut data.bytes, 12 + 1 * 16, 39);
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
		Error::Invalid(Invalid::RecordSize { lump: 1 })
	);
	let mut data = three_regions();
	put_i32(&mut data.bytes, 12 + 14 * 16, 0);
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
		Error::Invalid(Invalid::MissingWorld)
	);
}

#[test]
fn input_record_depth_and_work_budgets_fail_explicitly() {
	let data = three_regions();
	for (limits, resource, limit) in [
		(
			Limits {
				input_bytes: data.bytes.len() - 1,
				..Limits::default()
			},
			Resource::InputBytes,
			data.bytes.len() - 1,
		),
		(
			Limits {
				records: 3,
				..Limits::default()
			},
			Resource::Records,
			3,
		),
		(
			Limits {
				tree_depth: 1,
				..Limits::default()
			},
			Resource::TreeDepth,
			1,
		),
		(
			Limits {
				work: 0,
				..Limits::default()
			},
			Resource::Work,
			0,
		),
	] {
		assert_eq!(
			Bsp::decode(&data.bytes, limits).unwrap_err(),
			Error::Budget { resource, limit }
		);
	}
}

#[test]
fn invalid_plane_node_leaf_and_model_references_are_rejected() {
	for (offset, value) in [(0, -1), (0, 2), (4, 2), (4, -4), (8, i32::MIN)] {
		let mut data = three_regions();
		put_i32(&mut data.bytes, data.nodes + offset, value);
		assert_eq!(
			Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
			Error::Invalid(Invalid::Reference { lump: 5, record: 0 })
		);
	}
	let mut data = three_regions();
	put_i32(&mut data.bytes, data.models + 36, i32::MIN);
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
		Error::Invalid(Invalid::Reference {
			lump: 14,
			record: 0
		})
	);
}

#[test]
fn invalid_query_points_and_all_constraint_indices_are_rejected() {
	let bsp = decode(&three_regions());
	for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
		assert_eq!(
			bsp.locate([bad, 0.0, 0.0]).unwrap_err(),
			Error::Invalid(Invalid::Point)
		);
		assert_eq!(
			bsp.matches(&[], [0.0, bad, 0.0]).unwrap_err(),
			Error::Invalid(Invalid::Point)
		);
	}
	let constraints = [
		HalfSpace {
			plane: 0,
			side: Side::Back,
		},
		HalfSpace {
			plane: 99,
			side: Side::Front,
		},
	];
	assert_eq!(
		bsp.matches(&constraints, [1.0, 0.0, 0.0]).unwrap_err(),
		Error::Invalid(Invalid::Constraint { plane: 99 })
	);
}

#[test]
fn nonfinite_or_zero_planes_are_rejected() {
	for (offset, value) in [(0, f32::NAN), (12, f32::INFINITY)] {
		let mut data = three_regions();
		put_float(&mut data.bytes, data.planes + offset, value);
		assert_eq!(
			Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
			Error::Invalid(Invalid::Plane { record: 0 })
		);
	}
	let mut data = three_regions();
	put_float(&mut data.bytes, data.planes, 0.0);
	put_float(&mut data.bytes, data.planes + 4, -0.0);
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
		Error::Invalid(Invalid::Plane { record: 0 })
	);
}

#[test]
fn partition_order_and_plane_constraints_are_independent_of_culling_boxes() {
	let bsp = decode(&three_regions());
	let parts = bsp.world_partitions().unwrap();
	assert_eq!(parts.coverage(), Coverage::PartitionConstraints);
	assert_eq!(
		parts
			.partitions()
			.iter()
			.map(|part| part.leaf)
			.collect::<Vec<_>>(),
		[0, 1, 2]
	);
	assert_eq!(
		parts.constraints(0).unwrap(),
		[HalfSpace {
			plane: 0,
			side: Side::Front
		}]
	);
	assert_eq!(
		parts.constraints(1).unwrap(),
		[
			HalfSpace {
				plane: 0,
				side: Side::Back
			},
			HalfSpace {
				plane: 1,
				side: Side::Front
			},
		]
	);
	assert_eq!(
		parts.constraints(2).unwrap(),
		[
			HalfSpace {
				plane: 0,
				side: Side::Back
			},
			HalfSpace {
				plane: 1,
				side: Side::Back
			},
		]
	);
	assert!(parts.constraints(3).is_none());
	// Stored bounds stop at 64; the structural half-space remains unbounded.
	assert!(
		bsp.matches(parts.constraints(0).unwrap(), [10_000.0, 0.0, 0.0])
			.unwrap()
	);
	let again = bsp.world_partitions().unwrap();
	assert_eq!(parts.partitions(), again.partitions());
	for id in 0..parts.partitions().len() {
		assert_eq!(parts.constraints(id), again.constraints(id));
	}
}

#[test]
#[ignore = "set SOURCE_BSP_DM_LOCKDOWN to the externally SHA-256-verified pinned public fixture"]
fn pinned_dm_lockdown_counts_tree_and_points() {
	let path = std::env::var_os("SOURCE_BSP_DM_LOCKDOWN").expect("fixture path is required");
	let bytes =
		std::fs::read(path).expect("read the local pinned fixture; this test never downloads");
	// Verify SHA-256 externally before invoking this ignored test; see the
	// fixture README. These independent golden facts are not a hash substitute.
	assert_eq!(bytes.len(), 11_384_400);
	let bsp = Bsp::decode(&bytes, Limits::default()).unwrap();
	assert_eq!((bsp.version(), bsp.revision()), (19, 10781));
	assert_eq!(
		(
			bsp.planes().len(),
			bsp.nodes().len(),
			bsp.leaves().len(),
			bsp.models().len()
		),
		(6508, 2682, 2725, 42)
	);
	let parts = bsp.world_partitions().unwrap();
	assert_eq!(parts.partitions().len(), 2320);
	assert_eq!(
		parts.partitions().iter().map(|part| part.leaf).min(),
		Some(1)
	);
	assert_eq!(
		parts.partitions().iter().map(|part| part.leaf).max(),
		Some(2320)
	);
	assert_eq!(
		(0..parts.partitions().len())
			.map(|id| parts.constraints(id).unwrap().len())
			.max(),
		Some(25)
	);
	for (point, leaf, cluster) in [
		([-3667.48, 3439.48, 12.0], 1735, 805),
		([-3893.09, 1413.9, 8.0], 2213, 1071),
		([-2932.0, 3218.0, 8.0], 1209, 561),
		([-3194.96, 3768.0, 2.0], 1617, 727),
		([-4116.0, 4600.0, 2.0], 1057, 484),
		([-3079.48, 4656.52, 138.0], 747, 329),
		([-4066.64, 5622.0, 12.0], 617, 285),
		([-2651.0, 4225.0, 141.0], 105, 38),
		([-2739.0, 2563.0, 143.0], 1322, 626),
	] {
		assert_eq!(bsp.locate(point).unwrap(), leaf);
		assert_eq!(
			(
				bsp.leaves()[leaf].contents,
				bsp.leaves()[leaf].area,
				bsp.leaves()[leaf].cluster
			),
			(0, 1, cluster)
		);
	}
}

fn put_float(bytes: &mut [u8], at: usize, value: f32) {
	bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_i16(bytes: &mut [u8], at: usize, value: i16) {
	bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_i32(bytes: &mut [u8], at: usize, value: i32) {
	bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_short_bounds(bytes: &mut [u8], at: usize) {
	for axis in 0..3 {
		put_i16(bytes, at + axis * 2, -64);
		put_i16(bytes, at + 6 + axis * 2, 64);
	}
}

#[test]
fn reversed_bounds_and_nonfinite_model_origin_are_rejected() {
	let mut data = three_regions();
	put_i16(&mut data.bytes, data.nodes + 12, 100);
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
		Error::Invalid(Invalid::Bounds { lump: 5, record: 0 })
	);
	let mut data = three_regions();
	put_i16(&mut data.bytes, data.leaves + 8, 100);
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
		Error::Invalid(Invalid::Bounds {
			lump: 10,
			record: 0
		})
	);
	for offset in [12, 24] {
		let mut data = three_regions();
		put_float(&mut data.bytes, data.models + offset, f32::NAN);
		assert_eq!(
			Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
			Error::Invalid(Invalid::Bounds {
				lump: 14,
				record: 0
			})
		);
	}
}

fn shared_chain(depth: usize) -> Fixture {
	let nodes = (0..depth)
		.map(|node| {
			let next = if node + 1 == depth {
				-1
			} else {
				(node + 1) as i32
			};
			(0, [next, next])
		})
		.collect::<Vec<_>>();
	fixture(&[[1.0, 0.0, 0.0, 0.0]], &nodes, 1, 0)
}

#[test]
fn shared_dag_expansion_and_constraint_storage_have_separate_bounds() {
	let data = shared_chain(24);
	let bsp = Bsp::decode(
		&data.bytes,
		Limits {
			partitions: 16,
			..Limits::default()
		},
	)
	.unwrap();
	assert_eq!(
		bsp.world_partitions().unwrap_err(),
		Error::Budget {
			resource: Resource::Partitions,
			limit: 16
		}
	);
	let bsp = Bsp::decode(
		&three_regions().bytes,
		Limits {
			constraints: 2,
			..Limits::default()
		},
	)
	.unwrap();
	assert_eq!(
		bsp.world_partitions().unwrap_err(),
		Error::Budget {
			resource: Resource::Constraints,
			limit: 2
		}
	);
	let bsp = Bsp::decode(
		&shared_chain(8).bytes,
		Limits {
			work: 400,
			..Limits::default()
		},
	)
	.unwrap();
	assert_eq!(
		bsp.world_partitions().unwrap_err(),
		Error::Budget {
			resource: Resource::Work,
			limit: 400
		}
	);
}

#[test]
fn shared_nodes_unfold_with_distinct_parent_constraints() {
	let mut data = three_regions();
	put_i32(&mut data.bytes, data.nodes + 4, 1);
	let bsp = decode(&data);
	let parts = bsp.world_partitions().unwrap();
	assert_eq!(
		parts
			.partitions()
			.iter()
			.map(|part| part.leaf)
			.collect::<Vec<_>>(),
		[1, 2, 1, 2]
	);
	assert_eq!(parts.constraints(0).unwrap()[0].side, Side::Front);
	assert_eq!(parts.constraints(2).unwrap()[0].side, Side::Back);
	assert_eq!(bsp.locate([1.0, 0.0, 6.0]).unwrap(), 1);
	assert_eq!(bsp.locate([-1.0, 0.0, 6.0]).unwrap(), 1);
}

#[test]
fn sloped_split_and_exact_boundary_have_one_documented_side() {
	let bsp = decode(&three_regions());
	for (point, leaf) in [
		([1.0, -1.0, 99.0], 0),
		([-0.0, -0.0, 0.0], 0),
		([-1.0, 0.0, 5.0], 1),
		([-1.0, 0.0, 4.0], 2),
		([100_000.0, 0.0, 0.0], 0),
	] {
		assert_eq!(bsp.locate(point).unwrap(), leaf);
		let partitions = bsp.world_partitions().unwrap();
		let included = partitions
			.partitions()
			.iter()
			.enumerate()
			.filter_map(|(id, partition)| {
				bsp.matches(partitions.constraints(id).unwrap(), point)
					.unwrap()
					.then_some(partition.leaf)
			})
			.collect::<Vec<_>>();
		assert_eq!(included, [leaf]);
	}
}

#[test]
fn structural_records_preserve_metadata_without_claiming_collision() {
	let mut data = three_regions();
	put_i32(&mut data.bytes, data.leaves + 56, 32);
	put_i16(&mut data.bytes, data.leaves + 56 + 6, 257 | (35 << 9));
	let bsp = decode(&data);
	assert_eq!(bsp.version(), 19);
	assert_eq!(bsp.revision(), 7);
	assert_eq!(bsp.coverage(), Coverage::PartitionConstraints);
	assert_eq!(
		(
			bsp.planes().len(),
			bsp.nodes().len(),
			bsp.leaves().len(),
			bsp.models().len()
		),
		(2, 2, 3, 1)
	);
	assert_eq!(bsp.nodes()[0].children, [Child::Leaf(0), Child::Node(1)]);
	assert_eq!(bsp.leaves()[1].contents, 32);
	assert_eq!(bsp.leaves()[1].cluster, -1);
	assert_eq!((bsp.leaves()[1].area, bsp.leaves()[1].flags), (257, 35));
	assert_eq!(bsp.models()[0].origin, [0.0; 3]);
	fn owned_worker_data<T: Send + Sync>() {}
	owned_worker_data::<Bsp>();
	owned_worker_data::<super::WorldPartitions>();
}

fn three_regions() -> Fixture {
	fixture(
		&[[1.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 5.0]],
		&[(0, [-1, 1]), (1, [-2, -3])],
		3,
		0,
	)
}

#[test]
fn unsupported_file_lump_and_compression_versions_are_distinct() {
	let mut data = three_regions();
	put_i32(&mut data.bytes, 4, 21);
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
		Error::UnsupportedVersion(21)
	);
	put_i32(&mut data.bytes, 4, 19);
	put_i32(&mut data.bytes, 8 + 10 * 16 + 8, 2);
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
		Error::UnsupportedLumpVersion {
			lump: 10,
			version: 2
		}
	);
	put_i32(&mut data.bytes, 8 + 10 * 16 + 8, 0);
	put_i32(&mut data.bytes, 8 + 10 * 16 + 12, 168);
	assert_eq!(
		Bsp::decode(&data.bytes, Limits::default()).unwrap_err(),
		Error::UnsupportedCompression { lump: 10 }
	);
}

#[test]
fn unused_lump_contents_are_not_claimed_to_be_decoded() {
	let mut data = three_regions();
	put_i32(&mut data.bytes, 8 + 40 * 16 + 8, 123);
	put_i32(&mut data.bytes, 8 + 40 * 16 + 12, 99);
	assert_eq!(decode(&data).coverage(), Coverage::PartitionConstraints);
}
