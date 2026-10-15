use super::*;

#[test]
fn a_chamfer_cuts_a_corner_off_the_box() {
	let mut planes = box_planes(DVec3::splat(-8.0), DVec3::splat(8.0));
	let normal = DVec3::new(1.0, 1.0, 0.0).normalize();
	planes.push((normal, 8.0));
	let chamfer = polygon(&planes, planes.len() - 1).expect("the chamfer is a face");
	assert_eq!(chamfer.len(), 4);
	let top = polygon(&planes, 4).expect("the top remains");
	assert_eq!(top.len(), 5);
}

#[test]
fn a_plane_that_misses_the_brush_makes_no_face() {
	let mut planes = box_planes(DVec3::splat(-8.0), DVec3::splat(8.0));
	// Lies wholly outside the box, so clipping by the others removes it.
	planes.push((DVec3::Z, 100.0));
	assert!(polygon(&planes, planes.len() - 1).is_none());
}

#[test]
fn box_faces_are_its_six_sides_wound_outward() {
	let planes = box_planes(DVec3::new(-16.0, -32.0, 0.0), DVec3::new(16.0, 32.0, 64.0));
	for (index, &(normal, distance)) in planes.iter().enumerate() {
		let points = polygon(&planes, index).expect("every box side is a face");
		assert_eq!(points.len(), 4);
		for point in &points {
			assert!((point.dot(normal) - distance).abs() < 1e-6);
			assert!(point.x.abs() <= 16.0 + 1e-6 && point.y.abs() <= 32.0 + 1e-6);
			assert!((-1e-6..=64.0 + 1e-6).contains(&point.z));
		}
		let winding = (points[1] - points[0]).cross(points[2] - points[0]);
		assert!(winding.dot(normal) > 0.0, "side {index} winds inward");
	}
}

/// The six planes of an axis-aligned box, outward, as brush sides store them.
fn box_planes(min: DVec3, max: DVec3) -> Vec<(DVec3, f64)> {
	vec![
		(DVec3::X, max.x),
		(-DVec3::X, -min.x),
		(DVec3::Y, max.y),
		(-DVec3::Y, -min.y),
		(DVec3::Z, max.z),
		(-DVec3::Z, -min.z),
	]
}

#[test]
#[ignore = "needs SOURCE_BSP_EXPORT and a map; writes Wavefront OBJ files for inspection"]
fn export_obj() {
	use std::io::Write;
	let output =
		std::path::PathBuf::from(std::env::var_os("SOURCE_BSP_EXPORT").expect("export directory"));
	for variable in ["SOURCE_BSP_CTF_2FORT", "SOURCE_BSP_PL_SNOWYCOAST"] {
		let Some(bytes) = fixture(variable) else {
			continue;
		};
		let limits = Limits {
			records: 4_000_000,
			work: 50_000_000,
			..Limits::default()
		};
		let geometry = CollisionGeometry::decode(&bytes, limits).expect("decodes");
		let mut file = std::io::BufWriter::new(
			std::fs::File::create(output.join(format!("{variable}.obj"))).unwrap(),
		);
		let mut vertex = 1;
		for triangle in geometry.triangles() {
			if triangle.contents & 1 == 0 {
				continue;
			}
			let model = match triangle.owner {
				Owner::Solid(solid) => geometry.solids[solid as usize].model,
				Owner::Displacement(_) => Some(0),
			};
			if model != Some(0) {
				continue;
			}
			for point in triangle.points {
				writeln!(file, "v {} {} {}", point.x, point.y, point.z).unwrap();
			}
			writeln!(file, "f {} {} {}", vertex, vertex + 1, vertex + 2).unwrap();
			vertex += 3;
		}
	}
}

/// Reads a fixture map named by an environment variable, or skips.
fn fixture(variable: &str) -> Option<Vec<u8>> {
	let path = std::env::var_os(variable)?;
	Some(std::fs::read(path).expect("read the local fixture; this test never downloads"))
}

#[test]
#[ignore = "needs SOURCE_BSP_CTF_2FORT and friends pointing at local maps"]
fn real_maps_decode_into_bounded_geometry() {
	let maps = [
		"SOURCE_BSP_CTF_2FORT",
		"SOURCE_BSP_ARENA_BYRE",
		"SOURCE_BSP_CP_DUSTBOWL",
		"SOURCE_BSP_PL_GOLDRUSH",
		"SOURCE_BSP_CTF_WELL",
		"SOURCE_BSP_CTF_SAWMILL",
		"SOURCE_BSP_CTF_TURBINE",
		"SOURCE_BSP_PL_SNOWYCOAST",
	];
	let limits = Limits {
		records: 4_000_000,
		work: 50_000_000,
		..Limits::default()
	};
	let mut tested = 0;
	for variable in maps {
		let Some(bytes) = fixture(variable) else {
			continue;
		};
		let started = std::time::Instant::now();
		let geometry = CollisionGeometry::decode(&bytes, limits).expect("decodes");
		let elapsed = started.elapsed();
		let triangles: Vec<Triangle> = geometry.triangles().collect();
		let walkable = geometry
			.displacements
			.iter()
			.flat_map(|displacement| &displacement.triangles)
			.filter(|triangle| triangle.tags & raw::DISP_TRI_TAG_WALKABLE != 0)
			.count();
		println!(
			"{variable}: {} solids, {} displacements ({walkable} walkable triangles), {} triangles in {elapsed:?}",
			geometry.solids.len(),
			geometry.displacements.len(),
			triangles.len(),
		);
		assert!(!geometry.solids.is_empty());
		for triangle in &triangles {
			for point in triangle.points {
				assert!(
					point.is_finite() && point.abs().max_element() < 32_768.0,
					"{variable}: {point}"
				);
			}
		}
		// Displacements are terrain, so most of their triangles face up.
		let (up, all) = triangles
			.iter()
			.filter(|triangle| matches!(triangle.owner, Owner::Displacement(_)))
			.fold((0, 0), |(up, all), triangle| {
				(up + usize::from(triangle.normal.z > 0.0), all + 1)
			});
		assert!(
			up * 2 >= all,
			"{variable}: {up} of {all} displacement triangles face up"
		);
		tested += 1;
	}
	assert!(tested > 0, "no fixture maps were named");
}

#[test]
#[ignore = "needs SOURCE_BSP_* variables pointing at local maps"]
fn real_maps_have_static_props() {
	use crate::bsp::props::{PropSolid, StaticProps};
	let mut tested = 0;
	for variable in [
		"SOURCE_BSP_CTF_2FORT",
		"SOURCE_BSP_CTF_WELL",
		"SOURCE_BSP_CTF_SAWMILL",
		"SOURCE_BSP_CTF_TURBINE",
	] {
		let Some(bytes) = fixture(variable) else {
			continue;
		};
		let props = StaticProps::decode(&bytes, Limits::default()).expect("decodes");
		let solid = props
			.props
			.iter()
			.filter(|prop| prop.solid == PropSolid::Physics)
			.count();
		println!(
			"{variable}: v{}, {} models, {} props, {solid} with .phy collision",
			props.version,
			props.models.len(),
			props.props.len()
		);
		assert!(!props.props.is_empty());
		assert!(props.models.iter().all(|model| model.ends_with(".mdl")));
		tested += 1;
	}
	assert!(tested > 0);
}
