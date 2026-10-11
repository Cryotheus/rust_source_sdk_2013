#![cfg(test)]

use super::*;

#[test]
fn a_smaller_target_cannot_stitch_a_route_outside_its_own_bounds() {
	let geometry = geometry(&[cube([0.0, -2.0, -8.0], [0.0, 2.0, 8.0])]);
	for (domain, count) in [
		(target(), 1),
		(
			AnalysisDomain::new([-8.0, -1.0, -1.0], [8.0, 1.0, 1.0]).unwrap(),
			2,
		),
	] {
		let pair = build_pair(&geometry, domain);
		let whole = BrushSpace::build(&geometry, domain, Limits::default()).unwrap();
		assert_eq!(pair.left().components().len(), count);
		assert_eq!(pair.right().components().len(), count);
		assert_eq!(whole.components().len(), count);
		assert_eq!(pair.certificates().len(), count);
		let points = [[-0.5, 0.0, 0.0], [0.5, 0.0, 0.0]];
		assert_relations(&pair, &whole, &points);
		for atlas in [pair.left(), pair.right()] {
			assert_eq!(atlas.domain().min()[1], domain.min()[1]);
			assert_eq!(atlas.domain().max()[1], domain.max()[1]);
		}
		assert_certificates(&pair);
	}
}

#[test]
fn all_query_resources_and_late_refusals_leave_both_atlases_unchanged() {
	let geometry = geometry(&[cube([-2.0, 0.0, -8.0], [8.0, 0.0, 8.0])]);
	let pair = build_pair(&geometry, target());
	let before_stats = [pair.left().statistics(), pair.right().statistics()];
	let before_planes: Vec<_> = [pair.left(), pair.right()]
		.into_iter()
		.map(|atlas| {
			atlas
				.planes()
				.iter()
				.map(|&p| coefficients(p).map(f32::to_bits))
				.collect::<Vec<_>>()
		})
		.collect();
	let expected = tuples(pair.certificates());
	assert_eq!(expected.len(), 2);
	assert!(pair.overlap_statistics().geometry.exact_predicates > 0);
	let mut cases = Vec::new();
	let mut limits = overlap_limits();
	limits.geometry.work = 0;
	cases.push((limits, "work"));
	let mut limits = overlap_limits();
	limits.geometry.constraints = pair.left().planes().len() + pair.right().planes().len() - 1;
	cases.push((limits, "constraints"));
	let mut limits = overlap_limits();
	limits.geometry.cells = 0;
	cases.push((limits, "cells"));
	let mut limits = overlap_limits();
	limits.geometry.vertices = 0;
	cases.push((limits, "vertices"));
	let mut limits = overlap_limits();
	limits.geometry.predicates = 0;
	cases.push((limits, "predicates"));
	let mut limits = overlap_limits();
	limits.geometry.exact_predicates = 0;
	cases.push((limits, "exact predicates"));
	let mut limits = overlap_limits();
	limits.pairs = 0;
	cases.push((limits, "regional pairs"));
	// One positive record exists temporarily before the second output refuses.
	let mut limits = overlap_limits();
	limits.certificates = 1;
	cases.push((limits, "regional certificates"));
	// A one-unit-short whole-query budget also refuses after temporary hits.
	let mut limits = overlap_limits();
	limits.geometry.work = pair.overlap_statistics().geometry.work - 1;
	cases.push((limits, "work"));
	for (limits, wanted) in cases {
		match enumerate_overlap(pair.left(), pair.right(), limits) {
			Err(Error::Budget { resource, .. }) => assert_eq!(resource, wanted),
			other => panic!("expected atomic {wanted} refusal, got {other:?}"),
		}
		assert_eq!(
			[pair.left().statistics(), pair.right().statistics()],
			before_stats
		);
		for (id, atlas) in [pair.left(), pair.right()].into_iter().enumerate() {
			assert_eq!(
				atlas
					.planes()
					.iter()
					.map(|&p| coefficients(p).map(f32::to_bits))
					.collect::<Vec<_>>(),
				before_planes[id]
			);
		}
		assert_eq!(
			tuples(
				&enumerate_overlap(pair.left(), pair.right(), overlap_limits())
					.unwrap()
					.certificates
			),
			expected
		);
	}
	assert_eq!(tuples(pair.certificates()), expected);
}

