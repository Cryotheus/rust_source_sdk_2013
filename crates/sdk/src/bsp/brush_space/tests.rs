#![cfg(test)]

use super::*;

#[test]
fn adapter_preflights_geometry_budget_and_reports_separate_source_counts() {
	let geometry = structural_geometry(&[[1.0, 0.0, 0.0, 0.0]], &[(0, [-1, -1])], &[0]);
	let domain = AnalysisDomain::new([-2.0; 3], [2.0; 3]).unwrap();
	assert!(matches!(
		BrushSpace::build(
			&geometry,
			domain,
			Limits {
				work: 0,
				..Limits::default()
			}
		),
		Err(Error::Budget {
			resource: "work",
			limit: 0
		})
	));
	let space = BrushSpace::build(&geometry, domain, Limits::default()).unwrap();
	assert_eq!(space.coverage(), Coverage::BoundedSourceSolids);
	assert_eq!(
		space.source_statistics(),
		SourceStatistics {
			partitions: 2,
			world_brushes: 0
		}
	);
	assert_eq!(space.components().len(), 1);
	assert_eq!(space.fragments().len(), 2);
	let incident = |point| match space.locate(point).unwrap() {
		PointRelation::Free { fragments } => fragments,
		other => panic!("expected a represented free point, got {other:?}"),
	};
	let front = incident([0.5, 0.0, 0.0]);
	let back = incident([-0.5, 0.0, 0.0]);
	assert_eq!(front.len(), 1);
	assert_eq!(back.len(), 1);
	assert_eq!(incident([0.0; 3]), front);
	assert_ne!(front, back);
	assert_eq!(space.fragments()[front[0]].partition(), 0);
	assert_eq!(space.fragments()[back[0]].partition(), 1);
	assert!(matches!(
		space.locate([2.0, 0.0, 0.0]).unwrap(),
		PointRelation::OutsideDomain
	));
	assert!(matches!(
		space.locate([f32::NAN, 0.0, 0.0]),
		Err(Error::Point)
	));
}

#[test]
fn artificial_domain_rejects_nonfinite_or_flat_bounds() {
	assert!(matches!(
		AnalysisDomain::new([0.0; 3], [1.0, 0.0, 1.0]),
		Err(Error::Domain)
	));
	assert!(matches!(
		AnalysisDomain::new([0.0; 3], [1.0, f32::INFINITY, 1.0]),
		Err(Error::Domain)
	));
	assert!(AnalysisDomain::new([-1.0; 3], [1.0; 3]).is_ok());
}

fn assert_detail_wall(
	geometry: &super::super::brushes::BrushGeometry,
	brush: usize,
	leaf: usize,
	domain: AnalysisDomain,
	inside: [f32; 3],
	free: [[f32; 3]; 2],
) {
	assert_eq!(
		geometry.brushes()[brush].contents & 0x0800_0001,
		0x0800_0001
	);
	assert_eq!(geometry.bsp().leaves()[leaf].contents & 1, 0);
	for point in [inside, free[0], free[1]] {
		assert_eq!(geometry.bsp().locate(point).unwrap(), leaf);
	}
	let space = BrushSpace::build(geometry, domain, Limits::default()).unwrap();
	eprintln!(
		"brush-space detail brush={brush} leaf={leaf} source={:?} geometry={:?} fragments={} interfaces={} components={}",
		space.source_statistics(),
		space.statistics(),
		space.fragments().len(),
		space.interfaces().len(),
		space.components().len()
	);
	assert!(
		space
			.solids()
			.iter()
			.any(|solid| solid.source() == SolidSource::WorldBrush(brush))
	);
	assert!(matches!(
		space.locate(inside).unwrap(),
		PointRelation::Solid
	));
	// Independent source analysis proves this original axial detail brush
	// spans the entire domain cross-section between the two free witnesses.
	assert_ne!(component_at(&space, free[0]), component_at(&space, free[1]));
}

fn assert_domain_components(space: &BrushSpace, count: usize) {
	assert_eq!(space.components().len(), count);
	assert!(
		space
			.components()
			.iter()
			.all(|component| component.touches_domain())
	);
}

/// Match hand-derived dyadic coordinates by exact source-plane incidence.
fn assert_exact_vertices(kernel: &mut Kernel, cell: &Cell, expected: &[[f32; 3]]) {
	assert_eq!(cell.vertices.len(), expected.len());
	assert!(kernel.full_dimension(cell).unwrap());
	for &point in expected {
		let support = std::array::from_fn(|axis| {
			let mut normal = [0.0; 3];
			normal[axis] = 1.0;
			plane(kernel, normal, point[axis])
		});
		let expected_vertex = kernel.vertex(support).unwrap();
		assert!(
			cell.vertices
				.iter()
				.any(|&actual| kernel.same_vertex(actual, expected_vertex).unwrap()),
			"missing exact vertex {point:?}",
		);
	}
}

#[test]
fn atlas_budget_exhaustion_after_temporary_geometry_returns_no_partial_result() {
	let domain = AnalysisDomain::new([-2.0; 3], [2.0; 3]).unwrap();
	let brushes = vec![source_box([-0.5; 3], [0.5; 3])];
	let complete = super::atlas::test_space(domain, brushes.clone(), Limits::default()).unwrap();
	assert_eq!(complete.components().len(), 1);
	let limit = complete.statistics().cells - 1;
	let result = super::atlas::test_space(
		domain,
		brushes,
		Limits {
			cells: limit,
			..Limits::default()
		},
	);
	assert!(
		matches!(result, Err(Error::Budget { resource: "cells", limit: actual }) if actual == limit)
	);
	assert_eq!(
		component_at(&complete, [-1.0, 0.0, 0.0]),
		component_at(&complete, [1.0, 0.0, 0.0])
	);
}

#[test]
fn axial_bounds_round_outward_and_nonaxial_constraints_keep_the_domain() {
	let domain = AnalysisDomain::new([-2.0; 3], [2.0; 3]).unwrap();
	let mut budget = Budget::new(Limits::default());
	let fraction = super::bounds::Bounds::from_planes(
		domain,
		[
			source_plane([3.0, 0.0, 0.0], 1.0),
			source_plane([-3.0, 0.0, 0.0], -1.0),
		],
		&mut budget,
	)
	.unwrap()
	.unwrap();
	// The exact value 1/3 lies strictly between these adjacent binary64 values.
	assert!(fraction.min[0] <= f64::from_bits(0x3fd5_5555_5555_5555));
	assert!(fraction.max[0] >= f64::from_bits(0x3fd5_5555_5555_5556));
	let fallback = super::bounds::Bounds::from_planes(
		domain,
		[source_plane([1.0, 1.0, 0.0], 0.0)],
		&mut budget,
	)
	.unwrap()
	.unwrap();
	assert_eq!(fallback.min, [-2.0; 3]);
	assert_eq!(fallback.max, [2.0; 3]);
	assert!(
		super::bounds::Bounds::from_planes(
			domain,
			[
				source_plane([1.0, 0.0, 0.0], 0.0),
				source_plane([-1.0, 0.0, 0.0], -1.0),
			],
			&mut budget
		)
		.unwrap()
		.is_none()
	);
}

#[test]
fn boundary_only_and_outside_sheets_preserve_domain_interior() {
	for position in [2.0, 3.0] {
		let sheet = source_box([position, -2.0, -2.0], [position, 2.0, 2.0]);
		let space = synthetic_space(vec![sheet]);
		assert_domain_components(&space, 1);
		let component = component_at(&space, [0.0; 3]);
		for point in [[-1.5, 0.0, 0.0], [1.5, 0.0, 0.0]] {
			assert_eq!(component_at(&space, point), component);
		}
	}
}

