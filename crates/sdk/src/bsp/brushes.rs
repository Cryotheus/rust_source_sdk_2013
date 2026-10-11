//! Checked serialized brush regions, separate from complete world collision.
//!
//! Detail brushes can lie inside a nonsolid structural leaf. These records do
//! not turn the partition tree into free-space cells or establish adjacency.
//! A model region is the union of its referenced convex half-space sets. It
//! contains no entity transform, activity, collision group or traversal policy.

#[cfg(test)]
mod tests;

use super::lumps::{Budget, Directory};
use super::{
	Bsp, Child, Error, Invalid, Limits, Plane, Resource, Work, bound, finite, raw, reserve,
};
use std::ops::Range;

/// A checked range in the side array and the serialized contents bits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Brush {
	pub sides: Range<usize>,
	pub contents: u32,
}

/// A checked plane reference and otherwise unresolved serialized side metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrushSide {
	pub plane: usize,
	/// Texture-info reference, retained without decoding that lump.
	pub texinfo: i16,
	/// Displacement-info reference, retained without resolving its geometry.
	pub displacement: i16,
	/// Serialized bevel marker, retained without changing the half-space set.
	pub bevel: i16,
}

/// Errors distinguish absent region data from an empty-space answer.
#[derive(Debug, thiserror::Error)]
pub enum BrushError {
	#[error(transparent)]
	Bsp(#[from] Error),
	#[error("brush {brush} has no side constraints")]
	ZeroSides { brush: usize },
	#[error("model {model} is out of range")]
	ModelIndex { model: usize },
	#[error("model {model} has no referenced brush geometry")]
	NoBrushes { model: usize },
}

/// A structural BSP and the brush references decoded from the same input bytes.
///
/// The partition phase and extra-lump phase each have their own byte/codec/work
/// bounds from Limits. Their retained decoded record counts are additionally
/// checked together. This is not an aggregate two-phase codec or RSS cap.
#[derive(Debug, Clone)]
pub struct BrushGeometry {
	bsp: Bsp,
	brushes: Vec<Brush>,
	sides: Vec<BrushSide>,
	leaf_brushes: Vec<usize>,
	leaf_ranges: Vec<Range<usize>>,
}

impl BrushGeometry {
	/// Decodes brush-side, brush and leaf-brush records with all structural
	/// references checked. Other side metadata remains explicitly unresolved.
	///
	/// Empty brush arrays are allowed as source facts; model_region refuses to
	/// turn their absence into an all-space or empty-room region.
	pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self, BrushError> {
		let bsp = Bsp::decode(bytes, limits)?;
		let mut work = Work::new(limits.work);
		let directory = Directory::read(bytes, limits, &mut work)?;
		let mut budget = Budget::new(limits);
		let leaves = directory.prepare(raw::LUMP_LEAVES, &[0, 1], &mut budget, &mut work)?;
		let references = directory.prepare(raw::LUMP_LEAF_BRUSHES, &[0], &mut budget, &mut work)?;
		let brushes = directory.prepare(raw::LUMP_BRUSHES, &[0], &mut budget, &mut work)?;
		let sides = directory.prepare(raw::LUMP_BRUSH_SIDES, &[0], &mut budget, &mut work)?;
		let leaf_width = if leaves.version == 0 {
			raw::LEAF_V0_BYTES
		} else {
			raw::LEAF_V1_BYTES
		};
		let leaf_count = count(leaves.length, leaf_width, raw::LUMP_LEAVES)?;
		let reference_count = count(
			references.length,
			raw::LEAF_BRUSH_BYTES,
			raw::LUMP_LEAF_BRUSHES,
		)?;
		let brush_count = count(brushes.length, raw::BRUSH_BYTES, raw::LUMP_BRUSHES)?;
		let side_count = count(sides.length, raw::BRUSH_SIDE_BYTES, raw::LUMP_BRUSH_SIDES)?;
		let mut total = 0usize;
		for amount in [
			bsp.planes().len(),
			bsp.nodes().len(),
			bsp.leaves().len(),
			bsp.models().len(),
			leaf_count,
			reference_count,
			brush_count,
			side_count,
		] {
			total = total.checked_add(amount).ok_or(Error::Budget {
				resource: Resource::Records,
				limit: limits.records,
			})?;
			bound(total, limits.records, Resource::Records)?;
		}
		if leaf_count != bsp.leaves().len() {
			return Err(Error::from(Invalid::RecordSize {
				lump: raw::LUMP_LEAVES,
			})
			.into());
		}

		let mut decoded_sides = Vec::new();
		reserve(&mut decoded_sides, side_count, Resource::Records)?;
		let side_bytes = sides.decode()?;
		for (index, record) in side_bytes.chunks_exact(raw::BRUSH_SIDE_BYTES).enumerate() {
			work.spend(1)?;
			let plane = usize::from(short(record, 0));
			if plane >= bsp.planes().len() {
				return Err(Error::from(Invalid::Reference {
					lump: raw::LUMP_BRUSH_SIDES,
					record: index,
				})
				.into());
			}
			decoded_sides.push(BrushSide {
				plane,
				texinfo: short(record, 2) as i16,
				displacement: short(record, 4) as i16,
				bevel: short(record, 6) as i16,
			});
		}
		drop(side_bytes);

		let mut decoded_brushes = Vec::new();
		reserve(&mut decoded_brushes, brush_count, Resource::Records)?;
		let brush_bytes = brushes.decode()?;
		for (index, record) in brush_bytes.chunks_exact(raw::BRUSH_BYTES).enumerate() {
			work.spend(1)?;
			let first = index_value(record, 0, raw::LUMP_BRUSHES, index)?;
			let length = index_value(record, 4, raw::LUMP_BRUSHES, index)?;
			if length == 0 {
				return Err(BrushError::ZeroSides { brush: index });
			}
			let sides = checked_range(first, length, side_count, raw::LUMP_BRUSHES, index)?;
			decoded_brushes.push(Brush {
				sides,
				contents: word(record, 8),
			});
		}
		drop(brush_bytes);

		let mut decoded_references = Vec::new();
		reserve(&mut decoded_references, reference_count, Resource::Records)?;
		let reference_bytes = references.decode()?;
		for (index, record) in reference_bytes
			.chunks_exact(raw::LEAF_BRUSH_BYTES)
			.enumerate()
		{
			work.spend(1)?;
			let brush = usize::from(short(record, 0));
			if brush >= brush_count {
				return Err(Error::from(Invalid::Reference {
					lump: raw::LUMP_LEAF_BRUSHES,
					record: index,
				})
				.into());
			}
			decoded_references.push(brush);
		}
		drop(reference_bytes);

		let mut leaf_ranges = Vec::new();
		reserve(&mut leaf_ranges, leaf_count, Resource::Records)?;
		let leaf_bytes = leaves.decode()?;
		for (index, record) in leaf_bytes.chunks_exact(leaf_width).enumerate() {
			work.spend(1)?;
			leaf_ranges.push(checked_range(
				usize::from(short(record, raw::LEAF_FIRST_BRUSH_OFFSET)),
				usize::from(short(record, raw::LEAF_BRUSH_COUNT_OFFSET)),
				reference_count,
				raw::LUMP_LEAVES,
				index,
			)?);
		}
		Ok(Self {
			bsp,
			brushes: decoded_brushes,
			sides: decoded_sides,
			leaf_brushes: decoded_references,
			leaf_ranges,
		})
	}

