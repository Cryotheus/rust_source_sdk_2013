use super::*;

struct Fixture {
	planes: Vec<[f32; 4]>,
	sides: Vec<[i32; 4]>,
	brushes: Vec<[i32; 3]>,
	leaves: Vec<(u32, Vec<u16>)>,
	nodes: Vec<[i32; 3]>,
	models: Vec<(i32, [f32; 3])>,
}

impl Fixture {
	fn new() -> Self {
		Self {
			planes: Vec::new(),
			sides: Vec::new(),
			brushes: Vec::new(),
			leaves: vec![(0, Vec::new())],
			nodes: Vec::new(),
			models: vec![(-1, [0.0; 3])],
		}
	}

	fn brush(&mut self, planes: &[[f32; 4]], contents: u32) -> u16 {
		let first = self.sides.len() as i32;
		for &plane in planes {
			self.sides.push([self.planes.len() as i32, -1, -1, 0]);
			self.planes.push(plane);
		}
		let index = self.brushes.len() as u16;
		self.brushes
			.push([first, planes.len() as i32, contents as i32]);
		index
	}

	fn cube(&mut self, min: [f32; 3], max: [f32; 3]) -> u16 {
		self.brush(
			&[
				[-1.0, 0.0, 0.0, -min[0]],
				[1.0, 0.0, 0.0, max[0]],
				[0.0, -1.0, 0.0, -min[1]],
				[0.0, 1.0, 0.0, max[1]],
				[0.0, 0.0, -1.0, -min[2]],
				[0.0, 0.0, 1.0, max[2]],
			],
			1,
		)
	}

	// Literal public record layout, independent of decoder size constants.
	fn bytes(&self) -> Vec<u8> {
		let mut lumps: Vec<Vec<u8>> = (0..64).map(|_| Vec::new()).collect();
		for plane in &self.planes {
			for &value in plane {
				lumps[1].extend(value.to_le_bytes());
			}
			lumps[1].extend(0_i32.to_le_bytes());
		}
		for node in &self.nodes {
			for &value in node {
				lumps[5].extend(value.to_le_bytes());
			}
			for value in [-1000_i16, -1000, -1000, 1000, 1000, 1000] {
				lumps[5].extend(value.to_le_bytes());
			}
			lumps[5].extend([0; 8]);
		}
		for (contents, references) in &self.leaves {
			let first = (lumps[17].len() / 2) as u16;
			for &brush in references {
				lumps[17].extend(brush.to_le_bytes());
			}
			lumps[10].extend(contents.to_le_bytes());
			lumps[10].extend((-1_i16).to_le_bytes());
			lumps[10].extend(0_u16.to_le_bytes());
			for value in [-1000_i16, -1000, -1000, 1000, 1000, 1000] {
				lumps[10].extend(value.to_le_bytes());
			}
			lumps[10].extend([0; 4]);
			lumps[10].extend(first.to_le_bytes());
			lumps[10].extend((references.len() as u16).to_le_bytes());
			lumps[10].extend((-1_i16).to_le_bytes());
			lumps[10].extend([0; 26]);
		}
		for (head, origin) in &self.models {
			for value in [-1000.0_f32, -1000.0, -1000.0, 1000.0, 1000.0, 1000.0] {
				lumps[14].extend(value.to_le_bytes());
			}
			for &value in origin {
				lumps[14].extend(value.to_le_bytes());
			}
			lumps[14].extend(head.to_le_bytes());
			lumps[14].extend([0; 8]);
		}
		for brush in &self.brushes {
			for &value in brush {
				lumps[18].extend(value.to_le_bytes());
			}
		}
		for side in &self.sides {
			lumps[19].extend((side[0] as u16).to_le_bytes());
			for &value in &side[1..] {
				lumps[19].extend((value as i16).to_le_bytes());
			}
		}
		let mut bytes = vec![0; 1036];
		bytes[..4].copy_from_slice(b"VBSP");
		bytes[4..8].copy_from_slice(&19_i32.to_le_bytes());
		for (index, lump) in lumps.into_iter().enumerate() {
			let at = 8 + 16 * index;
			let start = bytes.len() as i32;
			bytes[at..at + 4].copy_from_slice(&start.to_le_bytes());
			bytes[at + 4..at + 8].copy_from_slice(&(lump.len() as i32).to_le_bytes());
			bytes.extend(lump);
		}
		bytes
	}
}

fn one_cube() -> Fixture {
	let mut fixture = Fixture::new();
	let brush = fixture.cube([-1.0; 3], [1.0; 3]);
	fixture.leaves[0].1.push(brush);
	fixture
}

fn decode(fixture: &Fixture) -> BrushGeometry {
	BrushGeometry::decode(&fixture.bytes(), Limits::default()).unwrap()
}