fn assert_certificates(pair: &RegionalPair) {
	let rows = pair.certificates();
	assert!(
		rows.windows(2)
			.all(|w| w[0].components() < w[1].components())
	);
	assert_eq!(pair.overlap_statistics().certificates, rows.len());
	assert_eq!(
		pair.overlap_statistics().left_planes,
		pair.left().planes().len()
	);
	assert_eq!(
		pair.overlap_statistics().right_planes,
		pair.right().planes().len()
	);
	for row in rows {
		let [a, b] = row.fragments();
		let left = &pair.left().fragments[a];
		let right = &pair.right().fragments[b];
		assert_eq!(row.components(), [left.component, right.component]);
		assert_eq!(left.partition, right.partition);
		assert!(left.cell.flat.is_none() && right.cell.flat.is_none());
		let mut kernel = Kernel::new(Limits::default());
		let offset = import_planes(&mut kernel, pair.left(), pair.right()).unwrap();
		let mut cell = kernel.copy_cell(&left.cell).unwrap();
		for &plane in &right.cell.constraints {
			cell = kernel
				.clip(cell, offset.checked_add(plane).unwrap())
				.unwrap()
				.unwrap();
		}
		assert!(cell.flat.is_none() && kernel.full_dimension(&cell).unwrap());
	}
	let reverse = enumerate_overlap(pair.right(), pair.left(), overlap_limits()).unwrap();
	let mut transposed: Vec<_> = tuples(&reverse.certificates)
		.into_iter()
		.map(|[a, b]| [b, a])
		.collect();
	transposed.sort_unstable();
	assert_eq!(tuples(rows), transposed);
}

// Independently check graph relations against literal whole-target membership;
// no generated fragment counts or coordinates supply the expected topology.
fn assert_relations(pair: &RegionalPair, whole: &BrushSpace, points: &[[f32; 3]]) {
	let left_count = pair.left().components().len();
	let mut parents: Vec<_> = (0..left_count + pair.right().components().len()).collect();
	fn root(parents: &[usize], mut id: usize) -> usize {
		while parents[id] != id {
			id = parents[id];
		}
		id
	}
	for certificate in pair.certificates() {
		let [left, right] = certificate.components();
		let a = root(&parents, left);
		let b = root(&parents, left_count + right);
		parents[b] = a;
	}
	let node = |point: [f32; 3]| {
		if widen(point[pair.split().axis()]) < widen(pair.split().boundaries()[2]) {
			component(pair.left(), point)
		} else {
			left_count + component(pair.right(), point)
		}
	};
	for &a in points {
		for &b in points {
			assert_eq!(
				root(&parents, node(a)) == root(&parents, node(b)),
				component(whole, a) == component(whole, b)
			);
		}
	}
}

#[test]
fn axial_and_oblique_closed_sheets_join_only_the_same_free_side() {
	for sheet in [
		vec![[1.0, 0.0, 0.0, 0.0], [-1.0, 0.0, 0.0, 0.0]],
		vec![[1.0, 1.0, 0.0, 0.0], [-1.0, -1.0, 0.0, 0.0]],
	] {
		let (pair, whole) = pair_and_whole(&[sheet]);
		let points = [[-0.5, -2.0, 0.0], [0.5, 2.0, 0.0]];
		assert_eq!(pair.left().components().len(), 2);
		assert_eq!(pair.right().components().len(), 2);
		assert_eq!(whole.components().len(), 2);
		assert_eq!(tuples(pair.certificates()), expected_pairs(&pair, &points));
		assert_eq!(pair.left().locate([0.0; 3]).unwrap(), PointRelation::Solid);
		assert_eq!(pair.right().locate([0.0; 3]).unwrap(), PointRelation::Solid);
		assert_relations(&pair, &whole, &points);
		assert_certificates(&pair);
	}
}

fn build_pair(geometry: &BrushGeometry, domain: AnalysisDomain) -> RegionalPair {
	RegionalPair::build(
		geometry,
		split(domain),
		Limits::default(),
		Limits::default(),
		overlap_limits(),
	)
	.unwrap()
}

