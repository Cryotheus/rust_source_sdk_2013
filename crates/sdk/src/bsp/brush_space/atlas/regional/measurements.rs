//! Fixed public-fixture measurements; records are parsed by the private bounded runner.
#![cfg(test)]

use crate::bsp::brush_space::{
	AnalysisDomain, BrushSpace, Error, Limits, PointRelation, RegionalOverlapLimits, RegionalPair,
	RegionalPairError, RegionalSplit, Statistics,
};
use crate::bsp::{Limits as BspLimits, brushes::BrushGeometry};
use std::io::Read;
use std::time::Instant;

const GEOMETRY_LIMITS: Limits = Limits {
	work: 20_000_000,
	predicates: 5_000_000,
	exact_predicates: 100_000,
	cells: 100_000,
	vertices: 2_000_000,
	constraints: 2_000_000,
};
const OVERLAP_LIMITS: RegionalOverlapLimits = RegionalOverlapLimits {
	geometry: GEOMETRY_LIMITS,
	pairs: 100_000,
	certificates: 100_000,
};
const BSP_LIMITS: BspLimits = BspLimits {
	input_bytes: 67_108_864,
	expanded_bytes: 67_108_864,
	dictionary_bytes: 67_108_864,
	probability_bytes: 1_048_576,
	codec_bytes: 201_326_592,
	records: 262_144,
	tree_depth: 1_024,
	work: 2_000_000,
	partitions: 131_072,
	constraints: 1_000_000,
};

#[derive(Clone, Copy)]
struct Job {
	name: &'static str,
	min: [f32; 3],
	max: [f32; 3],
	split: [f32; 3],
}

fn statistics_fields(statistics: Statistics) -> [usize; 6] {
	[
		statistics.work,
		statistics.predicates,
		statistics.exact_predicates,
		statistics.cells,
		statistics.vertices,
		statistics.constraints,
	]
}

fn print_statistics(statistics: Statistics) {
	for count in statistics_fields(statistics) {
		print!("\t{count}");
	}
}

fn sum_statistics(left: Statistics, right: Statistics, overlap: Statistics) -> Statistics {
	let sum = |a: usize, b: usize, c: usize| {
		a.checked_add(b)
			.and_then(|n| n.checked_add(c))
			.expect("fixed three-ledger measurement sum must fit usize")
	};
	Statistics {
		work: sum(left.work, right.work, overlap.work),
		predicates: sum(left.predicates, right.predicates, overlap.predicates),
		exact_predicates: sum(
			left.exact_predicates,
			right.exact_predicates,
			overlap.exact_predicates,
		),
		cells: sum(left.cells, right.cells, overlap.cells),
		vertices: sum(left.vertices, right.vertices, overlap.vertices),
		constraints: sum(left.constraints, right.constraints, overlap.constraints),
	}
}

fn report_error(job: usize, phase: &str, elapsed_us: u128, error: Error) {
	match error {
		Error::Budget { resource, limit } => {
			println!(
				"regional-pair-v1\trefusal\t{job}\t{phase}\t{elapsed_us}\t{resource}\t{limit}"
			);
		}

		error => {
			println!("regional-pair-v1\tfailed\t{job}\t{phase}\t{elapsed_us}\t{error:?}");
			panic!(
				"unexpected source/arithmetic/invariant/allocation measurement failure: {error:?}"
			);
		}
	}
}

fn report_atlas(
	job: usize,
	side: &str,
	elapsed_us: Option<u128>,
	atlas: &BrushSpace,
	witnesses: [[f32; 3]; 3],
) {
	print!("regional-pair-v1\tatlas\t{job}\t{side}\t");
	match elapsed_us {
		Some(value) => print!("{value}"),
		None => print!("unavailable"),
	}
	let source = atlas.source_statistics();
	print!(
		"\t{}\t{}\t{}\t{}\t{}\t{}",
		atlas.planes().len(),
		atlas.fragments().len(),
		atlas.interfaces().len(),
		atlas.components().len(),
		source.partitions,
		source.world_brushes
	);
	print_statistics(atlas.statistics());
	println!();
	for (id, component) in atlas.components().iter().enumerate() {
		println!(
			"regional-pair-v1\tnode\t{job}\t{side}\t{id}\t{}\t{}",
			component.touches_domain(),
			component.fragments().len()
		);
	}
	for (id, point) in witnesses.into_iter().enumerate() {
		let started = Instant::now();
		let result = atlas.locate(point);
		let elapsed_us = started.elapsed().as_micros();
		match result {
			Ok(PointRelation::Free { fragments }) => {
				println!(
					"regional-pair-v1\tlocate\t{job}\t{side}\t{id}\t{elapsed_us}\tFree\t{}",
					fragments.len()
				);
				for fragment in fragments {
					let component = atlas
						.fragments()
						.get(fragment)
						.expect("public locate fragment must belong to this atlas")
						.component();
					assert!(component < atlas.components().len());
					println!(
						"regional-pair-v1\tmember\t{job}\t{side}\t{id}\t{fragment}\t{component}"
					);
				}
			}

			Ok(relation) => {
				let name = match relation {
					PointRelation::Solid => "Solid",
					PointRelation::Unrepresented => "Unrepresented",
					PointRelation::OutsideDomain => "OutsideDomain",
					PointRelation::Free { .. } => unreachable!(),
				};
				println!("regional-pair-v1\tlocate\t{job}\t{side}\t{id}\t{elapsed_us}\t{name}\t0");
			}

			Err(Error::Budget { resource, limit }) => {
				println!(
					"regional-pair-v1\tlocate_refusal\t{job}\t{side}\t{id}\t{elapsed_us}\t{resource}\t{limit}"
				);
			}

			Err(error) => {
				println!(
					"regional-pair-v1\tfailed\t{job}\tlocate_{side}_{id}\t{elapsed_us}\t{error:?}"
				);
				panic!("unexpected membership measurement failure: {error:?}");
			}
		}
	}
}

