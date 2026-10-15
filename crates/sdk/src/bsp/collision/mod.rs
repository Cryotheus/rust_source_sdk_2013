//! Collision geometry of a Source BSP, as polygons and triangles for building
//! navigation.
//!
//! Each brush becomes a convex polyhedron by clipping a large polygon on every
//! side's plane against the brush's other planes. Each displacement becomes its
//! tessellated triangle grid, built on its base face like the engine builds it.
//! Everything is decoded from owned bytes with no engine calls. With the
//! `parallel` feature, the per-brush and per-displacement work runs on rayon's
//! current thread pool, so a caller that installs a bounded pool bounds it.
//!
//! Brush contents are kept as the serialized bits (solid, player clip, NPC clip,
//! water, and the rest), so a consumer decides what blocks which actor. Static
//! props and the transforms of brush entities are not applied here.

#[cfg(test)]
mod tests;

use super::brushes::{BrushError, BrushGeometry};
use super::lumps::{Budget, Directory};
use super::{Error, Invalid, Limits, Plane, Resource, Work};
use crate::math::glam::{DVec3, Vec3};
use sdk_raw::bsp as raw;

/// Clip distance under which a point counts as on a plane.
const EPSILON: f64 = 0.01;

/// Half the side of the starting polygon on each plane, beyond any map's extent.
const HALF_EXTENT: f64 = 65_536.0;

/// Polygons with less area than this are clipping slivers and are dropped.
const MIN_AREA: f64 = 0.01;

/// A failure to decode collision geometry.
#[derive(Debug, thiserror::Error)]
pub enum CollisionError {
	#[error(transparent)]
	Bsp(#[from] Error),
	#[error(transparent)]
	Brushes(#[from] BrushError),
	#[error("displacement {displacement} is built on face {face}, which is not a quad")]
	DisplacementFace { displacement: usize, face: usize },
	#[error("displacement {displacement} has unsupported power {power}")]
	DisplacementPower { displacement: usize, power: i32 },
}

/// The decoded collision geometry of one map.
#[derive(Debug, Clone, Default)]
pub struct CollisionGeometry {
	/// One per brush with at least one non-degenerate face, in brush order.
	pub solids: Vec<Solid>,
	/// One per displacement record, in record order.
	pub displacements: Vec<Displacement>,
}

impl CollisionGeometry {
	/// Decodes the brushes and displacements of a BSP.
	pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self, CollisionError> {
		let geometry = BrushGeometry::decode(bytes, limits)?;
		let solids = solids(&geometry)?;
		let displacements = displacements(bytes, limits, geometry.bsp().planes())?;
		Ok(Self {
			solids,
			displacements,
		})
	}

	/// Every triangle of every solid face and displacement, with its owner.
	///
	/// Solid faces are fanned from their first point. Each triangle's points
	/// wind counter-clockwise seen from the side its normal faces.
	pub fn triangles(&self) -> impl Iterator<Item = Triangle> + '_ {
		let solids = self.solids.iter().enumerate().flat_map(|(index, solid)| {
			solid.faces.iter().flat_map(move |face| {
				(1..face.points.len() - 1).map(move |corner| Triangle {
					points: [face.points[0], face.points[corner], face.points[corner + 1]],
					normal: face.normal,
					contents: solid.contents,
					owner: Owner::Solid(index as u32),
					tags: 0,
				})
			})
		});
		let displacements =
			self.displacements
				.iter()
				.enumerate()
				.flat_map(|(index, displacement)| {
					displacement.triangles.iter().map(move |triangle| {
						let [a, b, c] = triangle
							.indices
							.map(|vertex| displacement.vertices[vertex as usize]);
						Triangle {
							points: [a, b, c],
							normal: (b - a).cross(c - a).normalize_or_zero(),
							contents: displacement.contents,
							owner: Owner::Displacement(index as u32),
							tags: triangle.tags,
						}
					})
				});
		solids.chain(displacements)
	}
}

/// A tessellated displacement surface.
#[derive(Debug, Clone)]
pub struct Displacement {
	/// The face it is built on.
	pub face: u32,
	/// Serialized contents bits.
	pub contents: u32,
	/// Rows and columns of vertices are `2^power + 1`.
	pub power: u32,
	/// Row-major vertex grid, starting at the corner nearest its start position.
	pub vertices: Vec<Vec3>,
	/// Two triangles per grid cell, in the engine's order.
	pub triangles: Vec<DisplacementTriangle>,
}

/// One displacement triangle and its tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplacementTriangle {
	/// Indices into the displacement's vertices, counter-clockwise from above
	/// the base face.
	pub indices: [u32; 3],
	/// Serialized tags, such as [`raw::DISP_TRI_TAG_WALKABLE`].
	pub tags: u16,
}

