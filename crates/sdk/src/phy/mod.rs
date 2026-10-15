//! Collision hulls from a model's `.phy` file.
//!
//! A `.phy` holds one or more solids, each an IVP compact surface: a tree of
//! convex pieces ("ledges") made of triangles over shared points. Only the
//! tree's leaf ledges are kept, as the inner ones only bound their children.
//! Points come out in Source inches and the model's own axes. MOPP surfaces
//! and the legacy format without a `VPHY` header are not supported.

#[cfg(test)]
mod tests;

use crate::math::glam::Vec3;

/// Bytes of the file header: size, id, solid count, checksum.
const FILE_HEADER_BYTES: usize = 16;

/// IVP measures in meters.
const INCHES_PER_METER: f32 = 1.0 / 0.0254;

/// Bytes of an IVP compact ledge before its triangles.
const LEDGE_BYTES: usize = 16;

/// Deepest ledge tree walked; real trees are a few dozen levels at most.
const MAX_DEPTH: usize = 256;

/// Bytes of an IVP ledge tree node.
const NODE_BYTES: usize = 28;

/// Bytes of an IVP compact point: three coordinates and a padding float.
const POINT_BYTES: usize = 16;

/// Bytes of the `VPHY` header before the IVP compact surface.
const SOLID_HEADER_BYTES: usize = 28;

/// Bytes of the IVP compact surface header.
const SURFACE_BYTES: usize = 48;

/// Bytes of an IVP compact triangle.
const TRIANGLE_BYTES: usize = 16;

/// One convex piece of a solid.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ConvexHull {
	pub points: Vec<Vec3>,
	/// Indices into `points`.
	pub triangles: Vec<[u32; 3]>,
}

/// A failure to read a `.phy` file.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PhyError {
	#[error("the .phy file is truncated or has an out-of-range offset")]
	Truncated,
	#[error("solid {solid} has no VPHY header; the legacy format is unsupported")]
	Legacy { solid: usize },
	#[error("solid {solid} is model type {model_type}; only compact surfaces are supported")]
	ModelType { solid: usize, model_type: i16 },
	#[error("a ledge tree is deeper than {MAX_DEPTH} levels")]
	Depth,
}

/// Every convex piece of every solid in a `.phy` file.
pub fn decode(bytes: &[u8]) -> Result<Vec<ConvexHull>, PhyError> {
	let header_size = usize::try_from(integer(bytes, 0)?).map_err(|_| PhyError::Truncated)?;
	let solids = usize::try_from(integer(bytes, 8)?).map_err(|_| PhyError::Truncated)?;
	let mut at = header_size.max(FILE_HEADER_BYTES);
	let mut hulls = Vec::new();
	for solid in 0..solids {
		let size = usize::try_from(integer(bytes, at)?).map_err(|_| PhyError::Truncated)?;
		let data = bytes
			.get(at + 4..at + 4 + size)
			.ok_or(PhyError::Truncated)?;
		at += 4 + size;
		if data.get(..4) != Some(b"VPHY") {
			return Err(PhyError::Legacy { solid });
		}
		let model_type = i16::from_le_bytes([data[6], data[7]]);
		if model_type != 0 {
			return Err(PhyError::ModelType { solid, model_type });
		}
		let surface = data.get(SOLID_HEADER_BYTES..).ok_or(PhyError::Truncated)?;
		if surface.len() < SURFACE_BYTES {
			return Err(PhyError::Truncated);
		}
		let root = usize::try_from(integer(surface, 32)?).map_err(|_| PhyError::Truncated)?;
		let mut nodes = vec![(root, 0)];
		while let Some((node, depth)) = nodes.pop() {
			if depth > MAX_DEPTH {
				return Err(PhyError::Depth);
			}
			let right = integer(surface, node)?;
			if right == 0 {
				let ledge = offset(node, integer(surface, node + 4)?)?;
				hulls.push(ledge_hull(surface, ledge)?);
			} else {
				nodes.push((offset(node, right)?, depth + 1));
				nodes.push((node + NODE_BYTES, depth + 1));
			}
		}
	}
	Ok(hulls)
}

fn float(bytes: &[u8], at: usize) -> Result<f32, PhyError> {
	Ok(f32::from_bits(integer(bytes, at)? as u32))
}

fn integer(bytes: &[u8], at: usize) -> Result<i32, PhyError> {
	let word = bytes
		.get(at..at.checked_add(4).ok_or(PhyError::Truncated)?)
		.ok_or(PhyError::Truncated)?;
	Ok(i32::from_le_bytes([word[0], word[1], word[2], word[3]]))
}

/// Reads one compact ledge's triangles and the points they use.
fn ledge_hull(surface: &[u8], ledge: usize) -> Result<ConvexHull, PhyError> {
	let points_at = offset(ledge, integer(surface, ledge)?)?;
	let triangle_count = surface
		.get(ledge + 12..ledge + 14)
		.map(|count| i16::from_le_bytes([count[0], count[1]]))
		.ok_or(PhyError::Truncated)?;
	let triangle_count = usize::try_from(triangle_count).map_err(|_| PhyError::Truncated)?;
	let mut hull = ConvexHull::default();
	// IVP point indices are sparse within a shared array; renumber them densely.
	let mut renumbered = std::collections::HashMap::new();
	for triangle in 0..triangle_count {
		let at = ledge + LEDGE_BYTES + triangle * TRIANGLE_BYTES;
		let mut indices = [0; 3];
		for (edge, index) in indices.iter_mut().enumerate() {
			let point = (integer(surface, at + 4 + edge * 4)? as u32 & 0xffff) as usize;
			let next = renumbered.len() as u32;
			*index = match renumbered.entry(point) {
				std::collections::hash_map::Entry::Occupied(entry) => *entry.get(),

				std::collections::hash_map::Entry::Vacant(entry) => {
					let at = points_at + point * POINT_BYTES;
					let [x, y, z] = [
						float(surface, at)?,
						float(surface, at + 4)?,
						float(surface, at + 8)?,
					];
					// IVP's axes are Source's x, -z and y.
					hull.points.push(Vec3::new(x, z, -y) * INCHES_PER_METER);
					*entry.insert(next)
				}
			};
		}
		hull.triangles.push(indices);
	}
	Ok(hull)
}

fn offset(base: usize, relative: i32) -> Result<usize, PhyError> {
	base.checked_add_signed(relative as isize)
		.ok_or(PhyError::Truncated)
}