fn overwrite(bytes: &mut [u8], lump: usize, offset: usize, replacement: &[u8]) {
	let field = 8 + 16 * lump;
	let start = i32::from_le_bytes(bytes[field..field + 4].try_into().unwrap()) as usize;
	bytes[start + offset..start + offset + replacement.len()].copy_from_slice(replacement);
}

#[test]
fn union_membership_rejects_the_gap_inside_model_bounds() {
	let mut fixture = Fixture::new();
	let left = fixture.cube([-4.0, -1.0, -1.0], [-1.0, 1.0, 1.0]);
	let right = fixture.cube([1.0, -1.0, -1.0], [4.0, 1.0, 1.0]);
	fixture.leaves[0].1 = vec![right, left, right];
	let geometry = decode(&fixture);
	assert_eq!(
		geometry.leaf_brushes(0).unwrap(),
		[right as usize, left as usize, right as usize]
	);
	assert_eq!(geometry.model_brushes(0).unwrap(), [0, 1]);
	let region = geometry.model_region(0).unwrap();
	assert!(region.contains([-2.0, 0.0, 0.0]).unwrap());
	assert!(region.contains([2.0, 0.0, 0.0]).unwrap());
	assert!(!region.contains([0.0; 3]).unwrap());
	assert!(!region.contains([2.0, 2.0, 0.0]).unwrap());
	assert!(region.contains([-1.0, 1.0, 1.0]).unwrap());
}

#[test]
fn sloped_nonunit_planes_are_actual_constraints() {
	let mut fixture = Fixture::new();
	let brush = fixture.brush(
		&[
			[-1.0, 0.0, 0.0, 0.0],
			[0.0, -1.0, 0.0, 0.0],
			[1.0, 1.0, 0.0, 2.0],
			[0.0, 0.0, -1.0, 0.0],
			[0.0, 0.0, 1.0, 2.0],
		],
		0x8000_0020,
	);
	fixture.sides[2][1..].copy_from_slice(&[123, 456, 7]);
	fixture.leaves[0].1.push(brush);
	let geometry = decode(&fixture);
	assert_eq!(geometry.sides()[2].texinfo, 123);
	assert_eq!(geometry.sides()[2].displacement, 456);
	assert_eq!(geometry.sides()[2].bevel, 7);
	let region = geometry.model_region(0).unwrap();
	assert_eq!(region.brushes()[0].contents(), 0x8000_0020);
	assert!(region.contains([0.5, 0.5, 1.0]).unwrap());
	assert!(region.contains([1.0, 1.0, 1.0]).unwrap());
	assert!(!region.contains([1.5, 1.5, 1.0]).unwrap());
}

#[test]
fn world_and_submodel_references_are_distinct_and_include_solid_leaves() {
	let mut fixture = one_cube();
	let other = fixture.cube([10.0; 3], [12.0; 3]);
	fixture.leaves.push((1, vec![other]));
	fixture.models.push((-2, [30.0, 20.0, 10.0]));
	let geometry = decode(&fixture);
	assert_eq!(geometry.model_brushes(0).unwrap(), [0]);
	assert_eq!(geometry.model_brushes(1).unwrap(), [1]);
	let region = geometry.model_region(1).unwrap();
	assert_eq!(region.model(), 1);
	assert_eq!(region.stored_origin(), [30.0, 20.0, 10.0]);
	assert!(region.contains([11.0; 3]).unwrap());
	assert!(!region.contains([41.0, 31.0, 21.0]).unwrap());
}

#[test]
fn shared_nodes_leaves_and_references_do_not_duplicate_region_brushes() {
	let mut fixture = one_cube();
	fixture.leaves[0].1 = vec![0, 0];
	fixture.leaves.push((1, vec![0]));
	fixture.nodes = vec![[0, 1, 1], [0, -1, -2]];
	fixture.models[0].0 = 0;
	let geometry = decode(&fixture);
	assert_eq!(geometry.model_brushes(0).unwrap(), [0]);
	assert_eq!(geometry.model_region(0).unwrap().brushes().len(), 1);
}

#[test]
fn copied_regions_survive_source_drop_and_reject_nonfinite_queries() {
	fn shared<T: Send + Sync>() {}
	shared::<BrushGeometry>();
	shared::<ModelRegion>();
	let geometry = decode(&one_cube());
	let region = geometry.model_region(0).unwrap();
	drop(geometry);
	assert!(region.contains([0.0; 3]).unwrap());
	for point in [
		[f32::NAN, 0.0, 0.0],
		[0.0, f32::INFINITY, 0.0],
		[0.0, 0.0, f32::NEG_INFINITY],
	] {
		assert!(matches!(
			region.contains(point),
			Err(BrushError::Bsp(Error::Invalid(Invalid::Point)))
		));
	}
}