/// What a [`Triangle`] came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
	/// Index into [`CollisionGeometry::solids`].
	Solid(u32),
	/// Index into [`CollisionGeometry::displacements`].
	Displacement(u32),
}

/// A convex face of a solid, with its points counter-clockwise seen from
/// outside the brush.
#[derive(Debug, Clone)]
pub struct Polygon {
	/// Outward unit normal.
	pub normal: Vec3,
	pub points: Vec<Vec3>,
}

/// A brush as a convex polyhedron.
#[derive(Debug, Clone)]
pub struct Solid {
	/// Source brush index.
	pub brush: u32,
	/// The model whose tree first lists this brush; zero is the world. `None`
	/// for brushes no model reaches.
	pub model: Option<u32>,
	/// Serialized contents bits.
	pub contents: u32,
	/// Non-degenerate faces; bevel sides make none.
	pub faces: Vec<Polygon>,
	/// Bounds of the faces' points.
	pub min: Vec3,
	pub max: Vec3,
}

/// One triangle of the collision geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Triangle {
	pub points: [Vec3; 3],
	/// Unit normal, facing out of a solid or up from a displacement's face.
	pub normal: Vec3,
	pub contents: u32,
	pub owner: Owner,
	/// Displacement triangle tags; zero for solids.
	pub tags: u16,
}

/// Clips `points` to the back of `plane`, keeping what lies within [`EPSILON`].
fn clip(points: &[DVec3], normal: DVec3, distance: f64) -> Vec<DVec3> {
	let mut kept = Vec::with_capacity(points.len() + 1);
	for (index, &point) in points.iter().enumerate() {
		let next = points[(index + 1) % points.len()];
		let here = point.dot(normal) - distance;
		let there = next.dot(normal) - distance;
		if here <= EPSILON {
			kept.push(point);
		}
		if (here < -EPSILON && there > EPSILON) || (here > EPSILON && there < -EPSILON) {
			kept.push(point + (next - point) * (here / (here - there)));
		}
	}
	kept
}