	pub fn bsp(&self) -> &Bsp {
		&self.bsp
	}

	pub fn brushes(&self) -> &[Brush] {
		&self.brushes
	}

	pub fn sides(&self) -> &[BrushSide] {
		&self.sides
	}

	/// Source brush indices listed by a leaf; duplicates and ordering survive.
	pub fn leaf_brushes(&self, leaf: usize) -> Result<&[usize], BrushError> {
		let range = self
			.leaf_ranges
			.get(leaf)
			.ok_or(Error::from(Invalid::Reference {
				lump: raw::LUMP_LEAVES,
				record: leaf,
			}))?;
		Ok(&self.leaf_brushes[range.clone()])
	}

	/// Ascending unique brush indices reachable from this model's root.
	///
	/// Model zero is world ownership. A brush can be referenced by several
	/// leaves or models; no exclusive owner or native-engine brush ID is assumed.
	pub fn model_brushes(&self, model: usize) -> Result<Vec<usize>, BrushError> {
		self.collect(model, &mut Work::new(self.bsp.limits.work))
	}

	/// Copies the selected model's actual side planes into an owned region.
	///
	/// The stored model origin is metadata, not an applied world transform.
	/// Inconsistent or unbounded constraints remain such; this operation proves
	/// neither nonempty interior nor enclosure.
	pub fn model_region(&self, model: usize) -> Result<ModelRegion, BrushError> {
		self.model_region_bounded(model, self.bsp.limits.constraints)
	}