#[test]
fn candidate_cap_counts_occurrence_skips_and_already_certified_tuples() {
	let geometry = scene(
		&[],
		&[[1.0, 0.0, 0.0, 0.0]],
		&[(0, [-1, -1])],
		&[0],
		0,
		crate::bsp::Limits::default(),
	);
	let pair = build_pair(&geometry, target());
	let count = pair.overlap_statistics().candidate_pairs;
	assert_eq!(pair.certificates().len(), 1);
	assert!(count > pair.certificates().len());
	let exact_cap = RegionalOverlapLimits {
		pairs: count,
		certificates: 1,
		..overlap_limits()
	};
	assert_eq!(
		enumerate_overlap(pair.left(), pair.right(), exact_cap)
			.unwrap()
			.certificates
			.len(),
		1
	);
	assert!(matches!(
		enumerate_overlap(
			pair.left(),
			pair.right(),
			RegionalOverlapLimits {
				pairs: count - 1,
				..exact_cap
			}
		),
		Err(Error::Budget {
			resource: "regional pairs",
			..
		})
	));
	let left = BrushSpace::build(
		&geometry,
		AnalysisDomain::new([-2.0; 3], [0.0, 2.0, 2.0]).unwrap(),
		Limits::default(),
	)
	.unwrap();
	let right = BrushSpace::build(
		&geometry,
		AnalysisDomain::new([0.0, -2.0, -2.0], [2.0; 3]).unwrap(),
		Limits::default(),
	)
	.unwrap();
	let only_contact = enumerate_overlap(&left, &right, overlap_limits()).unwrap();
	assert!(only_contact.certificates.is_empty());
	assert!(only_contact.statistics.candidate_pairs > 0);
	assert!(matches!(
		enumerate_overlap(
			&left,
			&right,
			RegionalOverlapLimits {
				pairs: 0,
				..overlap_limits()
			}
		),
		Err(Error::Budget {
			resource: "regional pairs",
			..
		})
	));
	assert_certificates(&pair);
}

fn component(atlas: &BrushSpace, point: [f32; 3]) -> usize {
	let PointRelation::Free { fragments } = atlas.locate(point).unwrap() else {
		panic!("literal free witness required at {point:?}");
	};
	assert!(!fragments.is_empty());
	let component = atlas.fragments()[fragments[0]].component();
	assert!(
		fragments
			.iter()
			.all(|&id| atlas.fragments()[id].component() == component)
	);
	component
}

fn cube(min: [f32; 3], max: [f32; 3]) -> Vec<[f32; 4]> {
	vec![
		[-1.0, 0.0, 0.0, -min[0]],
		[1.0, 0.0, 0.0, max[0]],
		[0.0, -1.0, 0.0, -min[1]],
		[0.0, 1.0, 0.0, max[1]],
		[0.0, 0.0, -1.0, -min[2]],
		[0.0, 0.0, 1.0, max[2]],
	]
}

#[test]
fn either_affine_candidate_refuses_instead_of_certifying_a_planar_disk() {
	for affine_left in [true, false] {
		let [left_domain, right_domain] = split(target()).domains().unwrap();
		let mut left = super::super::test_space(left_domain, vec![], Limits::default()).unwrap();
		let mut right = super::super::test_space(right_domain, vec![], Limits::default()).unwrap();
		if affine_left {
			left.fragments[0].cell.flat = Some(0);
		} else {
			right.fragments[0].cell.flat = Some(0);
		}
		let before = [left.statistics(), right.statistics()];
		assert!(matches!(
			enumerate_overlap(&left, &right, overlap_limits()),
			Err(Error::Invariant)
		));
		assert!(matches!(
			enumerate_overlap(&right, &left, overlap_limits()),
			Err(Error::Invariant)
		));
		assert_eq!([left.statistics(), right.statistics()], before);
	}
}

#[test]
fn empty_source_has_one_join_and_keeps_both_artificial_boundaries() {
	let (pair, whole) = pair_and_whole(&[]);
	assert_eq!(pair.left().components().len(), 1);
	assert_eq!(pair.right().components().len(), 1);
	assert_eq!(whole.components().len(), 1);
	assert_eq!(tuples(pair.certificates()), [[0, 0]]);
	assert!(pair.left().components()[0].touches_domain());
	assert!(pair.right().components()[0].touches_domain());
	assert_eq!(pair.target(), target());
	assert_eq!(
		pair.left().source_statistics(),
		pair.right().source_statistics()
	);
	assert_relations(
		&pair,
		&whole,
		&[[-7.0, 0.0, 0.0], [0.0; 3], [7.0, 0.0, 0.0]],
	);
	assert_certificates(&pair);
}

#[test]
fn equality_only_nonsolid_ownership_stays_unrepresented_after_stitching() {
	let geometry = scene(
		&[],
		&[[1.0, 0.0, 0.0, 0.0], [-1.0, 0.0, 0.0, 0.0]],
		&[(0, [1, -1]), (1, [-2, -1])],
		&[0, 0],
		0,
		crate::bsp::Limits::default(),
	);
	let pair = build_pair(&geometry, target());
	let whole = BrushSpace::build(&geometry, target(), Limits::default()).unwrap();
	for atlas in [pair.left(), pair.right(), &whole] {
		assert_eq!(atlas.components().len(), 1);
		assert_eq!(
			atlas.locate([0.0; 3]).unwrap(),
			PointRelation::Unrepresented
		);
		assert!(atlas.solids().is_empty());
	}
	assert_eq!(pair.certificates().len(), 1);
	assert_relations(&pair, &whole, &[[-0.5, 0.0, 0.0], [0.5, 0.0, 0.0]]);
	assert_certificates(&pair);
}