fn measure(
	map: &str,
	variable: &str,
	length: usize,
	sha256: &str,
	revision: i32,
	witnesses: [[f32; 3]; 3],
	jobs: [Job; 2],
) {
	// At most six atlas builds, two overlap queries and eighteen fresh locate
	// calls per map: twelve source-traversal phases, with no retry or cap changes.
	// Across the fixed two-map study this is sixteen construction/stitch plus
	// thirty-six membership operations. Membership Statistics are unavailable.
	assert_eq!(GEOMETRY_LIMITS, Limits::default());
	assert_eq!(BSP_LIMITS, BspLimits::default());
	print!("regional-pair-v1\tstart\t{map}\t{length}\t{sha256}");
	print_statistics(Statistics {
		work: GEOMETRY_LIMITS.work,
		predicates: GEOMETRY_LIMITS.predicates,
		exact_predicates: GEOMETRY_LIMITS.exact_predicates,
		cells: GEOMETRY_LIMITS.cells,
		vertices: GEOMETRY_LIMITS.vertices,
		constraints: GEOMETRY_LIMITS.constraints,
	});
	print!(
		"\t{}\t{}",
		OVERLAP_LIMITS.pairs, OVERLAP_LIMITS.certificates
	);
	println!(
		"\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t6\t2\t18\t12",
		BSP_LIMITS.input_bytes,
		BSP_LIMITS.expanded_bytes,
		BSP_LIMITS.dictionary_bytes,
		BSP_LIMITS.probability_bytes,
		BSP_LIMITS.codec_bytes,
		BSP_LIMITS.records,
		BSP_LIMITS.tree_depth,
		BSP_LIMITS.work,
		BSP_LIMITS.partitions,
		BSP_LIMITS.constraints
	);
	let path = std::env::var_os(variable)
		.expect("externally SHA-256-verified private public-map fixture is required");
	let read_started = Instant::now();
	let file = std::fs::File::open(path).expect("open verified fixture; never download");
	let mut bytes = Vec::new();
	let bound = length.checked_add(1).expect("fixed fixture byte bound");
	bytes
		.try_reserve_exact(bound)
		.expect("bounded fixture input allocation");
	file.take(u64::try_from(bound).unwrap())
		.read_to_end(&mut bytes)
		.expect("read bounded private fixture snapshot");
	assert_eq!(bytes.len(), length);
	let input_read_us = read_started.elapsed().as_micros();
	let decode_started = Instant::now();
	let geometry = match BrushGeometry::decode(&bytes, BSP_LIMITS) {
		Ok(geometry) => geometry,

		Err(error) => {
			println!(
				"regional-pair-v1\tfailed\tdecode\tdecode\t{}\t{error:?}",
				decode_started.elapsed().as_micros()
			);
			panic!("unexpected fixture source decode failure: {error:?}");
		}
	};
	let decode_us = decode_started.elapsed().as_micros();
	drop(bytes);
	assert_eq!(geometry.bsp().version(), 20);
	assert_eq!(geometry.bsp().revision(), revision);
	println!("regional-pair-v1\tdecode\t{input_read_us}\t{decode_us}\t{revision}");
	for (id, job) in jobs.into_iter().enumerate() {
		let domain = AnalysisDomain::new(job.min, job.max).unwrap();
		let split =
			RegionalSplit::new(domain, 0, job.split[0], job.split[1], job.split[2]).unwrap();
		print!("regional-pair-v1\ttarget\t{id}\t{}", job.name);
		for value in job.min.into_iter().chain(job.max) {
			print!("\t{value}");
		}
		println!("\t0\t{}\t{}\t{}", job.split[0], job.split[1], job.split[2]);
		let whole_started = Instant::now();
		let whole = BrushSpace::build(&geometry, domain, GEOMETRY_LIMITS);
		let whole_us = whole_started.elapsed().as_micros();
		match whole {
			Ok(atlas) => report_atlas(id, "whole", Some(whole_us), &atlas, witnesses),
			Err(error) => report_error(id, "whole", whole_us, error),
		}
		let pair_started = Instant::now();
		let pair = RegionalPair::build(
			&geometry,
			split,
			GEOMETRY_LIMITS,
			GEOMETRY_LIMITS,
			OVERLAP_LIMITS,
		);
		let pair_us = pair_started.elapsed().as_micros();
		match pair {
			Ok(pair) => {
				let overlap = pair.overlap_statistics();
				let total = sum_statistics(
					pair.left().statistics(),
					pair.right().statistics(),
					overlap.geometry,
				);
				print!(
					"regional-pair-v1\tpair\t{id}\t{pair_us}\t{}\t{}\t{}\t{}",
					overlap.left_planes,
					overlap.right_planes,
					overlap.candidate_pairs,
					overlap.certificates
				);
				print_statistics(overlap.geometry);
				print_statistics(total);
				println!();
				report_atlas(id, "left", None, pair.left(), witnesses);
				report_atlas(id, "right", None, pair.right(), witnesses);
				assert_eq!(pair.certificates().len(), overlap.certificates);
				for certificate in pair.certificates() {
					let [left, right] = certificate.components();
					let [left_fragment, right_fragment] = certificate.fragments();
					assert_eq!(pair.left().fragments()[left_fragment].component(), left);
					assert_eq!(pair.right().fragments()[right_fragment].component(), right);
					println!(
						"regional-pair-v1\tjoin\t{id}\t{left}\t{right}\t{left_fragment}\t{right_fragment}"
					);
				}
			}

			Err(RegionalPairError::LeftBuild(error)) => report_error(id, "left", pair_us, error),
			Err(RegionalPairError::RightBuild(error)) => report_error(id, "right", pair_us, error),
			Err(RegionalPairError::Overlap(error)) => report_error(id, "overlap", pair_us, error),

			Err(error @ RegionalPairError::Split) => {
				panic!("literal planned split failed: {error:?}")
			}
		}
		println!("regional-pair-v1\ttarget_end\t{id}");
	}
	println!("regional-pair-v1\tend\t2");
}

