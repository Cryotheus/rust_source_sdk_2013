#![cfg(test)]

use super::*;

#[test]
fn all_original_sides_reject_rank_drop_and_oblique_aabb_false_candidates() {
	// The first opposite constraints force an affine plane; later sides make it
	// empty. Another oblique halfspace has a full-domain broadphase yet no volume.
	for room in [
		vec![
			[1.0, 0.0, 0.0, 0.0],
			[-1.0, 0.0, 0.0, 0.0],
			[0.0, 1.0, 0.0, -3.0],
		],
		vec![[2.0, 2.0, 2.0, -13.0]],
	] {
		let (atlas, region) = scene(2.0, &[], &[room]);
		assert!(
			atlas
				.region_incidence(&region, [0.0; 3], query_limits())
				.unwrap()
				.incidences()
				.is_empty()
		);
	}
}

#[test]
fn boundary_only_face_edge_point_and_inconsistent_regions_have_no_incidence() {
	for room in [
		cube([2.0, -1.0, -1.0], [3.0, 1.0, 1.0]),
		cube([2.0, 2.0, -1.0], [3.0, 3.0, 1.0]),
		cube([2.0; 3], [3.0; 3]),
		vec![[1.0, 0.0, 0.0, -1.0], [-1.0, 0.0, 0.0, -1.0]],
		vec![[1.0, 1.0, 1.0, -6.0]],
	] {
		let (atlas, region) = scene(2.0, &[], &[room]);
		assert!(
			atlas
				.region_incidence(&region, [0.0; 3], query_limits())
				.unwrap()
				.incidences()
				.is_empty()
		);
	}
}

#[test]
fn closed_solid_volume_and_sheet_have_no_positive_region_volume() {
	let sheet = vec![[1.0, 0.0, 0.0, 0.0], [-1.0, 0.0, 0.0, 0.0]];
	let (atlas, region) = scene(
		2.0,
		std::slice::from_ref(&sheet),
		std::slice::from_ref(&sheet),
	);
	assert_eq!(atlas.components().len(), 2);
	assert_eq!(atlas.locate([0.0; 3]).unwrap(), PointRelation::Solid);
	assert!(
		atlas
			.region_incidence(&region, [0.0; 3], query_limits())
			.unwrap()
			.incidences()
			.is_empty()
	);
	let solid = cube([-1.0; 3], [1.0; 3]);
	let (atlas, region) = scene(
		2.0,
		std::slice::from_ref(&solid),
		std::slice::from_ref(&solid),
	);
	assert!(
		atlas
			.region_incidence(&region, [0.0; 3], query_limits())
			.unwrap()
			.incidences()
			.is_empty()
	);
}

fn component(atlas: &BrushSpace, point: [f32; 3]) -> usize {
	let PointRelation::Free { fragments } = atlas.locate(point).unwrap() else {
		panic!("free witness required");
	};
	let first = atlas.fragments()[fragments[0]].component();
	assert!(
		fragments
			.iter()
			.all(|&id| atlas.fragments()[id].component() == first)
	);
	first
}

fn components(result: &RegionIncidence) -> Vec<usize> {
	result
		.incidences()
		.iter()
		.map(RegionComponentIncidence::component)
		.collect()
}