#[test]
fn bounds_index_keeps_closed_contacts_orders_ids_and_fails_atomically() {
	use super::bounds::{Bounds, Index};
	let point = Bounds {
		min: [1.0, 0.0, 0.0],
		max: [1.0, 0.0, 0.0],
	};
	let entries = vec![
		(
			9,
			Bounds {
				min: [0.0, -1.0, -1.0],
				max: [1.0, 1.0, 1.0],
			},
		),
		(2, point),
		(9, point),
		(
			5,
			Bounds {
				min: [-2.0, -1.0, -1.0],
				max: [-1.0, 1.0, 1.0],
			},
		),
	];
	let mut budget = Budget::new(Limits::default());
	let index = Index::new(entries.clone(), &mut budget).unwrap();
	assert_eq!(index.query(point, &mut budget).unwrap(), vec![2, 9]);
	let mut no_output = Budget::new(Limits {
		constraints: 0,
		..Limits::default()
	});
	assert!(matches!(
		index.query(point, &mut no_output),
		Err(Error::Budget {
			resource: "constraints",
			limit: 0
		})
	));
	for (resource, limits) in [
		(
			"work",
			Limits {
				work: 0,
				..Limits::default()
			},
		),
		(
			"cells",
			Limits {
				cells: 0,
				..Limits::default()
			},
		),
		(
			"constraints",
			Limits {
				constraints: 0,
				..Limits::default()
			},
		),
	] {
		let mut budget = Budget::new(limits);
		assert!(matches!(
			Index::new(entries.clone(), &mut budget),
			Err(Error::Budget { resource: actual, limit: 0 }) if actual == resource
		));
	}
}

fn box_planes(kernel: &mut Kernel, min: [f32; 3], max: [f32; 3]) -> Vec<usize> {
	let mut result = Vec::new();
	for axis in 0..3 {
		let mut normal = [0.0; 3];
		normal[axis] = -1.0;
		result.push(plane(kernel, normal, -min[axis]));
		normal[axis] = 1.0;
		result.push(plane(kernel, normal, max[axis]));
	}
	result
}

/// Five deterministic nested domains, with unchanged production limits. Geometry
/// budget refusal is a reported research result after the qualified local stage;
/// it is not a successful analysis of that domain or any subsequent domain.
fn brush_space_domain_ladder(
	map: &str,
	geometry: &super::super::brushes::BrushGeometry,
	brush: usize,
	leaf: usize,
	detail: AnalysisDomain,
	inside: [f32; 3],
	free: [[f32; 3]; 2],
) {
	assert_eq!(
		geometry.brushes()[brush].contents & 0x0800_0001,
		0x0800_0001
	);
	assert_eq!(geometry.bsp().leaves()[leaf].contents & 1, 0);
	for point in [inside, free[0], free[1]] {
		assert_eq!(geometry.bsp().locate(point).unwrap(), leaf);
	}
	let centered = |half_extent| {
		AnalysisDomain::new(
			inside.map(|value| value - half_extent),
			inside.map(|value| value + half_extent),
		)
		.unwrap()
	};
	let stages = [
		("detail", detail),
		("half_extent_64", centered(64.0)),
		("half_extent_256", centered(256.0)),
		("half_extent_1024", centered(1024.0)),
		(
			"artificial_65536",
			AnalysisDomain::new([-65536.0; 3], [65536.0; 3]).unwrap(),
		),
	];
	println!(
		"brush-space ladder map={map} planned_stages={} limits={:?} source_traversal_limits=input_bsp coverage=BoundedSourceSolids",
		stages.len(),
		Limits::default()
	);
	for (stage, &(name, domain)) in stages.iter().enumerate() {
		println!(
			"brush-space ladder map={map} stage={stage} name={name} outcome=begin domain_min={:?} domain_max={:?}",
			domain.min(),
			domain.max()
		);
		let build_started = std::time::Instant::now();
		let result = BrushSpace::build(geometry, domain, Limits::default());
		let build_us = build_started.elapsed().as_micros();
		let space = match result {
			Ok(space) => space,

			Err(error @ Error::Budget { .. }) => {
				println!(
					"brush-space ladder map={map} stage={stage} name={name} outcome=budget_refusal phase=build error={error:?} build_us={build_us} completed_stages={stage} unrun_stages={}",
					stages.len() - stage - 1
				);
				assert!(
					stage > 0,
					"qualified local baseline unexpectedly refused: {error:?}"
				);
				return;
			}

			Err(error) => panic!(
				"ladder map={map} stage={stage} name={name}: unexpected build failure {error:?}"
			),
		};
		assert_eq!(space.coverage(), Coverage::BoundedSourceSolids);
		assert!(
			space
				.solids()
				.iter()
				.any(|solid| solid.source() == SolidSource::WorldBrush(brush))
		);
		println!(
			"brush-space ladder map={map} stage={stage} name={name} outcome=constructed build_us={build_us} source={:?} geometry={:?} fragments={} interfaces={} components={}",
			space.source_statistics(),
			space.statistics(),
			space.fragments().len(),
			space.interfaces().len(),
			space.components().len()
		);
		let query_started = std::time::Instant::now();
		let result = ladder_witness_components(&space, inside, free);
		let query_us = query_started.elapsed().as_micros();
		let components = match result {
			Ok(components) => components,

			Err(error @ Error::Budget { .. }) => {
				println!(
					"brush-space ladder map={map} stage={stage} name={name} outcome=budget_refusal phase=query error={error:?} query_us={query_us} completed_stages={stage} unrun_stages={}",
					stages.len() - stage - 1
				);
				assert!(
					stage > 0,
					"qualified local witness query unexpectedly refused: {error:?}"
				);
				return;
			}

			Err(error) => panic!(
				"ladder map={map} stage={stage} name={name}: unexpected query failure {error:?}"
			),
		};
		if stage == 0 {
			// Only the local slab spans the entire domain cross-section. In a
			// larger box, a free path may legitimately pass around its ends.
			assert_ne!(components[0], components[1]);
		}
		println!(
			"brush-space ladder map={map} stage={stage} name={name} outcome=success query_us={query_us} wall_witnesses_same_component={}",
			components[0] == components[1]
		);
	}
	println!(
		"brush-space ladder map={map} outcome=all_requested_domains_complete completed_stages={} unrun_stages=0 enclosure_claim=false native_collision_parity_claim=false",
		stages.len()
	);
}

#[test]
fn closed_line_and_point_solids_exclude_membership_without_splitting_space() {
	for extent in [0.0, 2.0] {
		let solid = source_box([0.0, 0.0, -extent], [0.0, 0.0, extent]);
		let space = synthetic_space(vec![solid]);
		assert_domain_components(&space, 1);
		let component = component_at(&space, [-1.0, 0.0, 0.0]);
		for point in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]] {
			assert_eq!(component_at(&space, point), component);
		}
		assert!(matches!(
			space.locate([0.0; 3]).unwrap(),
			PointRelation::Solid
		));
		if extent == 0.0 {
			assert_eq!(component_at(&space, [0.0, 0.0, 1.0]), component);
		} else {
			assert!(matches!(
				space.locate([0.0, 0.0, 1.0]).unwrap(),
				PointRelation::Solid
			));
		}
	}
}