	/// Copies a region with an additional caller-owned constraint allowance.
	///
	/// The smaller of this allowance and the snapshot's per-query limit is
	/// checked before any plane copies. This lets an atlas bound all retained
	/// regions together; work still has the snapshot's independent per-call cap.
	pub fn model_region_bounded(
		&self,
		model: usize,
		max_constraints: usize,
	) -> Result<ModelRegion, BrushError> {
		let limit = max_constraints.min(self.bsp.limits.constraints);
		let mut work = Work::new(self.bsp.limits.work);
		let indices = self.collect(model, &mut work)?;
		if indices.is_empty() {
			return Err(BrushError::NoBrushes { model });
		}
		let mut constraints = 0usize;
		for &index in &indices {
			work.spend(1)?;
			constraints = constraints
				.checked_add(self.brushes[index].sides.len())
				.ok_or(Error::Budget {
					resource: Resource::Constraints,
					limit,
				})?;
			bound(constraints, limit, Resource::Constraints)?;
		}
		let mut brushes = Vec::new();
		reserve(&mut brushes, indices.len(), Resource::Records)?;
		for index in indices {
			work.spend(1)?;
			let brush = &self.brushes[index];
			let mut planes = Vec::new();
			reserve(&mut planes, brush.sides.len(), Resource::Constraints)?;
			for side in &self.sides[brush.sides.clone()] {
				work.spend(1)?;
				planes.push(self.bsp.planes()[side.plane]);
			}
			brushes.push(RegionBrush {
				source: index,
				contents: brush.contents,
				planes,
			});
		}
		Ok(ModelRegion {
			model,
			origin: self.bsp.models()[model].origin,
			brushes,
			constraints,
			work: self.bsp.limits.work,
		})
	}

	fn collect(&self, model: usize, work: &mut Work) -> Result<Vec<usize>, BrushError> {
		let head = self
			.bsp
			.models()
			.get(model)
			.ok_or(BrushError::ModelIndex { model })?
			.head;
		let mut nodes = flags(self.bsp.nodes().len(), work)?;
		let mut leaves = flags(self.bsp.leaves().len(), work)?;
		let mut brushes = flags(self.brushes.len(), work)?;
		let mut stack = Vec::new();
		reserve(&mut stack, 1, Resource::Records)?;
		stack.push(head);
		while let Some(child) = stack.pop() {
			work.spend(1)?;
			match child {
				Child::Node(index) => {
					if nodes[index] {
						continue;
					}
					nodes[index] = true;
					reserve(&mut stack, 2, Resource::Records)?;
					stack.extend(self.bsp.nodes()[index].children.into_iter().rev());
				}

				Child::Leaf(index) => {
					if leaves[index] {
						continue;
					}
					leaves[index] = true;
					for &brush in &self.leaf_brushes[self.leaf_ranges[index].clone()] {
						work.spend(1)?;
						brushes[brush] = true;
					}
				}
			}
		}
		let mut indices = Vec::new();
		for (index, included) in brushes.into_iter().enumerate() {
			work.spend(1)?;
			if included {
				reserve(&mut indices, 1, Resource::Records)?;
				indices.push(index);
			}
		}
		Ok(indices)
	}
}

/// A region remains usable after its source BrushGeometry is dropped.
#[derive(Debug, Clone)]
pub struct ModelRegion {
	model: usize,
	origin: [f32; 3],
	brushes: Vec<RegionBrush>,
	constraints: usize,
	work: usize,
}

impl ModelRegion {
	pub fn model(&self) -> usize {
		self.model
	}