fn displacements(
	bytes: &[u8],
	limits: Limits,
	planes: &[Plane],
) -> Result<Vec<Displacement>, CollisionError> {
	let mut work = Work::new(limits.work);
	let directory = Directory::read(bytes, limits, &mut work)?;
	let mut budget = Budget::new(limits);
	let lump =
		|id, versions: &[i32], width, budget: &mut Budget, work: &mut Work| -> Result<_, Error> {
			let lump = directory.prepare(id, versions, budget, work)?;
			if lump.length % width != 0 {
				return Err(Invalid::RecordSize { lump: id }.into());
			}
			lump.decode()
		};
	let infos = lump(
		raw::LUMP_DISP_INFO,
		&[0],
		raw::DISP_INFO_BYTES,
		&mut budget,
		&mut work,
	)?;
	if infos.is_empty() {
		return Ok(Vec::new());
	}
	let verts = lump(
		raw::LUMP_DISP_VERTS,
		&[0],
		raw::DISP_VERT_BYTES,
		&mut budget,
		&mut work,
	)?;
	let tris = lump(
		raw::LUMP_DISP_TRIS,
		&[0],
		raw::DISP_TRI_BYTES,
		&mut budget,
		&mut work,
	)?;
	let faces = lump(
		raw::LUMP_FACES,
		&[1],
		raw::FACE_BYTES,
		&mut budget,
		&mut work,
	)?;
	let edges = lump(
		raw::LUMP_EDGES,
		&[0],
		raw::EDGE_BYTES,
		&mut budget,
		&mut work,
	)?;
	let surfedges = lump(
		raw::LUMP_SURFEDGES,
		&[0],
		raw::SURFEDGE_BYTES,
		&mut budget,
		&mut work,
	)?;
	let vertexes = lump(
		raw::LUMP_VERTEXES,
		&[0],
		raw::VERTEX_BYTES,
		&mut budget,
		&mut work,
	)?;
	let count = infos.len() / raw::DISP_INFO_BYTES;
	let reference = |lump, record| CollisionError::Bsp(Invalid::Reference { lump, record }.into());

	let build = |index: usize| -> Result<Displacement, CollisionError> {
		let info = &infos[index * raw::DISP_INFO_BYTES..][..raw::DISP_INFO_BYTES];
		let start = vector(info, 0);
		let first_vert = integer(info, 12);
		let first_tri = integer(info, 16);
		let power = integer(info, 20);
		let contents = integer(info, 32) as u32;
		let face = usize::from(unsigned_short(info, 36));
		if !(2..=4).contains(&power) {
			return Err(CollisionError::DisplacementPower {
				displacement: index,
				power,
			});
		}
		let size = (1usize << power) + 1;
		let record = faces
			.get(face * raw::FACE_BYTES..(face + 1) * raw::FACE_BYTES)
			.ok_or_else(|| reference(raw::LUMP_DISP_INFO, index))?;
		if short(record, raw::FACE_EDGE_COUNT_OFFSET) != 4 {
			return Err(CollisionError::DisplacementFace {
				displacement: index,
				face,
			});
		}
		let first_edge = integer(record, raw::FACE_FIRST_EDGE_OFFSET);
		let mut corners = [DVec3::ZERO; 4];
		for (corner, slot) in corners.iter_mut().enumerate() {
			let at = usize::try_from(first_edge)
				.ok()
				.and_then(|first| first.checked_add(corner))
				.and_then(|at| at.checked_mul(raw::SURFEDGE_BYTES))
				.filter(|&at| at + raw::SURFEDGE_BYTES <= surfedges.len())
				.ok_or_else(|| reference(raw::LUMP_FACES, face))?;
			let surfedge = integer(&surfedges, at);
			let edge = surfedge.unsigned_abs() as usize * raw::EDGE_BYTES;
			let end = if surfedge >= 0 { 0 } else { 2 };
			let vertex = edges
				.get(edge..edge + raw::EDGE_BYTES)
				.map(|edge| usize::from(unsigned_short(edge, end)) * raw::VERTEX_BYTES)
				.filter(|&vertex| vertex + raw::VERTEX_BYTES <= vertexes.len())
				.ok_or_else(|| reference(raw::LUMP_SURFEDGES, at / raw::SURFEDGE_BYTES))?;
			*slot = vector(&vertexes, vertex);
		}
		// The engine starts the grid at the corner nearest the start position.
		let nearest = (0..4)
			.min_by(|&a, &b| {
				corners[a]
					.distance_squared(start)
					.total_cmp(&corners[b].distance_squared(start))
			})
			.unwrap_or(0);
		corners.rotate_left(nearest);

		let vertex_count = size * size;
		let verts_at = usize::try_from(first_vert)
			.ok()
			.filter(|&first| (first + vertex_count) * raw::DISP_VERT_BYTES <= verts.len())
			.ok_or_else(|| reference(raw::LUMP_DISP_INFO, index))?;
		let step = 1.0 / (size - 1) as f64;
		let mut vertices = Vec::with_capacity(vertex_count);
		for row in 0..size {
			let along = row as f64 * step;
			let left = corners[0].lerp(corners[1], along);
			let right = corners[3].lerp(corners[2], along);
			for column in 0..size {
				let vert = &verts[(verts_at + row * size + column) * raw::DISP_VERT_BYTES..];
				let base = left.lerp(right, column as f64 * step);
				let offset = vector(vert, 0) * f64::from(float(vert, 12));
				vertices.push((base + offset).as_vec3());
			}
		}

		let cells = size - 1;
		let tri_count = cells * cells * 2;
		let tags_at = usize::try_from(first_tri)
			.ok()
			.filter(|&first| (first + tri_count) * raw::DISP_TRI_BYTES <= tris.len());
		let plane = planes
			.get(usize::from(unsigned_short(record, raw::FACE_PLANE_OFFSET)))
			.ok_or_else(|| reference(raw::LUMP_FACES, face))?;
		let mut up = DVec3::from_array(plane.normal.map(f64::from));
		if record[raw::FACE_SIDE_OFFSET] != 0 {
			up = -up;
		}
		let mut triangles = Vec::with_capacity(tri_count);
		for row in 0..cells {
			for column in 0..cells {
				let at = (row * size + column) as u32;
				let size = size as u32;
				let pair = if (row * cells + column) % 2 == 1 {
					[[at, at + size, at + 1], [at + 1, at + size, at + size + 1]]
				} else {
					[[at, at + size, at + size + 1], [at, at + size + 1, at + 1]]
				};
				for mut indices in pair {
					let tags = tags_at.map_or(0, |first| {
						unsigned_short(&tris, (first + triangles.len()) * raw::DISP_TRI_BYTES)
					});
					let [a, b, c] = indices.map(|vertex| vertices[vertex as usize].as_dvec3());
					if (b - a).cross(c - a).dot(up) < 0.0 {
						indices.swap(1, 2);
					}
					triangles.push(DisplacementTriangle { indices, tags });
				}
			}
		}
		Ok(Displacement {
			face: face as u32,
			contents,
			power: power as u32,
			vertices,
			triangles,
		})
	};
	each(count, build).into_iter().collect()
}