#[test]
fn closed_sheets_preserve_components_under_side_order_and_positive_scaling() {
	for extent in [1.0, 2.0] {
		for reversed in [false, true] {
			let mut sheet = source_box([0.0, -extent, -extent], [0.0, extent, extent]);
			if reversed {
				sheet.reverse();
				for plane in &mut sheet {
					plane.normal = plane.normal.map(|value| value * 2.0);
					plane.distance *= 2.0;
				}
			}
			let space = synthetic_space(vec![sheet]);
			assert_domain_components(&space, if extent == 2.0 { 2 } else { 1 });
			let left = component_at(&space, [-1.0, 0.0, 0.0]);
			let right = component_at(&space, [1.0, 0.0, 0.0]);
			assert_eq!(left == right, extent == 1.0);
			assert!(matches!(
				space.locate([0.0; 3]).unwrap(),
				PointRelation::Solid
			));
			if extent == 1.0 {
				assert_eq!(component_at(&space, [0.0, 1.5, 0.0]), left);
				// The patch perimeter is closed even when aperture closures
				// contain it geometrically.
				assert!(matches!(
					space.locate([0.0, 1.0, 0.0]).unwrap(),
					PointRelation::Solid
				));
			}
			assert!(matches!(
				space.locate([3.0, 0.0, 0.0]).unwrap(),
				PointRelation::OutsideDomain
			));
		}
	}
}

/// A free boundary point may have several incident fragments, but their
/// interiors must all belong to its one open free-space component.
fn component_at(space: &BrushSpace, point: [f32; 3]) -> usize {
	let PointRelation::Free { fragments } = space.locate(point).unwrap() else {
		panic!("expected a free point at {point:?}");
	};
	assert!(!fragments.is_empty());
	let component = space.fragments()[fragments[0]].component();
	assert!(component < space.components().len());
	assert!(
		fragments
			.iter()
			.all(|&id| space.fragments()[id].component() == component)
	);
	component
}

/// These exactly represented sample points are independent of constructed vertices.
fn contains(kernel: &Kernel, cell: &Cell, point: [f32; 3]) -> bool {
	cell.constraints.iter().all(|&index| {
		let p = kernel.planes[index];
		p.normal
			.iter()
			.zip(point)
			.map(|(&a, b)| f64::from(a) * f64::from(b))
			.sum::<f64>()
			<= f64::from(p.distance)
	})
}

#[test]
fn coplanar_closed_half_sheets_block_the_seam_but_keep_a_subnormal_slit() {
	let tiny = f32::from_bits(1);
	for (overlap, open) in [(0.0, false), (0.5, false), (-tiny, true)] {
		let lower = source_box([0.0, -2.0, -2.0], [0.0, overlap, 2.0]);
		let upper = source_box([0.0, -overlap, -2.0], [0.0, 2.0, 2.0]);
		let space = synthetic_space(vec![lower, upper]);
		assert_domain_components(&space, if open { 1 } else { 2 });
		let left = component_at(&space, [-1.0, 0.0, 0.0]);
		let right = component_at(&space, [1.0, 0.0, 0.0]);
		assert_eq!(left == right, open);
		if open {
			assert_eq!(component_at(&space, [0.0; 3]), left);
			assert!(matches!(
				space.locate([0.0, tiny, 0.0]).unwrap(),
				PointRelation::Solid
			));
			assert!(matches!(
				space.locate([0.0, -tiny, 0.0]).unwrap(),
				PointRelation::Solid
			));
		} else {
			assert!(matches!(
				space.locate([0.0; 3]).unwrap(),
				PointRelation::Solid
			));
		}
	}
}

#[test]
fn coplanar_closed_solid_never_emits_a_strict_outside_interface() {
	let mut kernel = Kernel::new(Limits::default());
	let left = cube(&mut kernel, [-1.0; 3], [0.0, 1.0, 1.0]);
	let right = cube(&mut kernel, [0.0, -1.0, -1.0], [1.0; 3]);
	let aperture = kernel
		.interface(&left, &right, left.constraints[1], right.constraints[0])
		.unwrap()
		.unwrap();
	let wall = box_planes(&mut kernel, [-0.25, -1.0, -1.0], [0.25, 1.0, 1.0]);
	assert!(kernel.subtract(aperture, &wall).unwrap().is_empty());
	let aperture = kernel
		.interface(&left, &right, left.constraints[1], right.constraints[0])
		.unwrap()
		.unwrap();
	let exactly_on_interface = plane(&mut kernel, [1.0, 0.0, 0.0], 0.0);
	assert!(
		kernel
			.subtract(aperture, &[exactly_on_interface])
			.unwrap()
			.is_empty()
	);
}

fn cube(kernel: &mut Kernel, min: [f32; 3], max: [f32; 3]) -> Cell {
	kernel
		.domain(AnalysisDomain::new(min, max).unwrap())
		.unwrap()
}

#[test]
fn domain_and_exact_predicates_obey_independent_cumulative_budgets() {
	let limits = Limits {
		work: 0,
		..Limits::default()
	};
	assert!(matches!(
		Kernel::new(limits).domain(AnalysisDomain::new([-1.0; 3], [1.0; 3]).unwrap()),
		Err(Error::Budget {
			resource: "work",
			..
		})
	));
	let limits = Limits {
		cells: 0,
		..Limits::default()
	};
	assert!(matches!(
		Kernel::new(limits).domain(AnalysisDomain::new([-1.0; 3], [1.0; 3]).unwrap()),
		Err(Error::Budget {
			resource: "cells",
			..
		})
	));
	let limits = Limits {
		vertices: 7,
		..Limits::default()
	};
	assert!(matches!(
		Kernel::new(limits).domain(AnalysisDomain::new([-1.0; 3], [1.0; 3]).unwrap()),
		Err(Error::Budget {
			resource: "vertices",
			..
		})
	));
	let limits = Limits {
		constraints: 5,
		..Limits::default()
	};
	assert!(matches!(
		Kernel::new(limits).domain(AnalysisDomain::new([-1.0; 3], [1.0; 3]).unwrap()),
		Err(Error::Budget {
			resource: "constraints",
			..
		})
	));
	let limits = Limits {
		predicates: 0,
		..Limits::default()
	};
	assert!(matches!(
		Kernel::new(limits).domain(AnalysisDomain::new([-1.0; 3], [1.0; 3]).unwrap()),
		Err(Error::Budget {
			resource: "predicates",
			..
		})
	));
	let limits = Limits {
		exact_predicates: 0,
		..Limits::default()
	};
	let mut budget = Budget::new(limits);
	let rows = [
		[8388608.0, 8388607.0, 8388607.0],
		[8388609.0, 8388608.0, 8388608.0],
		[8388608.0, 8388607.0, 8388608.0],
	];
	assert!(matches!(
		det3(rows, &mut budget),
		Err(Error::Budget {
			resource: "exact predicates",
			..
		})
	));
}

#[test]
fn equality_only_nonsolid_ownership_stays_explicit_across_a_geometric_interface() {
	let geometry = structural_geometry(
		&[[1.0, 0.0, 0.0, 0.0], [-1.0, 0.0, 0.0, 0.0]],
		&[(0, [1, -1]), (1, [-2, -1])],
		&[0, 0],
	);
	// Front/front owns exactly x=0, but has no full-dimensional volume.
	assert_eq!(geometry.bsp().locate([0.0; 3]).unwrap(), 1);
	let space = BrushSpace::build(
		&geometry,
		AnalysisDomain::new([-2.0; 3], [2.0; 3]).unwrap(),
		Limits::default(),
	)
	.unwrap();
	assert!(space.solids().is_empty());
	assert_eq!(space.source_statistics().partitions, 3);
	assert_eq!(space.fragments().len(), 2);
	assert!(
		space
			.fragments()
			.iter()
			.all(|fragment| fragment.leaf() == 0)
	);
	assert_domain_components(&space, 1);
	assert_eq!(
		component_at(&space, [-0.5, 0.0, 0.0]),
		component_at(&space, [0.5, 0.0, 0.0])
	);
	assert!(!space.interfaces().is_empty());
	// The geometric aperture is outside the solid union, yet its midpoint's
	// structural owner is the omitted lower-dimensional occurrence.
	assert!(matches!(
		space.locate([0.0; 3]).unwrap(),
		PointRelation::Unrepresented
	));
}