	pub fn stored_origin(&self) -> [f32; 3] {
		self.origin
	}

	pub fn brushes(&self) -> &[RegionBrush] {
		&self.brushes
	}

	/// Total copied side constraints across this region's unique brushes.
	pub fn constraint_count(&self) -> usize {
		self.constraints
	}

	/// Tests the union of closed serialized brush half-spaces at a finite point
	/// in MODEL coordinates. No translation or rotation is applied here.
	///
	/// This is point membership, including boundaries, not hull clearance or a
	/// promise of native collision/float-rounding equivalence. Coefficients and
	/// coordinates are promoted to f64 with a fixed evaluation order.
	pub fn contains(&self, point: [f32; 3]) -> Result<bool, BrushError> {
		if !finite(point) {
			return Err(Error::from(Invalid::Point).into());
		}
		let point = point.map(f64::from);
		let mut work = Work::new(self.work);
		for brush in &self.brushes {
			work.spend(1)?;
			let mut inside = true;
			for plane in &brush.planes {
				work.spend(1)?;
				let normal = plane.normal.map(f64::from);
				let distance = ((normal[0] * point[0] + normal[1] * point[1])
					+ normal[2] * point[2])
					- f64::from(plane.distance);
				if distance > 0.0 {
					inside = false;
					break;
				}
			}
			if inside {
				return Ok(true);
			}
		}
		Ok(false)
	}
}

/// One source brush's copied plane constraints.
#[derive(Debug, Clone)]
pub struct RegionBrush {
	source: usize,
	contents: u32,
	planes: Vec<Plane>,
}

impl RegionBrush {
	pub fn source(&self) -> usize {
		self.source
	}

	pub fn contents(&self) -> u32 {
		self.contents
	}

	pub fn planes(&self) -> &[Plane] {
		&self.planes
	}
}

fn flags(length: usize, work: &mut Work) -> Result<Vec<bool>, Error> {
	work.spend(length)?;
	let mut flags = Vec::new();
	reserve(&mut flags, length, Resource::Records)?;
	flags.resize(length, false);
	Ok(flags)
}

fn count(length: usize, width: usize, lump: usize) -> Result<usize, Error> {
	if length % width != 0 {
		return Err(Invalid::RecordSize { lump }.into());
	}
	Ok(length / width)
}

fn checked_range(
	first: usize,
	length: usize,
	total: usize,
	lump: usize,
	record: usize,
) -> Result<Range<usize>, Error> {
	let end = first
		.checked_add(length)
		.filter(|&end| end <= total)
		.ok_or(Invalid::Reference { lump, record })?;
	Ok(first..end)
}

fn index_value(bytes: &[u8], at: usize, lump: usize, record: usize) -> Result<usize, Error> {
	usize::try_from(word(bytes, at) as i32).map_err(|_| Invalid::Reference { lump, record }.into())
}

fn short(bytes: &[u8], at: usize) -> u16 {
	u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn word(bytes: &[u8], at: usize) -> u32 {
	u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}