fn expected_pairs(pair: &RegionalPair, points: &[[f32; 3]]) -> Vec<[usize; 2]> {
	let mut pairs: Vec<_> = points
		.iter()
		.map(|&point| {
			[
				component(pair.left(), point),
				component(pair.right(), point),
			]
		})
		.collect();
	pairs.sort_unstable();
	pairs.dedup();
	pairs
}

#[test]
fn full_plane_prefix_and_checked_right_offset_preserve_all_vertex_supports() {
	let [left_domain, right_domain] = split(target()).domains().unwrap();
	let mut left = super::super::test_space(left_domain, vec![], Limits::default()).unwrap();
	let mut right = super::super::test_space(right_domain, vec![], Limits::default()).unwrap();
	// A support can name a registry entry absent from this cell's inequalities.
	// Replace equivalent x-min supports by a duplicate plane at the full tail.
	let duplicate = left.kernel.plane(left.kernel.planes[0]).unwrap();
	for vertex in &mut left.fragments[0].cell.vertices {
		for id in &mut vertex.support {
			if *id == 0 {
				*id = duplicate;
			}
		}
	}
	assert!(!left.fragments[0].cell.constraints.contains(&duplicate));
	right
		.kernel
		.plane(Plane {
			normal: [1.0, 1.0, 0.0],
			distance: 17.0,
		})
		.unwrap();
	let mut kernel = Kernel::new(Limits::default());
	let offset = import_planes(&mut kernel, &left, &right).unwrap();
	assert_eq!(offset, left.planes().len());
	assert_eq!(
		kernel.planes.len(),
		left.planes().len() + right.planes().len()
	);
	assert_eq!(kernel.budget.stats.constraints, kernel.planes.len());
	for (id, plane) in left.planes().iter().enumerate() {
		assert_eq!(
			coefficients(kernel.planes[id]).map(f32::to_bits),
			coefficients(*plane).map(f32::to_bits)
		);
	}
	for (id, plane) in right.planes().iter().enumerate() {
		assert_eq!(
			coefficients(kernel.planes[offset.checked_add(id).unwrap()]).map(f32::to_bits),
			coefficients(*plane).map(f32::to_bits)
		);
	}
	for fragment in &left.fragments {
		let copied = kernel.copy_cell(&fragment.cell).unwrap();
		assert_eq!(copied.constraints, fragment.cell.constraints);
		assert_eq!(copied.flat, fragment.cell.flat);
		for (a, b) in copied.vertices.iter().zip(&fragment.cell.vertices) {
			assert_eq!(a.support, b.support);
			assert_eq!(a.denominator, b.denominator);
		}
		assert!(kernel.full_dimension(&copied).unwrap());
	}
	let result = enumerate_overlap(&left, &right, overlap_limits()).unwrap();
	assert_eq!(result.certificates.len(), 1);
	assert_eq!(result.statistics.left_planes, left.planes().len());
	assert_eq!(result.statistics.right_planes, right.planes().len());
}

#[test]
fn fully_solid_target_is_a_complete_empty_pair() {
	let (pair, whole) = pair_and_whole(&[cube([-8.0; 3], [8.0; 3])]);
	assert!(pair.left().components().is_empty());
	assert!(pair.right().components().is_empty());
	assert!(whole.components().is_empty());
	assert!(pair.certificates().is_empty());
	assert_eq!(pair.overlap_statistics().candidate_pairs, 0);
	assert_eq!(pair.left().locate([0.0; 3]).unwrap(), PointRelation::Solid);
	assert_eq!(pair.right().locate([0.0; 3]).unwrap(), PointRelation::Solid);
	assert_certificates(&pair);
}

fn geometry(world: &[Vec<[f32; 4]>]) -> BrushGeometry {
	scene(world, &[], &[], &[0], -1, crate::bsp::Limits::default())
}