#[test]
fn equality_only_structural_solid_occurrence_remains_a_closed_wall() {
	let geometry = structural_geometry(
		&[[1.0, 0.0, 0.0, 0.0], [-1.0, 0.0, 0.0, 0.0]],
		&[(0, [1, -1]), (1, [-2, -1])],
		&[0, 1],
	);
	assert!(geometry.brushes().is_empty());
	assert_eq!(geometry.bsp().locate([0.0; 3]).unwrap(), 1);
	let space = BrushSpace::build(
		&geometry,
		AnalysisDomain::new([-2.0; 3], [2.0; 3]).unwrap(),
		Limits::default(),
	)
	.unwrap();
	assert_domain_components(&space, 2);
	assert_ne!(
		component_at(&space, [-0.5, 0.0, 0.0]),
		component_at(&space, [0.5, 0.0, 0.0])
	);
	assert!(matches!(
		space.locate([0.0; 3]).unwrap(),
		PointRelation::Solid
	));
	assert_eq!(space.solids().len(), 1);
	assert!(matches!(
		space.solids()[0].source(),
		SolidSource::StructuralPartition { leaf: 1, .. }
	));
}

#[test]
fn exact_bit_widening_preserves_signed_zero_subnormals_and_extremes() {
	for (input, expected) in [
		(0x0000_0000, 0x0000_0000_0000_0000),
		(0x8000_0000, 0x8000_0000_0000_0000),
		(0x0000_0001, 0x36a0_0000_0000_0000),
		(0x8000_0001, 0xb6a0_0000_0000_0000),
		(0x007f_ffff, 0x380f_ffff_c000_0000),
		(0x807f_ffff, 0xb80f_ffff_c000_0000),
		(0x0080_0000, 0x3810_0000_0000_0000),
		(0x8080_0000, 0xb810_0000_0000_0000),
		(0x7f7f_ffff, 0x47ef_ffff_e000_0000),
		(0xff7f_ffff, 0xc7ef_ffff_e000_0000),
		(0x3f80_0000, 0x3ff0_0000_0000_0000),
		(0xbf80_0000, 0xbff0_0000_0000_0000),
	] {
		assert_eq!(
			super::predicates::widen(f32::from_bits(input)).to_bits(),
			expected
		);
	}
	for bit in 0..23 {
		let expected = u64::from(874_u32 + bit) << 52;
		assert_eq!(
			super::predicates::widen(f32::from_bits(1_u32 << bit)).to_bits(),
			expected
		);
		assert_eq!(
			super::predicates::widen(f32::from_bits(0x8000_0000 | (1_u32 << bit))).to_bits(),
			expected | (1_u64 << 63),
		);
	}
}

#[test]
fn exact_fallback_resolves_cancelled_and_extreme_determinants() {
	let mut budget = Budget::new(Limits::default());
	let rows = [
		[8388608.0, 8388607.0, 8388607.0],
		[8388609.0, 8388608.0, 8388608.0],
		[8388608.0, 8388607.0, 8388608.0],
	];
	assert_eq!(det3(rows, &mut budget).unwrap(), Sign::Positive);
	let mut swapped = rows;
	swapped.swap(0, 1);
	assert_eq!(det3(swapped, &mut budget).unwrap(), Sign::Negative);
	let h = f32::MAX;
	let l = f32::from_bits(1);
	assert_eq!(
		det4(
			[[h, l, h, l], [l, h, l, l], [h, h, h, l], [l, l, h, h]],
			&mut budget
		)
		.unwrap(),
		Sign::Positive
	);
	assert_eq!(
		det4(
			[[l, h, h, h], [l, h, h, h], [h, l, h, h], [h, l, h, h]],
			&mut budget
		)
		.unwrap(),
		Sign::Zero
	);
	assert_eq!(budget.stats.exact_predicates, 4);
}

#[test]
fn exact_predicate_work_limits_include_the_complete_fallback() {
	let rows3 = [
		[8388608.0, 8388607.0, 8388607.0],
		[8388609.0, 8388608.0, 8388608.0],
		[8388608.0, 8388607.0, 8388608.0],
	];
	for work in [27, 28] {
		let mut budget = Budget::new(Limits {
			work,
			predicates: 1,
			exact_predicates: 1,
			..Limits::default()
		});
		let result = det3(rows3, &mut budget);
		if work == 28 {
			assert_eq!(result.unwrap(), Sign::Positive);
			assert_eq!(
				(
					budget.stats.work,
					budget.stats.predicates,
					budget.stats.exact_predicates
				),
				(28, 1, 1)
			);
		} else {
			assert!(matches!(
				result,
				Err(Error::Budget {
					resource: "work",
					limit: 27
				})
			));
			assert_eq!(
				(
					budget.stats.work,
					budget.stats.predicates,
					budget.stats.exact_predicates
				),
				(1, 1, 0)
			);
		}
	}

	let high = f32::MAX;
	let low = f32::from_bits(1);
	let rows4 = [
		[high, low, high, low],
		[low, high, low, low],
		[high, high, high, low],
		[low, low, high, high],
	];
	for work in [112, 113] {
		let mut budget = Budget::new(Limits {
			work,
			predicates: 1,
			exact_predicates: 1,
			..Limits::default()
		});
		let result = det4(rows4, &mut budget);
		if work == 113 {
			assert_eq!(result.unwrap(), Sign::Positive);
			assert_eq!(
				(
					budget.stats.work,
					budget.stats.predicates,
					budget.stats.exact_predicates
				),
				(113, 1, 1)
			);
		} else {
			assert!(matches!(
				result,
				Err(Error::Budget {
					resource: "work",
					limit: 112
				})
			));
			assert_eq!(
				(
					budget.stats.work,
					budget.stats.predicates,
					budget.stats.exact_predicates
				),
				(1, 1, 0)
			);
		}
	}
}

#[test]
fn inconsistent_oblique_constraints_reconnect_an_early_sheet_partition() {
	let mut solid = source_box([0.0, -2.0, -2.0], [0.0, 2.0, 2.0]);
	// These incompatible constraints are nonaxial, so the bounds index must
	// retain the candidate. They follow the equality that stops 3D clipping.
	solid.push(source_plane([0.0, 1.0, 1.0], -1.0));
	solid.push(source_plane([0.0, -1.0, -1.0], -1.0));
	for reversed in [false, true] {
		let mut planes = solid.clone();
		if reversed {
			planes.reverse();
		}
		let space = synthetic_space(vec![planes]);
		assert_domain_components(&space, 1);
		let component = component_at(&space, [0.0; 3]);
		for point in [
			[-1.0, 0.0, 0.0],
			[1.0, 0.0, 0.0],
			[0.0, -1.0, -1.0],
			[0.0, 1.0, 1.0],
		] {
			assert_eq!(component_at(&space, point), component);
		}
	}
}

/// Query budgets are separate from construction. A built atlas alone does not
/// certify that this research stage completed its independent witness checks.
fn ladder_witness_components(
	space: &BrushSpace,
	inside: [f32; 3],
	free: [[f32; 3]; 2],
) -> Result<[usize; 2], Error> {
	assert!(matches!(space.locate(inside)?, PointRelation::Solid));
	let mut components = [0; 2];
	for (index, point) in free.into_iter().enumerate() {
		let PointRelation::Free { fragments } = space.locate(point)? else {
			panic!("known free wall witness was not represented at {point:?}");
		};
		assert!(!fragments.is_empty());
		let component = space.fragments()[fragments[0]].component();
		assert!(component < space.components().len());
		assert!(
			fragments
				.iter()
				.all(|&id| space.fragments()[id].component() == component)
		);
		components[index] = component;
	}
	Ok(components)
}