#[test]
fn crossing_a_closed_sheet_witnesses_both_components_without_joining_them() {
	let sheet = vec![[1.0, 0.0, 0.0, 0.0], [-1.0, 0.0, 0.0, 0.0]];
	let (atlas, region) = scene(2.0, &[sheet], &[cube([-1.0; 3], [1.0; 3])]);
	let before = atlas.statistics();
	let result = atlas
		.region_incidence(&region, [0.0; 3], query_limits())
		.unwrap();
	assert_eq!(components(&result), [0, 1]);
	assert_ne!(
		component(&atlas, [-1.0, 0.0, 0.0]),
		component(&atlas, [1.0, 0.0, 0.0])
	);
	assert_eq!(atlas.components().len(), 2);
	assert_eq!(atlas.statistics(), before);
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
fn cumulative_pairs_include_later_candidates_in_an_already_witnessed_component() {
	let (atlas, region) = scene(
		2.0,
		&[],
		&[cube([-1.0; 3], [1.0; 3]), cube([-0.5; 3], [0.5; 3])],
	);
	// One structural free cube, no world solids: each nested brush has exactly
	// one broadphase candidate, and both candidates belong to the same component.
	assert_eq!(atlas.fragments().len(), 1);
	assert_eq!(atlas.components().len(), 1);
	let mut limits = query_limits();
	limits.pairs = 2;
	let result = atlas.region_incidence(&region, [0.0; 3], limits).unwrap();
	assert_eq!(result.statistics().candidate_pairs, 2);
	assert_eq!(result.statistics().incidences, 1);
	assert_eq!(result.incidences()[0].source_region_brush(), 0);
	limits.pairs = 1;
	assert!(matches!(
		atlas.region_incidence(&region, [0.0; 3], limits),
		Err(RegionQueryError::Geometry(Error::Budget {
			resource: "region pairs",
			limit: 1
		}))
	));
}

#[test]
fn every_plane_must_translate_even_after_a_hit_or_an_empty_bound() {
	for first in [cube([-1.0; 3], [1.0; 3]), cube([20.0; 3], [21.0; 3])] {
		let later = vec![
			[1.0, 0.0, 0.0, -4.0],
			[-1.0, 0.0, 0.0, -4.0],
			[f32::from_bits(1), 0.0, 0.0, 1.0],
		];
		let (atlas, region) = scene(2.0, &[], &[first, later]);
		assert!(matches!(
			atlas.region_incidence(&region, [1.0, 0.0, 0.0], query_limits()),
			Err(RegionQueryError::UnsupportedExactTranslation {
				source_region_brush: 1,
				plane: 2
			})
		));
	}
	let (atlas, region) = scene(2.0, &[], &[vec![[f32::from_bits(1), 0.0, 0.0, 0.0]]]);
	assert!(matches!(
		atlas.region_incidence(&region, [f32::from_bits(1), 0.0, 0.0], query_limits()),
		Err(RegionQueryError::UnsupportedExactTranslation {
			source_region_brush: 0,
			plane: 0
		})
	));
}

#[test]
fn exact_positive_volume_survives_tiny_and_oblique_intersections() {
	for room in [
		cube([0.0, -1.0, -1.0], [f32::from_bits(1), 1.0, 1.0]),
		vec![[1.0, 1.0, 1.0, f32::from_bits(0xc0bf_ffff)]],
	] {
		let (atlas, region) = scene(2.0, &[], &[room]);
		let result = atlas
			.region_incidence(&region, [0.0; 3], query_limits())
			.unwrap();
		assert_eq!(components(&result), [0]);
		assert_eq!(result.incidences()[0].source_region_brush(), 0);
	}
}

#[test]
fn invalid_origin_and_all_resource_failures_leave_the_atlas_unchanged() {
	let mut room = cube([-7.0; 3], [7.0; 3]);
	// The clipped vertex (-7, 7, 7) lies on this final side. Its three axial
	// supports and this row give det4=0 with permanent=28, requiring exact
	// fallback rather than a support-ID or permanent-zero shortcut. All three
	// free witnesses (-5,0,0), (0,0,0), (5,0,0) remain strictly inside.
	room.push([1.0, 1.0, 1.0, 7.0]);
	let (atlas, region) = three_rooms(&[room]);
	let before_stats = atlas.statistics();
	let before_planes = atlas.planes().len();
	let before_components: Vec<_> = [-5.0, 0.0, 5.0]
		.into_iter()
		.map(|x| component(&atlas, [x, 0.0, 0.0]))
		.collect();
	let successful = atlas
		.region_incidence(&region, [0.0; 3], query_limits())
		.unwrap();
	assert_eq!(successful.incidences().len(), 3);
	assert!(successful.statistics().geometry.exact_predicates > 0);
	for axis in 0..3 {
		let mut origin = [0.0; 3];
		origin[axis] = f32::INFINITY;
		assert!(matches!(
			atlas.region_incidence(&region, origin, query_limits()),
			Err(RegionQueryError::Geometry(Error::Point))
		));
	}
	let mut cases = Vec::new();
	let mut limits = query_limits();
	limits.geometry.work = 0;
	cases.push((limits, "work"));
	let mut limits = query_limits();
	limits.geometry.constraints = before_planes - 1;
	cases.push((limits, "constraints"));
	let mut limits = query_limits();
	limits.geometry.cells = 0;
	cases.push((limits, "cells"));
	let mut limits = query_limits();
	limits.geometry.vertices = 0;
	cases.push((limits, "vertices"));
	let mut limits = query_limits();
	limits.geometry.predicates = 0;
	cases.push((limits, "predicates"));
	let mut limits = query_limits();
	limits.geometry.exact_predicates = 0;
	cases.push((limits, "exact predicates"));
	let mut limits = query_limits();
	limits.pairs = 0;
	cases.push((limits, "region pairs"));
	let mut limits = query_limits();
	limits.incidences = 1;
	cases.push((limits, "region incidences"));
	// The successful accounting sets a precise one-unit-short budget, after
	// temporary geometry and incidence records have already been produced.
	let mut limits = query_limits();
	limits.geometry.work = successful.statistics().geometry.work - 1;
	cases.push((limits, "work"));
	for (limits, wanted) in cases {
		match atlas.region_incidence(&region, [0.0; 3], limits) {
			Err(RegionQueryError::Geometry(Error::Budget { resource, .. })) => {
				assert_eq!(resource, wanted)
			}

			other => panic!("expected atomic {wanted} refusal, got {other:?}"),
		}
	}
	assert_eq!(atlas.statistics(), before_stats);
	assert_eq!(atlas.planes().len(), before_planes);
	assert_eq!(atlas.components().len(), 3);
	assert_eq!(
		[-5.0, 0.0, 5.0]
			.map(|x| component(&atlas, [x, 0.0, 0.0]))
			.as_slice(),
		before_components
	);
	assert_eq!(
		components(
			&atlas
				.region_incidence(&region, [0.0; 3], query_limits())
				.unwrap()
		),
		components(&successful)
	);
}

#[test]
fn nonconvex_union_uses_actual_brushes_and_owned_sorted_witnesses() {
	let (atlas, region) = three_rooms(&[
		cube([-6.0, -1.0, -1.0], [-4.0, 1.0, 1.0]),
		cube([4.0, -1.0, -1.0], [6.0, 1.0, 1.0]),
		cube([-5.5, -0.5, -0.5], [-4.5, 0.5, 0.5]),
	]);
	assert_eq!(atlas.components().len(), 3);
	let left = component(&atlas, [-5.0, 0.0, 0.0]);
	let right = component(&atlas, [5.0, 0.0, 0.0]);
	let middle = component(&atlas, [0.0; 3]);
	let result = atlas
		.region_incidence(&region, [0.0; 3], query_limits())
		.unwrap();
	let mut expected = vec![left, right];
	expected.sort_unstable();
	assert_eq!(components(&result), expected);
	assert!(!components(&result).contains(&middle));
	for row in result.incidences() {
		assert_eq!(
			atlas.fragments()[row.fragment()].component(),
			row.component()
		);
		assert_eq!(
			row.source_region_brush(),
			if row.component() == left { 2 } else { 3 }
		);
	}
	let stats = result.statistics();
	assert_eq!(stats.region_brushes, 3);
	assert_eq!(stats.translated_planes, 18);
	assert_eq!(stats.incidences, 2);
	assert!(stats.candidate_pairs >= 3);
	assert!(stats.geometry.work >= 18 * TRANSLATION_WORK);
	drop(region);
	drop(atlas);
	assert_eq!(components(&result), expected);
}

#[test]
fn origin_is_applied_once_and_stored_model_origin_is_metadata() {
	let (atlas, region) = three_rooms(&[cube([-1.0; 3], [1.0; 3])]);
	assert_eq!(region.stored_origin(), [123.0, 456.0, 789.0]);
	let center = atlas
		.region_incidence(&region, [0.0; 3], query_limits())
		.unwrap();
	let translated = atlas
		.region_incidence(&region, [5.0, 0.0, 0.0], query_limits())
		.unwrap();
	assert_eq!(components(&center), [component(&atlas, [0.0; 3])]);
	assert_eq!(
		components(&translated),
		[component(&atlas, [5.0, 0.0, 0.0])]
	);
	let negative = atlas
		.region_incidence(&region, [-5.0, 0.0, 0.0], query_limits())
		.unwrap();
	assert_eq!(components(&negative), [component(&atlas, [-5.0, 0.0, 0.0])]);
	let signed_zero = atlas
		.region_incidence(&region, [f32::from_bits(0x8000_0000); 3], query_limits())
		.unwrap();
	assert_eq!(components(&signed_zero), components(&center));
	// Translated x interval [7, 9] overlaps the domain interior only on (7, 8).
	let partial = atlas
		.region_incidence(&region, [8.0, 0.0, 0.0], query_limits())
		.unwrap();
	assert_eq!(components(&partial), [component(&atlas, [7.5, 0.0, 0.0])]);
	// [8, 10] merely touches the artificial boundary and has no positive volume.
	assert!(
		atlas
			.region_incidence(&region, [9.0, 0.0, 0.0], query_limits())
			.unwrap()
			.incidences()
			.is_empty()
	);
}

#[test]
fn plane_prefix_import_preserves_constraint_and_vertex_support_identity() {
	let (atlas, region) = three_rooms(&[cube([-7.0; 3], [7.0; 3])]);
	let mut kernel = Kernel::new(Limits::default());
	kernel
		.budget
		.constraints(atlas.kernel.planes.len())
		.unwrap();
	reserve(&mut kernel.planes, atlas.kernel.planes.len()).unwrap();
	kernel.planes.extend_from_slice(&atlas.kernel.planes);
	for fragment in &atlas.fragments {
		assert!(fragment.cell.flat.is_none());
		let copied = kernel.copy_cell(&fragment.cell).unwrap();
		assert_eq!(copied.constraints, fragment.cell.constraints);
		assert_eq!(copied.flat, fragment.cell.flat);
		for (left, right) in copied.vertices.iter().zip(&fragment.cell.vertices) {
			assert_eq!(left.support, right.support);
			assert_eq!(left.denominator, right.denominator);
			for id in left.support {
				assert_eq!(
					coefficients(kernel.planes[id]).map(f32::to_bits),
					coefficients(atlas.kernel.planes[id]).map(f32::to_bits)
				);
			}
		}
		assert!(kernel.full_dimension(&copied).unwrap());
	}
	assert_eq!(
		atlas
			.region_incidence(&region, [0.0; 3], query_limits())
			.unwrap()
			.incidences()
			.len(),
		3
	);
}

fn query_limits() -> RegionQueryLimits {
	RegionQueryLimits {
		geometry: Limits::default(),
		pairs: 100_000,
		incidences: 100_000,
	}
}

// Literal BSP19 records: world model reaches only leaf 0; authored model 1
// reaches only leaf 1. This exercises public owned decoding/region construction.
fn scene(
	half: f32,
	world: &[Vec<[f32; 4]>],
	regions: &[Vec<[f32; 4]>],
) -> (BrushSpace, ModelRegion) {
	let mut lumps: Vec<Vec<u8>> = (0..64).map(|_| Vec::new()).collect();
	let mut side = 0i32;
	for (id, brush) in world.iter().chain(regions).enumerate() {
		lumps[18].extend(side.to_le_bytes());
		lumps[18].extend((brush.len() as i32).to_le_bytes());
		lumps[18].extend(if id < world.len() { 1u32 } else { 0u32 }.to_le_bytes());
		for plane in brush {
			for value in plane {
				lumps[1].extend(value.to_le_bytes());
			}
			lumps[1].extend(0i32.to_le_bytes());
			lumps[19].extend((side as u16).to_le_bytes());
			lumps[19].extend((-1i16).to_le_bytes());
			lumps[19].extend((-1i16).to_le_bytes());
			lumps[19].extend(0i16.to_le_bytes());
			side += 1;
		}
	}
	for (first, count) in [(0, world.len()), (world.len(), regions.len())] {
		lumps[10].extend(0u32.to_le_bytes());
		lumps[10].extend((-1i16).to_le_bytes());
		lumps[10].extend(0u16.to_le_bytes());
		for value in [-1000i16, -1000, -1000, 1000, 1000, 1000] {
			lumps[10].extend(value.to_le_bytes());
		}
		lumps[10].extend([0; 4]);
		lumps[10].extend((first as u16).to_le_bytes());
		lumps[10].extend((count as u16).to_le_bytes());
		lumps[10].extend((-1i16).to_le_bytes());
		lumps[10].extend([0; 26]);
		for id in first..first + count {
			lumps[17].extend((id as u16).to_le_bytes());
		}
	}
	for (head, origin) in [(-1i32, [0.0f32; 3]), (-2, [123.0, 456.0, 789.0])] {
		for value in [-1000.0f32, -1000.0, -1000.0, 1000.0, 1000.0, 1000.0] {
			lumps[14].extend(value.to_le_bytes());
		}
		for value in origin {
			lumps[14].extend(value.to_le_bytes());
		}
		lumps[14].extend(head.to_le_bytes());
		lumps[14].extend([0; 8]);
	}
	let mut bytes = vec![0; 1036];
	bytes[..4].copy_from_slice(b"VBSP");
	bytes[4..8].copy_from_slice(&19i32.to_le_bytes());
	for (id, lump) in lumps.into_iter().enumerate() {
		let at = 8 + 16 * id;
		let start = bytes.len() as i32;
		bytes[at..at + 4].copy_from_slice(&start.to_le_bytes());
		bytes[at + 4..at + 8].copy_from_slice(&(lump.len() as i32).to_le_bytes());
		bytes.extend(lump);
	}
	let geometry = BrushGeometry::decode(&bytes, crate::bsp::Limits::default()).unwrap();
	let region = geometry.model_region(1).unwrap();
	let domain = AnalysisDomain::new([-half; 3], [half; 3]).unwrap();
	let atlas = BrushSpace::build(&geometry, domain, Limits::default()).unwrap();
	(atlas, region)
}

fn three_rooms(regions: &[Vec<[f32; 4]>]) -> (BrushSpace, ModelRegion) {
	scene(
		8.0,
		&[
			cube([-3.0, -8.0, -8.0], [-2.0, 8.0, 8.0]),
			cube([2.0, -8.0, -8.0], [3.0, 8.0, 8.0]),
		],
		regions,
	)
}

#[test]
fn translation_matches_independent_fraction_oracle_bit_fixtures() {
	// Expected encodings were independently derived using Python Fraction and
	// monotone search across finite binary32 encodings, never rounded host f32.
	type TranslationFixture = (&'static str, [u32; 3], u32, [u32; 3], Option<u32>);

	let fixtures: &[TranslationFixture] = &[
		(
			"zero_origin_preserves_max",
			[0x3f800000, 0x00000001, 0x7f7fffff],
			0x7f7fffff,
			[0x00000000, 0x80000000, 0x00000000],
			Some(0x7f7fffff),
		),
		(
			"all_signed_zero_canonicalizes",
			[0xbf800000, 0x00000001, 0x7f7fffff],
			0x80000000,
			[0x80000000, 0x80000000, 0x80000000],
			Some(0x00000000),
		),
		(
			"axial_integer_translation",
			[0x3f800000, 0x00000000, 0x00000000],
			0x3f800000,
			[0x40a00000, 0x00000000, 0x00000000],
			Some(0x40c00000),
		),
		(
			"nonunit_exact_dot",
			[0x40000000, 0x40400000, 0xc0800000],
			0x40800000,
			[0x40a00000, 0x40000000, 0x3f000000],
			Some(0x41900000),
		),
		(
			"subnormal_input_preserved",
			[0x00000001, 0x00000000, 0x00000000],
			0x00000000,
			[0x3f800000, 0x00000000, 0x00000000],
			Some(0x00000001),
		),
		(
			"negative_min_subnormal",
			[0x80000001, 0x00000000, 0x00000000],
			0x00000000,
			[0x3f800000, 0x00000000, 0x00000000],
			Some(0x80000001),
		),
		(
			"subnormal_exact_double",
			[0x00000001, 0x00000000, 0x00000000],
			0x00000000,
			[0x40000000, 0x00000000, 0x00000000],
			Some(0x00000002),
		),
		(
			"largest_subnormal",
			[0x00000001, 0x00000000, 0x00000000],
			0x00800000,
			[0xbf800000, 0x00000000, 0x00000000],
			Some(0x007fffff),
		),
		(
			"cross_to_min_normal",
			[0x00000001, 0x00000000, 0x00000000],
			0x007fffff,
			[0x3f800000, 0x00000000, 0x00000000],
			Some(0x00800000),
		),
		(
			"below_min_subnormal",
			[0x00000001, 0x00000000, 0x00000000],
			0x00000000,
			[0x3f000000, 0x00000000, 0x00000000],
			None,
		),
		(
			"minimum_product_two_to_minus_298",
			[0x00000001, 0x00000000, 0x00000000],
			0x00000000,
			[0x00000001, 0x00000000, 0x00000000],
			None,
		),
		(
			"two_half_subnormals_combine",
			[0x00000001, 0x00000001, 0x00000000],
			0x00000000,
			[0x3f000000, 0x3f000000, 0x00000000],
			Some(0x00000001),
		),
		(
			"subnormal_spacing_hole",
			[0x00000001, 0x00000000, 0x00000000],
			0x00000000,
			[0x3fc00000, 0x00000000, 0x00000000],
			None,
		),
		(
			"normal_precision_hole",
			[0x3f800000, 0x00000001, 0x00000000],
			0x00000000,
			[0x3f800000, 0x3f800000, 0x00000000],
			None,
		),
		(
			"normal_half_ulp",
			[0x33800000, 0x00000000, 0x00000000],
			0x3f800000,
			[0x3f800000, 0x00000000, 0x00000000],
			None,
		),
		(
			"normal_exact_ulp",
			[0x34000000, 0x00000000, 0x00000000],
			0x3f800000,
			[0x3f800000, 0x00000000, 0x00000000],
			Some(0x3f800001),
		),
		(
			"maximum_finite_exact",
			[0x7f7fffff, 0x00000000, 0x00000000],
			0x00000000,
			[0x3f800000, 0x00000000, 0x00000000],
			Some(0x7f7fffff),
		),
		(
			"just_above_maximum_finite",
			[0x00000001, 0x00000000, 0x00000000],
			0x7f7fffff,
			[0x3f800000, 0x00000000, 0x00000000],
			None,
		),
		(
			"positive_range_overflow",
			[0x7f7fffff, 0x00000000, 0x00000000],
			0x00000000,
			[0x40000000, 0x00000000, 0x00000000],
			None,
		),
		(
			"negative_range_overflow",
			[0xff7fffff, 0x00000000, 0x00000000],
			0x00000000,
			[0x40000000, 0x00000000, 0x00000000],
			None,
		),
		(
			"cancel_overflowing_products_to_one",
			[0x7f7fffff, 0xff7fffff, 0x00000000],
			0x3f800000,
			[0x7f7fffff, 0x7f7fffff, 0x00000000],
			Some(0x3f800000),
		),
		(
			"cancel_overflowing_products_to_max",
			[0x7f7fffff, 0xff7fffff, 0x00000000],
			0x7f7fffff,
			[0x7f7fffff, 0x7f7fffff, 0x00000000],
			Some(0x7f7fffff),
		),
		(
			"cancel_overflowing_products_to_subnormal",
			[0x7f7fffff, 0xff7fffff, 0x00000000],
			0x00000001,
			[0x7f7fffff, 0x7f7fffff, 0x00000000],
			Some(0x00000001),
		),
		(
			"cancel_max_to_positive_zero",
			[0x3f800000, 0x00000000, 0x00000000],
			0x7f7fffff,
			[0xff7fffff, 0x00000000, 0x00000000],
			Some(0x00000000),
		),
		(
			"cancel_larger_than_max_to_max",
			[0x7f7fffff, 0xff7fffff, 0x00000000],
			0x00000000,
			[0x40000000, 0x3f800000, 0x00000000],
			Some(0x7f7fffff),
		),
		(
			"cancel_third_axis_to_negative_subnormal",
			[0x7f7fffff, 0xff7fffff, 0x80000001],
			0x00000000,
			[0x7f7fffff, 0x7f7fffff, 0x3f800000],
			Some(0x80000001),
		),
		(
			"maximum_accumulator_556_bits",
			[0x7f7fffff, 0x7f7fffff, 0x7f7fffff],
			0x7f7fffff,
			[0x7f7fffff, 0x7f7fffff, 0x7f7fffff],
			None,
		),
		(
			"negative_maximum_accumulator_556_bits",
			[0xff7fffff, 0xff7fffff, 0xff7fffff],
			0xff7fffff,
			[0x7f7fffff, 0x7f7fffff, 0x7f7fffff],
			None,
		),
	];
	for &(name, normal, distance, origin, expected) in fixtures {
		let mut budget = Budget::new(Limits::default());
		let plane = Plane {
			normal: normal.map(f32::from_bits),
			distance: f32::from_bits(distance),
		};
		let actual = translated_distance(plane, origin.map(f32::from_bits), &mut budget)
			.unwrap()
			.map(f32::to_bits);
		assert_eq!(actual, expected, "{name}");
		assert_eq!(budget.stats.work, TRANSLATION_WORK, "{name}");
		assert_eq!(budget.stats.exact_predicates, 0, "{name}");
	}
}

#[test]
fn translation_precharge_is_separate_from_determinant_fallbacks() {
	let plane = Plane {
		normal: [f32::MAX, -f32::MAX, 0.0],
		distance: f32::MAX,
	};
	let origin = [f32::MAX, f32::MAX, 0.0];
	let mut limits = Limits {
		work: TRANSLATION_WORK - 1,
		..Limits::default()
	};
	let mut budget = Budget::new(limits);
	assert!(matches!(
		translated_distance(plane, origin, &mut budget),
		Err(Error::Budget {
			resource: "work",
			..
		})
	));
	assert_eq!(budget.stats, Statistics::default());
	limits.work = TRANSLATION_WORK;
	let mut budget = Budget::new(limits);
	assert_eq!(
		translated_distance(plane, origin, &mut budget)
			.unwrap()
			.unwrap()
			.to_bits(),
		f32::MAX.to_bits()
	);
	assert_eq!(budget.stats.work, TRANSLATION_WORK);
	assert_eq!(budget.stats.predicates, 0);
	assert_eq!(budget.stats.exact_predicates, 0);
	assert_eq!(budget.stats.constraints, 0);
}

#[test]
fn unbounded_region_keeps_all_original_constraints() {
	let (atlas, region) = three_rooms(&[vec![[2.0, 0.0, 0.0, 2.0]]]);
	let result = atlas
		.region_incidence(&region, [0.0; 3], query_limits())
		.unwrap();
	let mut expected = vec![
		component(&atlas, [-5.0, 0.0, 0.0]),
		component(&atlas, [0.0; 3]),
	];
	expected.sort_unstable();
	assert_eq!(components(&result), expected);
}