/// Public BSP SHA-256 8e61224f8bdcc6c461589b85dc4a0823abcc430a66513a1bbcc131777b49ee50.
#[test]
#[ignore = "bounded regional measurements; requires externally SHA-256-verified SOURCE_BSP_ARENA_BYRE"]
fn pinned_arena_byre_whole_and_regional_pair() {
	measure(
		"arena_byre",
		"SOURCE_BSP_ARENA_BYRE",
		16_843_394,
		"8e61224f8bdcc6c461589b85dc4a0823abcc430a66513a1bbcc131777b49ee50",
		2059,
		[
			[-436.0, 640.0, 339.0],
			[-436.0, 634.0, 339.0],
			[-436.0, 646.0, 339.0],
		],
		[
			Job {
				name: "control_half64",
				min: [-500.0, 576.0, 275.0],
				max: [-372.0, 704.0, 403.0],
				split: [-452.0, -436.0, -420.0],
			},
			Job {
				name: "refused_half256",
				min: [-692.0, 384.0, 83.0],
				max: [-180.0, 896.0, 595.0],
				split: [-500.0, -436.0, -372.0],
			},
		],
	);
}

/// Public BSP SHA-256 cbd191411c0be57099da73458167001ec80d58bf37c71cb3c36b2911b6e80fd7.
#[test]
#[ignore = "bounded regional measurements; requires externally SHA-256-verified SOURCE_BSP_CTF_2FORT"]
fn pinned_ctf_2fort_whole_and_regional_pair() {
	measure(
		"ctf_2fort",
		"SOURCE_BSP_CTF_2FORT",
		22_751_863,
		"cbd191411c0be57099da73458167001ec80d58bf37c71cb3c36b2911b6e80fd7",
		4067,
		[
			[723.0, 2291.0, 164.0],
			[723.0, 2287.0, 164.0],
			[723.0, 2295.0, 164.0],
		],
		[
			Job {
				name: "control_half256",
				min: [467.0, 2035.0, -92.0],
				max: [979.0, 2547.0, 420.0],
				split: [659.0, 723.0, 787.0],
			},
			Job {
				name: "refused_half1024",
				min: [-301.0, 1267.0, -860.0],
				max: [1747.0, 3315.0, 1188.0],
				split: [659.0, 723.0, 787.0],
			},
		],
	);
}