#[test]
fn narrow_positive_doorway_survives_planar_solid_subtraction() {
	let mut kernel = Kernel::new(Limits::default());
	let left = cube(&mut kernel, [-1.0; 3], [0.0, 1.0, 1.0]);
	let right = cube(&mut kernel, [0.0, -1.0, -1.0], [1.0; 3]);
	let aperture = kernel
		.interface(&left, &right, left.constraints[1], right.constraints[0])
		.unwrap()
		.unwrap();
	let width = f32::from_bits(1);
	let low = box_planes(&mut kernel, [-0.25, -1.0, -1.0], [0.25, -width, 1.0]);
	let high = box_planes(&mut kernel, [-0.25, width, -1.0], [0.25, 1.0, 1.0]);
	let first = kernel.subtract(aperture, &low).unwrap();
	let result = union_subtract(&mut kernel, first, &high);
	assert_eq!(result.len(), 1);
	assert!(kernel.full_dimension(&result[0]).unwrap());
	assert!(contains(&kernel, &result[0], [0.0; 3]));
}

#[test]
fn nonaxial_unbounded_sheet_blocks_interfaces_and_keeps_closed_membership() {
	let space = super::atlas::test_space(
		AnalysisDomain::new([-1.0; 3], [1.0; 3]).unwrap(),
		vec![vec![
			source_plane([1.0, 1.0, 0.0], 0.0),
			source_plane([-1.0, -1.0, 0.0], 0.0),
		]],
		Limits::default(),
	)
	.unwrap();
	assert_domain_components(&space, 2);
	assert_ne!(
		component_at(&space, [0.5, 0.5, 0.0]),
		component_at(&space, [-0.5, -0.5, 0.0])
	);
	assert!(matches!(
		space.locate([0.5, -0.5, 0.0]).unwrap(),
		PointRelation::Solid
	));
}

#[test]
fn nonconvex_union_keeps_its_aabb_notch_free() {
	let mut kernel = Kernel::new(Limits::default());
	let domain = cube(&mut kernel, [-4.0; 3], [4.0; 3]);
	let first = box_planes(&mut kernel, [-3.0, -3.0, -3.0], [0.0, 3.0, 3.0]);
	let second = box_planes(&mut kernel, [0.0, -3.0, -3.0], [3.0, 0.0, 3.0]);
	let third = box_planes(&mut kernel, [-1.0, -2.0, -2.0], [1.0, -1.0, 2.0]);
	let initial = kernel.subtract(domain, &first).unwrap();
	let result = union_subtract(&mut kernel, initial, &second);
	let result = union_subtract(&mut kernel, result, &third);
	assert!(
		result
			.iter()
			.any(|cell| contains(&kernel, cell, [2.0, 2.0, 0.0]))
	);
	for point in [[-2.0, 2.0, 0.0], [2.0, -2.0, 0.0], [0.5, -1.5, 0.0]] {
		assert!(result.iter().all(|cell| !contains(&kernel, cell, point)));
	}
}

#[test]
fn nonsimple_pyramid_vertex_and_existing_edges_clip_completely() {
	let mut kernel = Kernel::new(Limits::default());
	let mut cell = cube(&mut kernel, [-2.0, -2.0, 0.0], [2.0, 2.0, 2.0]);
	for normal in [
		[1.0, 0.0, 1.0],
		[-1.0, 0.0, 1.0],
		[0.0, 1.0, 1.0],
		[0.0, -1.0, 1.0],
	] {
		let cut = plane(&mut kernel, normal, 1.0);
		cell = kernel.clip(cell, cut).unwrap().unwrap();
	}
	assert_eq!(cell.vertices.len(), 5);
	let half = plane(&mut kernel, [1.0, 0.0, 0.0], 0.0);
	let copied = kernel.copy_cell(&cell).unwrap();
	let half = kernel.clip(copied, half).unwrap().unwrap();
	assert_eq!(half.vertices.len(), 5);
	let truncation = plane(&mut kernel, [0.0, 0.0, 1.0], 0.5);
	let truncated = kernel.clip(cell, truncation).unwrap().unwrap();
	assert_eq!(truncated.vertices.len(), 8);
	assert!(kernel.full_dimension(&truncated).unwrap());
}

#[test]
fn oblique_octahedron_cut_preserves_nonsimple_vertices_and_hexagonal_facet() {
	let normals = [
		[-1.0, -1.0, -1.0],
		[-1.0, -1.0, 1.0],
		[-1.0, 1.0, -1.0],
		[-1.0, 1.0, 1.0],
		[1.0, -1.0, -1.0],
		[1.0, -1.0, 1.0],
		[1.0, 1.0, -1.0],
		[1.0, 1.0, 1.0],
	];
	let facet_vertices = [
		[0.5, -0.5, 0.0],
		[0.5, 0.0, -0.5],
		[-0.5, 0.5, 0.0],
		[0.0, 0.5, -0.5],
		[-0.5, 0.0, 0.5],
		[0.0, -0.5, 0.5],
	];
	let clipped_vertices = [
		[-1.0, 0.0, 0.0],
		[0.0, -1.0, 0.0],
		[0.0, 0.0, -1.0],
		[0.5, -0.5, 0.0],
		[0.5, 0.0, -0.5],
		[-0.5, 0.5, 0.0],
		[0.0, 0.5, -0.5],
		[-0.5, 0.0, 0.5],
		[0.0, -0.5, 0.5],
	];
	for (reverse, duplicates) in [(false, false), (true, false), (false, true), (true, true)] {
		let mut kernel = Kernel::new(Limits::default());
		let mut cell = cube(&mut kernel, [-2.0; 3], [2.0; 3]);
		for index in 0..normals.len() {
			let normal = normals[if reverse {
				normals.len() - 1 - index
			} else {
				index
			}];
			let support = plane(&mut kernel, normal, 1.0);
			cell = kernel.clip(cell, support).unwrap().unwrap();
			if duplicates {
				let duplicate = plane(&mut kernel, normal.map(|value| value * 2.0), 2.0);
				cell = kernel.clip(cell, duplicate).unwrap().unwrap();
			}
		}
		assert_eq!(cell.vertices.len(), 6);
		let cut = plane(&mut kernel, [1.0; 3], 0.0);
		let clipped = kernel.clip(cell, cut).unwrap().unwrap();
		assert_exact_vertices(&mut kernel, &clipped, &clipped_vertices);
		let facet = kernel.facet(&clipped, cut).unwrap().unwrap();
		assert_exact_vertices(&mut kernel, &facet, &facet_vertices);
	}
}