#[test]
fn unbounded_and_inconsistent_sets_do_not_become_claimed_rooms() {
	let mut fixture = Fixture::new();
	let brush = fixture.brush(&[[1.0, 0.0, 0.0, 0.0]], 1);
	fixture.leaves[0].1.push(brush);
	let region = decode(&fixture).model_region(0).unwrap();
	assert!(
		region
			.contains([-100_000.0, 100_000.0, -100_000.0])
			.unwrap()
	);
	assert!(!region.contains([1.0, 0.0, 0.0]).unwrap());
	let mut impossible = Fixture::new();
	let brush = impossible.brush(&[[1.0, 0.0, 0.0, 0.0], [-1.0, 0.0, 0.0, -1.0]], 1);
	impossible.leaves[0].1.push(brush);
	let region = decode(&impossible).model_region(0).unwrap();
	for x in [-100.0, 0.0, 0.5, 1.0, 100.0] {
		assert!(!region.contains([x, 0.0, 0.0]).unwrap());
	}
}

#[test]
fn finite_f32_products_are_evaluated_in_f64() {
	let mut fixture = Fixture::new();
	let brush = fixture.brush(&[[f32::MAX, -f32::MAX, 0.0, 0.0]], 1);
	fixture.leaves[0].1.push(brush);
	let region = decode(&fixture).model_region(0).unwrap();
	assert!(region.contains([f32::MAX, f32::MAX, 0.0]).unwrap());
	assert!(!region.contains([f32::MAX, 0.0, 0.0]).unwrap());
}

#[test]
fn empty_arrays_and_zero_sided_brushes_are_not_all_space() {
	let fixture = Fixture::new();
	let bytes = fixture.bytes();
	assert!(Bsp::decode(&bytes, Limits::default()).is_ok());
	let geometry = BrushGeometry::decode(&bytes, Limits::default()).unwrap();
	assert!(geometry.model_brushes(0).unwrap().is_empty());
	assert!(matches!(
		geometry.model_region(0),
		Err(BrushError::NoBrushes { model: 0 })
	));
	let mut zero = fixture;
	zero.brushes.push([0, 0, 1]);
	assert!(matches!(
		BrushGeometry::decode(&zero.bytes(), Limits::default()),
		Err(BrushError::ZeroSides { brush: 0 })
	));
}

#[test]
fn malformed_extra_ranges_and_references_are_rejected() {
	let fixture = one_cube();
	for (lump, offset, replacement, expected) in [
		(18, 0, (-1_i32).to_le_bytes().to_vec(), 18),
		(18, 4, (-1_i32).to_le_bytes().to_vec(), 18),
		(18, 0, i32::MAX.to_le_bytes().to_vec(), 18),
		(18, 4, 7_i32.to_le_bytes().to_vec(), 18),
		(19, 0, u16::MAX.to_le_bytes().to_vec(), 19),
		(17, 0, 1_u16.to_le_bytes().to_vec(), 17),
		(10, 24, 1_u16.to_le_bytes().to_vec(), 10),
		(10, 26, u16::MAX.to_le_bytes().to_vec(), 10),
	] {
		let mut bytes = fixture.bytes();
		overwrite(&mut bytes, lump, offset, &replacement);
		let error = BrushGeometry::decode(&bytes, Limits::default()).unwrap_err();
		assert!(
			matches!(error, BrushError::Bsp(Error::Invalid(Invalid::Reference { lump: found, .. })) if found == expected),
			"{error}"
		);
	}
}

#[test]
fn extra_lump_width_and_version_are_checked_without_changing_partition_decode() {
	let fixture = one_cube();
	let mut bytes = fixture.bytes();
	let at = 8 + 19 * 16;
	bytes[at + 4..at + 8].copy_from_slice(&47_i32.to_le_bytes());
	assert!(Bsp::decode(&bytes, Limits::default()).is_ok());
	assert!(matches!(
		BrushGeometry::decode(&bytes, Limits::default()),
		Err(BrushError::Bsp(Error::Invalid(Invalid::RecordSize {
			lump: 19
		})))
	));
	let mut bytes = fixture.bytes();
	bytes[at + 8..at + 12].copy_from_slice(&1_i32.to_le_bytes());
	assert!(matches!(
		BrushGeometry::decode(&bytes, Limits::default()),
		Err(BrushError::Bsp(Error::UnsupportedLumpVersion {
			lump: 19,
			version: 1
		}))
	));
}