/// Runs `f` for every index below `count`, in parallel with the `parallel`
/// feature, keeping index order.
fn each<T: Send>(count: usize, f: impl Fn(usize) -> T + Sync + Send) -> Vec<T> {
	#[cfg(feature = "parallel")]
	{
		use rayon::prelude::*;
		(0..count).into_par_iter().map(f).collect()
	}
	#[cfg(not(feature = "parallel"))]
	{
		(0..count).map(f).collect()
	}
}

fn float(bytes: &[u8], at: usize) -> f32 {
	f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn integer(bytes: &[u8], at: usize) -> i32 {
	i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The convex face of a brush on the side `index`, or `None` if it is degenerate.
fn polygon(planes: &[(DVec3, f64)], index: usize) -> Option<Vec<DVec3>> {
	let (normal, distance) = planes[index];
	let axis = if normal.z.abs() > normal.x.abs().max(normal.y.abs()) {
		DVec3::X
	} else {
		DVec3::Z
	};
	let right = axis.cross(normal).normalize() * HALF_EXTENT;
	let up = normal.cross(right).normalize() * HALF_EXTENT;
	let center = normal * distance;
	// Counter-clockwise seen from the front of the plane.
	let mut points = vec![
		center - right - up,
		center + right - up,
		center + right + up,
		center - right + up,
	];
	if (points[1] - points[0])
		.cross(points[2] - points[0])
		.dot(normal)
		< 0.0
	{
		points.reverse();
	}
	for (other, &(normal, distance)) in planes.iter().enumerate() {
		if other != index {
			points = clip(&points, normal, distance);
			if points.len() < 3 {
				return None;
			}
		}
	}
	let mut area = DVec3::ZERO;
	for corner in 1..points.len() - 1 {
		area += (points[corner] - points[0]).cross(points[corner + 1] - points[0]);
	}
	(area.length() * 0.5 >= MIN_AREA).then_some(points)
}

fn short(bytes: &[u8], at: usize) -> i16 {
	i16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn solids(geometry: &BrushGeometry) -> Result<Vec<Solid>, CollisionError> {
	let brushes = geometry.brushes();
	let mut owners = Vec::new();
	owners
		.try_reserve(brushes.len())
		.map_err(|_| Error::Allocation {
			resource: Resource::Records,
		})?;
	owners.resize(brushes.len(), None);
	for model in (0..geometry.bsp().models().len()).rev() {
		for brush in geometry.model_brushes(model)? {
			owners[brush] = Some(model as u32);
		}
	}
	let planes = geometry.bsp().planes();
	let sides = geometry.sides();
	let solids = each(brushes.len(), |index| {
		let brush = &brushes[index];
		let sides = &sides[brush.sides.clone()];
		let constraints: Vec<(DVec3, f64)> = sides
			.iter()
			.map(|side| {
				let plane = planes[side.plane];
				let normal = DVec3::from_array(plane.normal.map(f64::from));
				let length = normal.length();
				(normal / length, f64::from(plane.distance) / length)
			})
			.collect();
		let mut faces = Vec::new();
		let mut min = DVec3::splat(f64::INFINITY);
		let mut max = DVec3::splat(f64::NEG_INFINITY);
		for (index, side) in sides.iter().enumerate() {
			if side.bevel != 0 {
				continue;
			}
			let (normal, _) = constraints[index];
			if let Some(points) = polygon(&constraints, index) {
				for &point in &points {
					min = min.min(point);
					max = max.max(point);
				}
				faces.push(Polygon {
					normal: normal.as_vec3(),
					points: points.iter().map(|point| point.as_vec3()).collect(),
				});
			}
		}
		(!faces.is_empty()).then(|| Solid {
			brush: index as u32,
			model: owners[index],
			contents: brush.contents,
			faces,
			min: min.as_vec3(),
			max: max.as_vec3(),
		})
	});
	Ok(solids.into_iter().flatten().collect())
}

fn unsigned_short(bytes: &[u8], at: usize) -> u16 {
	u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn vector(bytes: &[u8], at: usize) -> DVec3 {
	DVec3::new(
		f64::from(float(bytes, at)),
		f64::from(float(bytes, at + 4)),
		f64::from(float(bytes, at + 8)),
	)
}