#[test]
fn original_finite_sheet_constraints_restore_free_interface_regions() {
	let mut kernel = Kernel::new(Limits::default());
	let domain = cube(&mut kernel, [-2.0; 3], [2.0; 3]);
	// The equality comes first. Volume subtraction stops before it reaches the
	// finite y/z extents, so those original constraints must survive for portals.
	let sheet = box_planes(&mut kernel, [0.0, -1.0, -1.0], [0.0, 1.0, 1.0]);
	let cells = kernel.subtract(domain, &sheet).unwrap();
	assert_eq!(cells.len(), 2);
	let left_support = cells[0]
		.constraints
		.iter()
		.copied()
		.find(|&support| kernel.parallel(support, sheet[0], true).unwrap())
		.unwrap();
	let right_support = cells[1]
		.constraints
		.iter()
		.copied()
		.find(|&support| kernel.parallel(support, sheet[0], true).unwrap())
		.unwrap();
	let interface = kernel
		.interface(&cells[0], &cells[1], left_support, right_support)
		.unwrap()
		.unwrap();
	assert!(contains(&kernel, &interface, [0.0; 3]));
	let apertures = kernel.subtract(interface, &sheet).unwrap();
	assert_eq!(apertures.len(), 4);
	assert!(
		apertures
			.iter()
			.all(|cell| kernel.full_dimension(cell).unwrap())
	);
	assert!(
		apertures
			.iter()
			.all(|cell| !contains(&kernel, cell, [0.0; 3]))
	);
	for point in [
		[0.0, -1.5, 0.0],
		[0.0, 1.5, 0.0],
		[0.0, 0.0, -1.5],
		[0.0, 0.0, 1.5],
	] {
		assert!(apertures.iter().any(|cell| contains(&kernel, cell, point)));
	}
}

#[test]
fn overlapping_and_nested_solids_preserve_only_the_union_complement() {
	let mut kernel = Kernel::new(Limits::default());
	let domain = cube(&mut kernel, [-4.0; 3], [4.0; 3]);
	let first = box_planes(&mut kernel, [-1.0, -4.0, -4.0], [1.0, 4.0, 4.0]);
	let overlap = box_planes(&mut kernel, [0.0, -4.0, -4.0], [2.0, 4.0, 4.0]);
	let nested = box_planes(&mut kernel, [-0.5, -1.0, -1.0], [0.5, 1.0, 1.0]);
	let first_result = kernel.subtract(domain, &first).unwrap();
	let result = union_subtract(&mut kernel, first_result, &overlap);
	let result = union_subtract(&mut kernel, result, &nested);
	assert_eq!(result.len(), 2);
	for x in [-0.5, 0.5, 1.5] {
		assert!(
			result
				.iter()
				.all(|cell| !contains(&kernel, cell, [x, 0.0, 0.0]))
		);
	}
	for x in [-3.0, 3.0] {
		assert_eq!(
			result
				.iter()
				.filter(|cell| contains(&kernel, cell, [x, 0.0, 0.0]))
				.count(),
			1
		);
	}
}

#[test]
fn overlapping_nested_and_duplicate_brushes_preserve_the_closed_union() {
	let first = source_box([-0.5, -2.0, -2.0], [0.5, 2.0, 2.0]);
	let overlap = source_box([0.0, -2.0, -2.0], [1.0, 2.0, 2.0]);
	let nested = source_box([-0.25, -1.0, -1.0], [0.25, 1.0, 1.0]);
	let space = synthetic_space(vec![first.clone(), overlap, nested, first]);
	assert_domain_components(&space, 2);
	assert_ne!(
		component_at(&space, [-1.5, 0.0, 0.0]),
		component_at(&space, [1.5, 0.0, 0.0])
	);
	for x in [-0.5, -0.25, 0.25, 0.75, 1.0] {
		assert!(matches!(
			space.locate([x, 0.0, 0.0]).unwrap(),
			PointRelation::Solid
		));
	}
}

#[test]
fn perpendicular_closed_sheets_keep_all_four_components() {
	let x_wall = source_box([0.0, -2.0, -2.0], [0.0, 2.0, 2.0]);
	let y_wall = source_box([-2.0, 0.0, -2.0], [2.0, 0.0, 2.0]);
	let space = synthetic_space(vec![x_wall, y_wall]);
	assert_domain_components(&space, 4);
	let mut components = Vec::new();
	for point in [
		[-1.0, -1.0, 0.0],
		[-1.0, 1.0, 0.0],
		[1.0, -1.0, 0.0],
		[1.0, 1.0, 0.0],
	] {
		let component = component_at(&space, point);
		assert!(!components.contains(&component));
		components.push(component);
	}
	for point in [[0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0; 3]] {
		assert!(matches!(space.locate(point).unwrap(), PointRelation::Solid));
	}
}

/// SHA-256: 8e61224f8bdcc6c461589b85dc4a0823abcc430a66513a1bbcc131777b49ee50.
#[test]
#[ignore = "research ladder; set SOURCE_BSP_ARENA_BYRE to the externally SHA-256-verified pinned fixture"]
fn pinned_arena_byre_brush_space_domain_ladder() {
	let geometry = verified_brush_fixture("SOURCE_BSP_ARENA_BYRE", 16_843_394);
	brush_space_domain_ladder(
		"arena_byre",
		&geometry,
		484,
		699,
		AnalysisDomain::new([-452.0, 632.0, 333.5], [-420.0, 648.0, 344.5]).unwrap(),
		[-436.0, 640.0, 339.0],
		[[-436.0, 634.0, 339.0], [-436.0, 646.0, 339.0]],
	);
}

/// SHA-256: 8e61224f8bdcc6c461589b85dc4a0823abcc430a66513a1bbcc131777b49ee50.
#[test]
#[ignore = "set SOURCE_BSP_ARENA_BYRE to the externally SHA-256-verified pinned public fixture"]
fn pinned_arena_byre_detail_wall_splits_one_nonsolid_structural_leaf() {
	let geometry = verified_brush_fixture("SOURCE_BSP_ARENA_BYRE", 16_843_394);
	assert_detail_wall(
		&geometry,
		484,
		699,
		AnalysisDomain::new([-452.0, 632.0, 333.5], [-420.0, 648.0, 344.5]).unwrap(),
		[-436.0, 640.0, 339.0],
		[[-436.0, 634.0, 339.0], [-436.0, 646.0, 339.0]],
	);
}

/// SHA-256: cbd191411c0be57099da73458167001ec80d58bf37c71cb3c36b2911b6e80fd7.
#[test]
#[ignore = "research ladder; set SOURCE_BSP_CTF_2FORT to the externally SHA-256-verified pinned fixture"]
fn pinned_ctf_2fort_brush_space_domain_ladder() {
	let geometry = verified_brush_fixture("SOURCE_BSP_CTF_2FORT", 22_751_863);
	brush_space_domain_ladder(
		"ctf_2fort",
		&geometry,
		3547,
		681,
		AnalysisDomain::new([713.5, 2285.0, 148.0], [732.5, 2297.0, 180.0]).unwrap(),
		[723.0, 2291.0, 164.0],
		[[723.0, 2287.0, 164.0], [723.0, 2295.0, 164.0]],
	);
}

/// SHA-256: cbd191411c0be57099da73458167001ec80d58bf37c71cb3c36b2911b6e80fd7.
#[test]
#[ignore = "set SOURCE_BSP_CTF_2FORT to the externally SHA-256-verified pinned public fixture"]
fn pinned_ctf_2fort_detail_wall_splits_one_nonsolid_structural_leaf() {
	let geometry = verified_brush_fixture("SOURCE_BSP_CTF_2FORT", 22_751_863);
	assert_detail_wall(
		&geometry,
		3547,
		681,
		AnalysisDomain::new([713.5, 2285.0, 148.0], [732.5, 2297.0, 180.0]).unwrap(),
		[723.0, 2291.0, 164.0],
		[[723.0, 2287.0, 164.0], [723.0, 2295.0, 164.0]],
	);
}

fn plane(kernel: &mut Kernel, normal: [f32; 3], distance: f32) -> usize {
	kernel.plane(Plane { normal, distance }).unwrap()
}