#[test]
fn invalid_query_indices_fail_before_access() {
	let geometry = decode(&one_cube());
	assert!(matches!(
		geometry.model_region(usize::MAX),
		Err(BrushError::ModelIndex { model: usize::MAX })
	));
	assert!(matches!(
		geometry.model_brushes(1),
		Err(BrushError::ModelIndex { model: 1 })
	));
	assert!(geometry.leaf_brushes(usize::MAX).is_err());
}

#[test]
fn aggregate_records_and_region_constraints_have_separate_budgets() {
	let fixture = one_cube();
	let exact = Limits {
		records: 17,
		..Limits::default()
	};
	assert!(BrushGeometry::decode(&fixture.bytes(), exact).is_ok());
	let short = Limits {
		records: 16,
		..exact
	};
	assert!(Bsp::decode(&fixture.bytes(), short).is_ok());
	assert!(matches!(
		BrushGeometry::decode(&fixture.bytes(), short),
		Err(BrushError::Bsp(Error::Budget {
			resource: Resource::Records,
			limit: 16
		}))
	));
	let geometry = BrushGeometry::decode(
		&fixture.bytes(),
		Limits {
			constraints: 5,
			..Limits::default()
		},
	)
	.unwrap();
	assert!(geometry.model_brushes(0).is_ok());
	assert!(matches!(
		geometry.model_region(0),
		Err(BrushError::Bsp(Error::Budget {
			resource: Resource::Constraints,
			limit: 5
		}))
	));
}

#[test]
fn repeated_side_sets_cannot_evade_region_work_budget() {
	let mut fixture = Fixture::new();
	fixture.planes.push([1.0, 0.0, 0.0, 0.0]);
	fixture.sides.push([0, -1, -1, 0]);
	for index in 0..100_u16 {
		fixture.brushes.push([0, 1, 1]);
		fixture.leaves[0].1.push(index);
	}
	let geometry = BrushGeometry::decode(
		&fixture.bytes(),
		Limits {
			work: 350,
			..Limits::default()
		},
	)
	.unwrap();
	assert!(matches!(
		geometry.model_region(0),
		Err(BrushError::Bsp(Error::Budget {
			resource: Resource::Work,
			..
		}))
	));
}

/// The external gate must pin the exact local BSP SHA, not only these counts.
#[test]
#[ignore = "requires the explicit hash-verified public 2Fort BSP fixture"]
fn pinned_2fort_brush_regions_and_notches() {
	let path = std::env::var_os("SOURCE_BSP_2FORT")
		.expect("set SOURCE_BSP_2FORT to the verified local BSP");
	let bytes = std::fs::read(path).unwrap();
	assert_eq!(bytes.len(), 22_751_863);
	let geometry = BrushGeometry::decode(&bytes, Limits::default()).unwrap();
	assert_eq!(geometry.brushes().len(), 5_676);
	assert_eq!(geometry.sides().len(), 39_684);
	assert_eq!(geometry.model_brushes(0).unwrap().len(), 5_222);
	for (model, brushes) in [
		(75, vec![5439]),
		(76, vec![5440, 5441, 5442]),
		(77, vec![5443, 5444, 5445]),
		(87, vec![5504]),
		(97, vec![5544]),
		(110, vec![5599]),
	] {
		assert_eq!(geometry.model_brushes(model).unwrap(), brushes);
		let region = geometry.model_region(model).unwrap();
		assert_eq!(region.stored_origin(), [0.0; 3]);
		assert!(
			region
				.brushes()
				.iter()
				.all(|brush| brush.planes().len() == 6 && brush.contents() == 1)
		);
	}
	let room = geometry.model_region(76).unwrap();
	for point in [
		[-1000.0, 1800.0, 300.0],
		[-600.0, 1500.0, 300.0],
		[-400.0, 2000.0, 300.0],
	] {
		assert!(room.contains(point).unwrap(), "{point:?}");
	}
	for point in [[-1000.0, 1400.0, 300.0], [-400.0, 1800.0, 300.0]] {
		assert!(!room.contains(point).unwrap(), "{point:?}");
	}
}

#[test]
fn caller_constraint_allowance_is_checked_before_region_copies() {
	let geometry = decode(&one_cube());
	assert!(matches!(
		geometry.model_region_bounded(0, 5),
		Err(BrushError::Bsp(Error::Budget {
			resource: Resource::Constraints,
			limit: 5
		}))
	));
	let region = geometry.model_region_bounded(0, 6).unwrap();
	assert_eq!(region.constraint_count(), 6);
	let geometry = BrushGeometry::decode(
		&one_cube().bytes(),
		Limits {
			constraints: 5,
			..Limits::default()
		},
	)
	.unwrap();
	assert!(matches!(
		geometry.model_region_bounded(0, usize::MAX),
		Err(BrushError::Bsp(Error::Budget {
			resource: Resource::Constraints,
			limit: 5
		}))
	));
}