#[test]
fn one_left_component_joins_every_right_component_around_a_finite_wall() {
	let (pair, whole) = pair_and_whole(&[cube([-2.0, 0.0, -8.0], [8.0, 0.0, 8.0])]);
	let points = [[0.0, -2.0, 0.0], [0.0, 2.0, 0.0]];
	assert_eq!(pair.left().components().len(), 1);
	assert_eq!(pair.right().components().len(), 2);
	assert_eq!(whole.components().len(), 1);
	assert_eq!(tuples(pair.certificates()), expected_pairs(&pair, &points));
	assert_eq!(pair.certificates().len(), 2);
	assert_eq!(
		component(pair.left(), [-4.0, -2.0, 0.0]),
		component(pair.left(), [-4.0, 2.0, 0.0])
	);
	assert_relations(
		&pair,
		&whole,
		&[
			[0.0, -2.0, 0.0],
			[0.0, 2.0, 0.0],
			[6.0, 2.0, 0.0],
			[-4.0, -2.0, 0.0],
		],
	);
	assert_certificates(&pair);
}

#[test]
fn original_closed_line_and_point_solids_remain_excluded_from_both_atlases() {
	for solid in [
		cube([0.0, 0.0, -8.0], [0.0, 0.0, 8.0]),
		cube([0.0; 3], [0.0; 3]),
	] {
		let (pair, whole) = pair_and_whole(&[solid]);
		for atlas in [pair.left(), pair.right(), &whole] {
			assert_eq!(atlas.components().len(), 1);
			assert_eq!(atlas.solids().len(), 1);
			assert_eq!(atlas.solids()[0].constraints().len(), 6);
			assert_eq!(atlas.locate([0.0; 3]).unwrap(), PointRelation::Solid);
		}
		assert_eq!(pair.certificates().len(), 1);
		assert_relations(&pair, &whole, &[[-0.5, -2.0, 0.0], [0.5, 2.0, 0.0]]);
		assert_certificates(&pair);
	}
}

fn overlap_limits() -> RegionalOverlapLimits {
	RegionalOverlapLimits {
		geometry: Limits::default(),
		pairs: 100_000,
		certificates: 100_000,
	}
}

#[test]
fn overlapping_oblique_bounds_cannot_replace_exact_intersection() {
	let domain = AnalysisDomain::new([-0.75; 3], [0.75; 3]).unwrap();
	// Free A: x+y < -0.5; free B: x+y > 0.5. Their conservative boxes overlap.
	let a = super::super::test_space(
		domain,
		vec![vec![Plane {
			normal: [-1.0, -1.0, 0.0],
			distance: 0.5,
		}]],
		Limits::default(),
	)
	.unwrap();
	let b = super::super::test_space(
		domain,
		vec![vec![Plane {
			normal: [1.0, 1.0, 0.0],
			distance: 0.5,
		}]],
		Limits::default(),
	)
	.unwrap();
	assert!(a.fragments[0].bounds.overlaps(b.fragments[0].bounds));
	for (left, right) in [(&a, &b), (&b, &a)] {
		let result = enumerate_overlap(left, right, overlap_limits()).unwrap();
		assert!(result.statistics.candidate_pairs > 0);
		assert!(result.certificates.is_empty());
	}
}

fn pair_and_whole(world: &[Vec<[f32; 4]>]) -> (RegionalPair, BrushSpace) {
	let geometry = geometry(world);
	let pair = build_pair(&geometry, target());
	let whole = BrushSpace::build(&geometry, target(), Limits::default()).unwrap();
	(pair, whole)
}

#[test]
fn privately_built_pair_owns_its_results_after_input_geometry_drop() {
	fn worker_owned<T: Send + Sync>() {}
	worker_owned::<RegionalPair>();
	let pair = {
		let input = geometry(&[cube([-2.0, 0.0, -8.0], [8.0, 0.0, 8.0])]);
		build_pair(&input, target())
	};
	assert_eq!(pair.left().components().len(), 1);
	assert_eq!(pair.right().components().len(), 2);
	assert_eq!(pair.certificates().len(), 2);
	let unrelated_source = geometry(&[cube([-8.0; 3], [8.0; 3])]);
	assert!(
		build_pair(&unrelated_source, target())
			.certificates()
			.is_empty()
	);
	assert_eq!(pair.certificates().len(), 2);
	assert_eq!(
		component(pair.left(), [0.0, -2.0, 0.0]),
		component(pair.left(), [0.0, 2.0, 0.0])
	);
	assert_certificates(&pair);
}