#[test]
fn positive_area_interface_excludes_edge_and_point_contacts() {
	for (offset, expected) in [
		([1.0, 0.5, 0.5], true),
		([1.0, 1.0, 0.0], false),
		([1.0, 1.0, 1.0], false),
	] {
		let mut kernel = Kernel::new(Limits::default());
		let left = cube(&mut kernel, [0.0; 3], [1.0; 3]);
		let right = cube(&mut kernel, offset, offset.map(|value| value + 1.0));
		let a = left.constraints[1];
		let b = right.constraints[0];
		let aperture = kernel.interface(&left, &right, a, b).unwrap();
		assert_eq!(aperture.is_some(), expected);
		if let Some(aperture) = aperture {
			assert!(kernel.full_dimension(&aperture).unwrap());
		}
	}
}

#[test]
fn proportional_oblique_interface_collapses_exactly_and_excludes_a_closed_slab() {
	let mut kernel = Kernel::new(Limits::default());
	let domain = cube(&mut kernel, [-1.0; 3], [1.0; 3]);
	let copied = kernel.copy_cell(&domain).unwrap();
	let left_support = plane(&mut kernel, [2.0; 3], 0.0);
	let right_support = plane(&mut kernel, [-3.0; 3], 0.0);
	let left = kernel.clip(domain, left_support).unwrap().unwrap();
	let right = kernel.clip(copied, right_support).unwrap().unwrap();
	let interface = kernel
		.interface(&left, &right, left_support, right_support)
		.unwrap()
		.unwrap();
	assert_exact_vertices(
		&mut kernel,
		&interface,
		&[
			[1.0, 0.0, -1.0],
			[1.0, -1.0, 0.0],
			[0.0, 1.0, -1.0],
			[-1.0, 1.0, 0.0],
			[0.0, -1.0, 1.0],
			[-1.0, 0.0, 1.0],
		],
	);
	let copied = kernel.copy_cell(&interface).unwrap();
	let nonpositive_x = plane(&mut kernel, [1.0, 0.0, 0.0], 0.0);
	let nonnegative_x = plane(&mut kernel, [-1.0, 0.0, 0.0], 0.0);
	let half = kernel.clip(copied, nonpositive_x).unwrap().unwrap();
	assert!(kernel.full_dimension(&half).unwrap());
	assert!(kernel.clip(half, nonnegative_x).unwrap().is_none());
	let width = f32::from_bits(1);
	let slab = [
		plane(&mut kernel, [1.0; 3], width),
		plane(&mut kernel, [-1.0; 3], width),
	];
	for &support in &slab {
		for &vertex in &interface.vertices {
			assert_eq!(kernel.side(vertex, support).unwrap(), Sign::Negative);
		}
	}
	assert!(kernel.subtract(interface, &slab).unwrap().is_empty());
}

#[test]
fn proportional_redundant_planes_preserve_geometry() {
	let mut kernel = Kernel::new(Limits::default());
	let mut cell = cube(&mut kernel, [-1.0; 3], [1.0; 3]);
	for (normal, distance) in [
		([2.0, 0.0, 0.0], 2.0),
		([1.0, 1.0, 0.0], 2.0),
		([1.0, 1.0, 1.0], 3.0),
	] {
		let redundant = plane(&mut kernel, normal, distance);
		cell = kernel.clip(cell, redundant).unwrap().unwrap();
	}
	assert_eq!(cell.vertices.len(), 8);
	let a = plane(&mut kernel, [1.0, 0.0, 0.0], 1.0);
	let b = plane(&mut kernel, [-2.0, 0.0, 0.0], -2.0);
	assert!(kernel.parallel(a, b, true).unwrap());
	assert!(kernel.full_dimension(&cell).unwrap());
}

#[test]
fn shared_structural_leaf_keeps_opposite_free_quadrants_disconnected() {
	let geometry = structural_geometry(
		&[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
		&[(0, [1, 2]), (1, [-2, -1]), (1, [-1, -2])],
		&[0, 1],
	);
	let space = BrushSpace::build(
		&geometry,
		AnalysisDomain::new([-2.0; 3], [2.0; 3]).unwrap(),
		Limits::default(),
	)
	.unwrap();
	assert_domain_components(&space, 2);
	assert_ne!(
		component_at(&space, [0.5, -0.5, 0.0]),
		component_at(&space, [-0.5, 0.5, 0.0])
	);
	for point in [[0.5, 0.5, 0.0], [-0.5, -0.5, 0.0], [0.0; 3]] {
		assert!(matches!(space.locate(point).unwrap(), PointRelation::Solid));
	}
	let partitions: Vec<_> = space.fragments().iter().map(Fragment::partition).collect();
	assert_eq!(partitions.len(), 2);
	assert_ne!(partitions[0], partitions[1]);
	assert!(
		space
			.fragments()
			.iter()
			.all(|fragment| fragment.leaf() == 0)
	);
}

#[test]
fn shared_structural_node_preserves_each_solid_occurrence_and_its_barrier() {
	let geometry = structural_geometry(
		&[[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0]],
		&[(1, [2, 1]), (0, [-1, 2]), (0, [-2, -1])],
		&[0, 1],
	);
	let space = BrushSpace::build(
		&geometry,
		AnalysisDomain::new([-2.0; 3], [2.0; 3]).unwrap(),
		Limits::default(),
	)
	.unwrap();
	assert_domain_components(&space, 2);
	// Distinct paths exclude an upper quadrant and a lower zero-thickness
	// sheet. Both solid membership and the lower barrier must survive.
	assert_ne!(
		component_at(&space, [-0.5, -0.5, 0.0]),
		component_at(&space, [0.5, -0.5, 0.0])
	);
	assert!(matches!(
		space.locate([0.5, 0.5, 0.0]).unwrap(),
		PointRelation::Solid
	));
	assert!(matches!(
		space.locate([0.0, -0.5, 0.0]).unwrap(),
		PointRelation::Solid
	));
	let occurrences: Vec<_> = space
		.solids()
		.iter()
		.map(|solid| match solid.source() {
			SolidSource::StructuralPartition { partition, leaf: 1 } => partition,
			other => panic!("unexpected solid source {other:?}"),
		})
		.collect();
	assert_eq!(occurrences.len(), 2);
	assert_ne!(occurrences[0], occurrences[1]);
}

#[test]
fn six_finite_closed_sheets_keep_the_enclosed_unseeded_component() {
	let mut brushes = Vec::new();
	for axis in 0..3 {
		for position in [-0.5, 0.5] {
			let mut min = [-0.5; 3];
			let mut max = [0.5; 3];
			min[axis] = position;
			max[axis] = position;
			brushes.push(source_box(min, max));
		}
	}
	let space = synthetic_space(brushes);
	assert_eq!(space.components().len(), 2);
	let enclosed = component_at(&space, [0.0; 3]);
	let outer = component_at(&space, [1.0, 0.0, 0.0]);
	assert_ne!(enclosed, outer);
	assert!(!space.components()[enclosed].touches_domain());
	assert!(space.components()[outer].touches_domain());
	for point in [[0.5, 0.0, 0.0], [0.5; 3]] {
		assert!(matches!(space.locate(point).unwrap(), PointRelation::Solid));
	}
}

fn source_box(min: [f32; 3], max: [f32; 3]) -> Vec<Plane> {
	let mut planes = Vec::new();
	for axis in 0..3 {
		let mut normal = [0.0; 3];
		normal[axis] = -1.0;
		planes.push(source_plane(normal, -min[axis]));
		normal[axis] = 1.0;
		planes.push(source_plane(normal, max[axis]));
	}
	planes
}

fn source_plane(normal: [f32; 3], distance: f32) -> Plane {
	Plane { normal, distance }
}

fn structural_geometry(
	planes: &[[f32; 4]],
	nodes: &[(i32, [i32; 2])],
	contents: &[u32],
) -> super::super::brushes::BrushGeometry {
	fn append_lump(bytes: &mut Vec<u8>, id: usize, data: &[u8]) {
		let start = i32::try_from(bytes.len()).unwrap();
		let length = i32::try_from(data.len()).unwrap();
		bytes[8 + id * 16..12 + id * 16].copy_from_slice(&start.to_le_bytes());
		bytes[12 + id * 16..16 + id * 16].copy_from_slice(&length.to_le_bytes());
		bytes.extend_from_slice(data);
	}
	let mut bytes = vec![0_u8; 1036];
	bytes[..4].copy_from_slice(b"VBSP");
	bytes[4..8].copy_from_slice(&19_i32.to_le_bytes());
	let mut plane_data = Vec::new();
	for row in planes {
		for value in row {
			plane_data.extend_from_slice(&value.to_le_bytes());
		}
		plane_data.extend_from_slice(&0_i32.to_le_bytes());
	}
	append_lump(&mut bytes, 1, &plane_data);
	let mut node_data = vec![0_u8; nodes.len() * 32];
	for (index, (plane, children)) in nodes.iter().enumerate() {
		let at = index * 32;
		node_data[at..at + 4].copy_from_slice(&plane.to_le_bytes());
		node_data[at + 4..at + 8].copy_from_slice(&children[0].to_le_bytes());
		node_data[at + 8..at + 12].copy_from_slice(&children[1].to_le_bytes());
		for axis in 0..3 {
			node_data[at + 12 + axis * 2..at + 14 + axis * 2]
				.copy_from_slice(&(-4_i16).to_le_bytes());
			node_data[at + 18 + axis * 2..at + 20 + axis * 2].copy_from_slice(&4_i16.to_le_bytes());
		}
	}
	append_lump(&mut bytes, 5, &node_data);
	let mut leaf_data = vec![0_u8; contents.len() * 56];
	for (index, contents) in contents.iter().enumerate() {
		let at = index * 56;
		leaf_data[at..at + 4].copy_from_slice(&contents.to_le_bytes());
		leaf_data[at + 4..at + 6].copy_from_slice(&(-1_i16).to_le_bytes());
		leaf_data[at + 6..at + 8].copy_from_slice(&1_i16.to_le_bytes());
		for axis in 0..3 {
			leaf_data[at + 8 + axis * 2..at + 10 + axis * 2]
				.copy_from_slice(&(-4_i16).to_le_bytes());
			leaf_data[at + 14 + axis * 2..at + 16 + axis * 2].copy_from_slice(&4_i16.to_le_bytes());
		}
	}
	append_lump(&mut bytes, 10, &leaf_data);
	let mut model_data = vec![0_u8; 48];
	for axis in 0..3 {
		model_data[axis * 4..axis * 4 + 4].copy_from_slice(&(-4.0_f32).to_le_bytes());
		model_data[12 + axis * 4..16 + axis * 4].copy_from_slice(&4.0_f32.to_le_bytes());
	}
	append_lump(&mut bytes, 14, &model_data);
	super::super::brushes::BrushGeometry::decode(&bytes, super::super::Limits::default()).unwrap()
}

#[test]
fn subnormal_normals_and_fractional_axial_bounds_preserve_sheet_candidates() {
	let tiny = f32::from_bits(1);
	for (scale, distance, sheet_point) in [(tiny, tiny, Some([1.0, 0.0, 0.0])), (3.0, 1.0, None)] {
		let space = synthetic_space(vec![vec![
			source_plane([scale, 0.0, 0.0], distance),
			source_plane([-scale, 0.0, 0.0], -distance),
		]]);
		assert_domain_components(&space, 2);
		assert_ne!(
			component_at(&space, [0.0; 3]),
			component_at(&space, [1.5, 0.0, 0.0])
		);
		if let Some(point) = sheet_point {
			assert!(matches!(space.locate(point).unwrap(), PointRelation::Solid));
		}
	}
}

#[test]
fn subtraction_budget_failure_after_a_temporary_piece_returns_only_error() {
	let mut kernel = Kernel::new(Limits::default());
	let domain = cube(&mut kernel, [-1.0; 3], [1.0; 3]);
	let solid = box_planes(&mut kernel, [-0.5; 3], [0.5; 3]);
	let before = kernel.budget.stats.cells;
	let limit = before + 3;
	kernel.budget.limits.cells = limit;

	// The first side creates a copy and both clipped halves, then emits one
	// outside piece. The next side's copy must fail before any result escapes.
	let result = kernel.subtract(domain, &solid);
	assert!(
		matches!(result, Err(Error::Budget { resource: "cells", limit: actual }) if actual == limit)
	);
	assert_eq!(kernel.budget.stats.cells, limit);
}

fn synthetic_space(brushes: Vec<Vec<Plane>>) -> BrushSpace {
	super::atlas::test_space(
		AnalysisDomain::new([-2.0; 3], [2.0; 3]).unwrap(),
		brushes,
		Limits::default(),
	)
	.unwrap()
}

#[test]
fn thin_closed_wall_splits_one_cell_without_a_tolerance() {
	let mut kernel = Kernel::new(Limits::default());
	let cell = cube(&mut kernel, [-1.0; 3], [1.0; 3]);
	let width = f32::from_bits(1);
	let solid = box_planes(&mut kernel, [-width, -1.0, -1.0], [width, 1.0, 1.0]);
	let result = kernel.subtract(cell, &solid).unwrap();
	assert_eq!(result.len(), 2);
	assert!(result.iter().all(|cell| !contains(&kernel, cell, [0.0; 3])));
	assert_eq!(
		result
			.iter()
			.filter(|cell| contains(&kernel, cell, [-0.5, 0.0, 0.0]))
			.count(),
		1
	);
	assert_eq!(
		result
			.iter()
			.filter(|cell| contains(&kernel, cell, [0.5, 0.0, 0.0]))
			.count(),
		1
	);
	for cell in &result {
		assert!(kernel.full_dimension(cell).unwrap());
	}
}

#[test]
fn unbounded_axial_and_oblique_halfspaces_use_the_finite_domain() {
	for normal in [[1.0, 0.0, 0.0], [1.0, 1.0, 0.0]] {
		let space = synthetic_space(vec![vec![source_plane(normal, 0.0)]]);
		assert_domain_components(&space, 1);
		component_at(&space, [1.0, 1.0, 0.0]);
		assert!(matches!(
			space.locate([-1.0, -1.0, 0.0]).unwrap(),
			PointRelation::Solid
		));
		assert!(matches!(
			space.locate([0.0; 3]).unwrap(),
			PointRelation::Solid
		));
	}
	let space = synthetic_space(vec![vec![
		source_plane([1.0, 0.0, 0.0], 0.0),
		source_plane([-1.0, 0.0, 0.0], 0.0),
	]]);
	assert_domain_components(&space, 2);
	assert_ne!(
		component_at(&space, [-1.0, 0.0, 0.0]),
		component_at(&space, [1.0, 0.0, 0.0])
	);
	assert!(matches!(
		space.locate([0.0; 3]).unwrap(),
		PointRelation::Solid
	));
}

fn union_subtract(kernel: &mut Kernel, cells: Vec<Cell>, solid: &[usize]) -> Vec<Cell> {
	cells
		.into_iter()
		.flat_map(|cell| kernel.subtract(cell, solid).unwrap())
		.collect()
}

fn verified_brush_fixture(variable: &str, length: usize) -> super::super::brushes::BrushGeometry {
	let path =
		std::env::var_os(variable).expect("externally SHA-256-verified fixture path is required");
	let bytes = std::fs::read(path).expect("read local verified fixture; never download");
	assert_eq!(bytes.len(), length);
	super::super::brushes::BrushGeometry::decode(&bytes, super::super::Limits::default()).unwrap()
}