#[test]
fn public_builder_labels_split_source_left_right_and_query_failures_atomically() {
	let geometry = geometry(&[]);
	let bounded = Limits {
		work: 0,
		..Limits::default()
	};
	assert!(matches!(
		RegionalPair::build(
			&geometry,
			split(target()),
			bounded,
			Limits::default(),
			overlap_limits()
		),
		Err(RegionalPairError::LeftBuild(Error::Budget {
			resource: "work",
			..
		}))
	));
	assert!(matches!(
		RegionalPair::build(
			&geometry,
			split(target()),
			Limits::default(),
			bounded,
			overlap_limits()
		),
		Err(RegionalPairError::RightBuild(Error::Budget {
			resource: "work",
			..
		}))
	));
	let limited_query = RegionalOverlapLimits {
		pairs: 0,
		..overlap_limits()
	};
	assert!(matches!(
		RegionalPair::build(
			&geometry,
			split(target()),
			Limits::default(),
			Limits::default(),
			limited_query
		),
		Err(RegionalPairError::Overlap(Error::Budget {
			resource: "regional pairs",
			..
		}))
	));
	let mut forged = split(target());
	forged.target = AnalysisDomain::new([-0.5; 3], [0.5; 3]).unwrap();
	assert!(matches!(
		RegionalPair::build(&geometry, forged, bounded, bounded, limited_query),
		Err(RegionalPairError::Split)
	));
	let shared = scene(
		&[],
		&[[1.0, 0.0, 0.0, 0.0]],
		&[(0, [-1, -1])],
		&[0],
		0,
		crate::bsp::Limits {
			partitions: 1,
			..crate::bsp::Limits::default()
		},
	);
	assert!(matches!(
		RegionalPair::build(
			&shared,
			split(target()),
			Limits::default(),
			Limits::default(),
			overlap_limits()
		),
		Err(RegionalPairError::LeftBuild(Error::Bsp(
			crate::bsp::Error::Budget {
				resource: crate::bsp::Resource::Partitions,
				limit: 1
			}
		)))
	));
	assert_eq!(build_pair(&geometry, target()).certificates().len(), 1);
}

#[test]
fn same_occurrence_face_edge_and_point_contacts_have_no_positive_join() {
	for min in [[0.0, -1.0, -1.0], [0.0, 0.0, -1.0], [0.0; 3]] {
		// These deliberately exercise only the private geometric helper; no
		// public API can attach arbitrary externally constructed atlases.
		let left = super::super::test_space(
			AnalysisDomain::new([-1.0; 3], [0.0; 3]).unwrap(),
			vec![],
			Limits::default(),
		)
		.unwrap();
		let right = super::super::test_space(
			AnalysisDomain::new(min, [1.0; 3]).unwrap(),
			vec![],
			Limits::default(),
		)
		.unwrap();
		assert!(left.fragments[0].bounds.overlaps(right.fragments[0].bounds));
		let result = enumerate_overlap(&left, &right, overlap_limits()).unwrap();
		assert!(result.certificates.is_empty());
		assert!(result.statistics.candidate_pairs > 0);
		assert!(
			enumerate_overlap(&right, &left, overlap_limits())
				.unwrap()
				.certificates
				.is_empty()
		);
	}
}

// Literal BSP19 wire records, independent of production record constants and
// all generated atlas geometry. Every brush is referenced from each leaf.
fn scene(
	world: &[Vec<[f32; 4]>],
	structural_planes: &[[f32; 4]],
	nodes: &[(i32, [i32; 2])],
	contents: &[u32],
	root: i32,
	source_limits: crate::bsp::Limits,
) -> BrushGeometry {
	let mut lumps: Vec<Vec<u8>> = (0..64).map(|_| Vec::new()).collect();
	for row in structural_planes {
		for value in row {
			lumps[1].extend(value.to_le_bytes());
		}
		lumps[1].extend(0i32.to_le_bytes());
	}
	let mut side = 0i32;
	for brush in world {
		lumps[18].extend(side.to_le_bytes());
		lumps[18].extend(i32::try_from(brush.len()).unwrap().to_le_bytes());
		lumps[18].extend(1u32.to_le_bytes());
		for row in brush {
			for value in row {
				lumps[1].extend(value.to_le_bytes());
			}
			lumps[1].extend(0i32.to_le_bytes());
			let plane = structural_planes.len() + usize::try_from(side).unwrap();
			lumps[19].extend(u16::try_from(plane).unwrap().to_le_bytes());
			lumps[19].extend((-1i16).to_le_bytes());
			lumps[19].extend((-1i16).to_le_bytes());
			lumps[19].extend(0i16.to_le_bytes());
			side += 1;
		}
	}
	for (plane, children) in nodes {
		lumps[5].extend(plane.to_le_bytes());
		for child in children {
			lumps[5].extend(child.to_le_bytes());
		}
		for value in [-8i16, -8, -8, 8, 8, 8] {
			lumps[5].extend(value.to_le_bytes());
		}
		lumps[5].extend([0; 8]);
	}
	for (id, bits) in contents.iter().enumerate() {
		lumps[10].extend(bits.to_le_bytes());
		lumps[10].extend((-1i16).to_le_bytes());
		lumps[10].extend(0u16.to_le_bytes());
		for value in [-8i16, -8, -8, 8, 8, 8] {
			lumps[10].extend(value.to_le_bytes());
		}
		lumps[10].extend([0; 4]);
		lumps[10].extend(u16::try_from(id * world.len()).unwrap().to_le_bytes());
		lumps[10].extend(u16::try_from(world.len()).unwrap().to_le_bytes());
		lumps[10].extend((-1i16).to_le_bytes());
		lumps[10].extend([0; 26]);
		for brush in 0..world.len() {
			lumps[17].extend(u16::try_from(brush).unwrap().to_le_bytes());
		}
	}
	for value in [-8.0f32, -8.0, -8.0, 8.0, 8.0, 8.0] {
		lumps[14].extend(value.to_le_bytes());
	}
	lumps[14].extend([0; 12]);
	lumps[14].extend(root.to_le_bytes());
	lumps[14].extend([0; 8]);
	let mut bytes = vec![0; 1036];
	bytes[..4].copy_from_slice(b"VBSP");
	bytes[4..8].copy_from_slice(&19i32.to_le_bytes());
	for (id, lump) in lumps.into_iter().enumerate() {
		let at = 8 + 16 * id;
		let start = i32::try_from(bytes.len()).unwrap();
		bytes[at..at + 4].copy_from_slice(&start.to_le_bytes());
		bytes[at + 4..at + 8].copy_from_slice(&i32::try_from(lump.len()).unwrap().to_le_bytes());
		bytes.extend(lump);
	}
	BrushGeometry::decode(&bytes, source_limits).unwrap()
}

#[test]
fn shared_leaf_ids_never_merge_opposite_structural_free_quadrants() {
	let geometry = scene(
		&[],
		&[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
		&[(0, [1, 2]), (1, [-2, -1]), (1, [-1, -2])],
		&[0, 1],
		0,
		crate::bsp::Limits::default(),
	);
	let pair = build_pair(&geometry, target());
	let whole = BrushSpace::build(&geometry, target(), Limits::default()).unwrap();
	let points = [[0.5, -0.5, 0.0], [-0.5, 0.5, 0.0]];
	for atlas in [pair.left(), pair.right(), &whole] {
		assert_eq!(atlas.components().len(), 2);
		assert!(
			atlas
				.fragments()
				.iter()
				.all(|fragment| fragment.leaf() == 0)
		);
		assert_eq!(atlas.locate([0.0; 3]).unwrap(), PointRelation::Solid);
	}
	assert_eq!(tuples(pair.certificates()), expected_pairs(&pair, &points));
	assert_relations(&pair, &whole, &points);
	assert_certificates(&pair);
}

#[test]
fn shared_node_occurrence_ordinals_survive_different_domain_filtering() {
	let geometry = scene(
		&[],
		&[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
		&[(0, [1, 1]), (1, [-1, -2])],
		&[0, 1],
		0,
		crate::bsp::Limits::default(),
	);
	let pair = RegionalPair::build(
		&geometry,
		RegionalSplit::new(target(), 0, -3.0, -2.0, -1.0).unwrap(),
		Limits::default(),
		Limits::default(),
		overlap_limits(),
	)
	.unwrap();
	assert_eq!(pair.left().source_statistics().partitions, 4);
	assert_eq!(pair.right().source_statistics().partitions, 4);
	assert!(
		pair.left()
			.fragments()
			.iter()
			.all(|fragment| fragment.partition() == 2)
	);
	assert!(
		pair.right()
			.fragments()
			.iter()
			.any(|fragment| fragment.partition() == 0)
	);
	assert!(
		pair.right()
			.fragments()
			.iter()
			.any(|fragment| fragment.partition() == 2)
	);
	assert_eq!(pair.certificates().len(), 1);
	let [a, b] = pair.certificates()[0].fragments();
	assert_eq!(pair.left().fragments()[a].partition(), 2);
	assert_eq!(pair.right().fragments()[b].partition(), 2);
	let whole = BrushSpace::build(&geometry, target(), Limits::default()).unwrap();
	assert_relations(
		&pair,
		&whole,
		&[[-7.0, 2.0, 0.0], [-2.0, 2.0, 0.0], [7.0, 2.0, 0.0]],
	);
	assert_certificates(&pair);
}

fn split(domain: AnalysisDomain) -> RegionalSplit {
	RegionalSplit::new(domain, 0, -1.0, 0.0, 1.0).unwrap()
}

#[test]
fn split_validation_uses_exact_finite_interior_ordering_on_every_axis() {
	for axis in 0..3 {
		let valid = RegionalSplit::new(target(), axis, -1.0, 0.0, 1.0).unwrap();
		let [left, right] = valid.domains().unwrap();
		for other in 0..3 {
			assert_eq!(left.min()[other], target().min()[other]);
			assert_eq!(right.max()[other], target().max()[other]);
			assert_eq!(left.max()[other], if other == axis { 1.0 } else { 8.0 });
			assert_eq!(right.min()[other], if other == axis { -1.0 } else { -8.0 });
		}
	}
	for (axis, a, seam, b) in [
		(3, -1.0, 0.0, 1.0),
		(usize::MAX, -1.0, 0.0, 1.0),
		(0, 0.0, 0.0, 1.0),
		(0, -1.0, 0.0, 0.0),
		(0, 1.0, 0.0, -1.0),
		(0, -8.0, 0.0, 1.0),
		(0, -1.0, 0.0, 8.0),
		(0, -9.0, 0.0, 1.0),
		(0, -1.0, 0.0, 9.0),
		(0, f32::NEG_INFINITY, 0.0, 1.0),
		(0, -1.0, f32::NAN, 1.0),
		(0, -1.0, 0.0, f32::INFINITY),
	] {
		assert!(matches!(
			RegionalSplit::new(target(), axis, a, seam, b),
			Err(RegionalPairError::Split)
		));
	}
	let tiny = AnalysisDomain::new([0.0, -1.0, -1.0], [f32::from_bits(4), 1.0, 1.0]).unwrap();
	let valid = RegionalSplit::new(
		tiny,
		0,
		f32::from_bits(1),
		f32::from_bits(2),
		f32::from_bits(3),
	)
	.unwrap();
	assert_eq!(valid.boundaries().map(f32::to_bits), [1, 2, 3]);
	let adjacent = AnalysisDomain::new([0.0, -1.0, -1.0], [f32::from_bits(2), 1.0, 1.0]).unwrap();
	assert!(matches!(
		RegionalSplit::new(
			adjacent,
			0,
			f32::from_bits(1),
			f32::from_bits(1),
			f32::from_bits(2)
		),
		Err(RegionalPairError::Split)
	));
}

#[test]
fn subnormal_width_open_doorway_connects_and_closed_doorway_separates() {
	for (width, count) in [(f32::from_bits(1), 1), (0.0, 2)] {
		let (pair, whole) = pair_and_whole(&[
			cube([0.0, -8.0, -8.0], [0.0, 0.0, 8.0]),
			cube([0.0, width, -8.0], [0.0, 8.0, 8.0]),
		]);
		// The open doorway contains real y=2^-150 despite no interior f32 sample.
		let points = [[-0.5, -2.0, 0.0], [0.5, 2.0, 0.0]];
		assert_eq!(whole.components().len(), count);
		assert_eq!(pair.left().components().len(), count);
		assert_eq!(pair.right().components().len(), count);
		assert_eq!(pair.certificates().len(), count);
		assert_eq!(tuples(pair.certificates()), expected_pairs(&pair, &points));
		assert_relations(&pair, &whole, &points);
		assert_certificates(&pair);
	}
}

fn target() -> AnalysisDomain {
	AnalysisDomain::new([-8.0; 3], [8.0; 3]).unwrap()
}

fn tuples(rows: &[ComponentPairCertificate]) -> Vec<[usize; 2]> {
	rows.iter()
		.map(ComponentPairCertificate::components)
		.collect()
}

#[test]
fn unseeded_enclosed_volume_and_exterior_keep_separate_joins_and_flags() {
	let mut world = Vec::new();
	for axis in 0..3 {
		for coordinate in [-0.5, 0.5] {
			let mut min = [-0.5; 3];
			let mut max = [0.5; 3];
			min[axis] = coordinate;
			max[axis] = coordinate;
			world.push(cube(min, max));
		}
	}
	let (pair, whole) = pair_and_whole(&world);
	let points = [[0.0; 3], [0.0, 2.0, 0.0]];
	for atlas in [pair.left(), pair.right(), &whole] {
		assert_eq!(atlas.components().len(), 2);
		assert!(!atlas.components()[component(atlas, points[0])].touches_domain());
		assert!(atlas.components()[component(atlas, points[1])].touches_domain());
		assert_eq!(atlas.locate([0.5; 3]).unwrap(), PointRelation::Solid);
	}
	assert_eq!(tuples(pair.certificates()), expected_pairs(&pair, &points));
	assert_relations(&pair, &whole, &points);
	assert_certificates(&pair);
}
