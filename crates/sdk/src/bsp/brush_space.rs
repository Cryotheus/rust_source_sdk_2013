//! Bounded convex brush-space geometry.
//!
//! Topology uses original f32 supporting planes and certified signs. Display
//! coordinates, BSP culling boxes and engine NAV data are not geometry inputs.

mod atlas;
mod bounds;
mod predicates;
mod tests;

use super::Plane;
use predicates::{Sign, det3, det4};

pub use atlas::{
	BrushSpace, Component, Coverage, Fragment, Interface, PointRelation, RegionComponentIncidence,
	RegionIncidence, RegionQueryError, RegionQueryLimits, RegionQueryStatistics, Solid,
	SolidSource, SourceStatistics,
};

pub use atlas::{
	ComponentPairCertificate, RegionalOverlapLimits, RegionalOverlapStatistics, RegionalPair,
	RegionalPairError, RegionalSplit,
};

/// One finite, explicitly artificial analysis domain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisDomain {
	min: [f32; 3],
	max: [f32; 3],
}

impl AnalysisDomain {
	pub fn new(min: [f32; 3], max: [f32; 3]) -> Result<Self, Error> {
		if !(0..3).all(|axis| {
			min[axis].is_finite()
				&& max[axis].is_finite()
				&& predicates::widen(min[axis]) < predicates::widen(max[axis])
		}) {
			return Err(Error::Domain);
		}
		Ok(Self { min, max })
	}

	pub fn max(self) -> [f32; 3] {
		self.max
	}

	pub fn min(self) -> [f32; 3] {
		self.min
	}
}

#[derive(Debug)]
pub(super) struct Budget {
	limits: Limits,
	stats: Statistics,
}

impl Budget {
	fn new(limits: Limits) -> Self {
		Self {
			limits,
			stats: Statistics::default(),
		}
	}

	fn add(
		current: &mut usize,
		count: usize,
		limit: usize,
		resource: &'static str,
	) -> Result<(), Error> {
		let next = current
			.checked_add(count)
			.filter(|&value| value <= limit)
			.ok_or(Error::Budget { resource, limit })?;
		*current = next;
		Ok(())
	}

	fn cell(&mut self) -> Result<(), Error> {
		self.work(1)?;
		Self::add(&mut self.stats.cells, 1, self.limits.cells, "cells")
	}

	fn constraints(&mut self, count: usize) -> Result<(), Error> {
		self.work(count)?;
		Self::add(
			&mut self.stats.constraints,
			count,
			self.limits.constraints,
			"constraints",
		)
	}

	pub(super) fn exact(&mut self, work: usize) -> Result<(), Error> {
		self.work(work)?;
		Self::add(
			&mut self.stats.exact_predicates,
			1,
			self.limits.exact_predicates,
			"exact predicates",
		)
	}

	pub(super) fn predicate(&mut self) -> Result<(), Error> {
		self.work(1)?;
		Self::add(
			&mut self.stats.predicates,
			1,
			self.limits.predicates,
			"predicates",
		)
	}

	fn vertices(&mut self, count: usize) -> Result<(), Error> {
		self.work(count)?;
		Self::add(
			&mut self.stats.vertices,
			count,
			self.limits.vertices,
			"vertices",
		)
	}

	fn work(&mut self, count: usize) -> Result<(), Error> {
		Self::add(&mut self.stats.work, count, self.limits.work, "work")
	}
}

/// Closed geometry; strict free-space membership is checked separately against solids.
#[derive(Debug)]
struct Cell {
	constraints: Vec<usize>,
	vertices: Vec<Vertex>,
	/// A planar cell has this exact affine hull. Otherwise its affine hull is R3.
	flat: Option<usize>,
}

/// Construction fails atomically; an incomplete graph is never a successful result.
#[derive(Debug, thiserror::Error)]
pub enum Error {
	#[error("analysis domain must have finite, strictly ordered bounds")]
	Domain,
	#[error("plane coefficients must be finite with a nonzero normal")]
	InvalidPlane,
	#[error("exact predicate arithmetic exceeded its qualified range")]
	Arithmetic,
	#[error("brush-space {resource} budget exceeded its limit of {limit}")]
	Budget {
		resource: &'static str,
		limit: usize,
	},
	#[error("brush-space allocation failed")]
	Allocation,
	#[error("convex geometry invariant failed")]
	Invariant,
	#[error("point coordinates must be finite")]
	Point,
	#[error(transparent)]
	Bsp(#[from] super::Error),
	#[error(transparent)]
	Brush(#[from] super::brushes::BrushError),
}

#[derive(Debug)]
struct Kernel {
	planes: Vec<Plane>,
	budget: Budget,
}

impl Kernel {
	fn new(limits: Limits) -> Self {
		Self {
			planes: Vec::new(),
			budget: Budget::new(limits),
		}
	}

	/// Intersects closed geometry with one halfspace. A lower-dimensional result
	/// is absent; callers handling strict subtraction inspect positive signs first.
	fn clip(&mut self, cell: Cell, plane: usize) -> Result<Option<Cell>, Error> {
		let mut signs = Vec::new();
		self.budget.vertices(cell.vertices.len())?;
		reserve(&mut signs, cell.vertices.len())?;
		let mut positive = false;
		let mut negative = false;
		for &vertex in &cell.vertices {
			let sign = self.side(vertex, plane)?;
			positive |= sign == Sign::Positive;
			negative |= sign == Sign::Negative;
			signs.push(sign);
		}
		if !positive {
			return Ok(Some(cell));
		}
		if !negative {
			return Ok(None);
		}
		self.budget.cell()?;
		let mut vertices = Vec::new();
		for (index, &vertex) in cell.vertices.iter().enumerate() {
			if signs[index] != Sign::Positive {
				self.push_vertex(&mut vertices, vertex)?;
			}
		}
		for left in 0..cell.vertices.len() {
			for right in left + 1..cell.vertices.len() {
				self.budget.work(1)?;
				if signs[left] == Sign::Zero
					|| signs[right] == Sign::Zero
					|| signs[left] == signs[right]
				{
					continue;
				}
				if let Some([a, b]) =
					self.edge_support(&cell, cell.vertices[left], cell.vertices[right])?
				{
					let vertex = self.vertex([a, b, plane])?;
					self.push_vertex(&mut vertices, vertex)?;
				}
			}
		}
		let mut constraints = self.copy_constraints(&cell.constraints)?;
		self.budget.constraints(1)?;
		reserve(&mut constraints, 1)?;
		constraints.push(plane);
		let clipped = Cell {
			constraints,
			vertices,
			flat: cell.flat,
		};
		if !self.full_dimension(&clipped)? {
			return Err(Error::Invariant);
		}
		Ok(Some(clipped))
	}

	fn copy_cell(&mut self, source: &Cell) -> Result<Cell, Error> {
		self.budget.cell()?;
		let constraints = self.copy_constraints(&source.constraints)?;
		self.budget.vertices(source.vertices.len())?;
		let mut vertices = Vec::new();
		reserve(&mut vertices, source.vertices.len())?;
		vertices.extend_from_slice(&source.vertices);
		Ok(Cell {
			constraints,
			vertices,
			flat: source.flat,
		})
	}

	fn copy_constraints(&mut self, source: &[usize]) -> Result<Vec<usize>, Error> {
		self.budget.constraints(source.len())?;
		let mut output = Vec::new();
		reserve(&mut output, source.len())?;
		output.extend_from_slice(source);
		Ok(output)
	}

	fn domain(&mut self, domain: AnalysisDomain) -> Result<Cell, Error> {
		self.budget.cell()?;
		let mut constraints = Vec::new();
		self.budget.constraints(6)?;
		reserve(&mut constraints, 6)?;
		for axis in 0..3 {
			let mut normal = [0.0; 3];
			normal[axis] = -1.0;
			constraints.push(self.plane(Plane {
				normal,
				distance: flip(domain.min[axis]),
			})?);
			normal[axis] = 1.0;
			constraints.push(self.plane(Plane {
				normal,
				distance: domain.max[axis],
			})?);
		}
		let mut vertices = Vec::new();
		self.budget.vertices(8)?;
		reserve(&mut vertices, 8)?;
		for x in 0..2 {
			for y in 0..2 {
				for z in 0..2 {
					vertices.push(self.vertex([
						constraints[x],
						constraints[2 + y],
						constraints[4 + z],
					])?);
				}
			}
		}
		Ok(Cell {
			constraints,
			vertices,
			flat: None,
		})
	}

	/// A common face pair with independent normals proves that two distinct
	/// vertices lie on an edge. Redundant and nonsimple supports are accepted.
	fn edge_support(
		&mut self,
		cell: &Cell,
		a: Vertex,
		b: Vertex,
	) -> Result<Option<[usize; 2]>, Error> {
		let mut first = cell.flat;
		for &plane in &cell.constraints {
			self.budget.work(1)?;
			if self.side(a, plane)? != Sign::Zero || self.side(b, plane)? != Sign::Zero {
				continue;
			}
			if let Some(previous) = first {
				if !self.parallel(previous, plane, false)? {
					return Ok(Some([previous, plane]));
				}
			} else {
				first = Some(plane);
			}
		}
		Ok(None)
	}

	/// Returns the positive-area facet, if this supporting plane defines one.
	fn facet(&mut self, cell: &Cell, plane: usize) -> Result<Option<Cell>, Error> {
		self.budget.cell()?;
		let constraints = self.copy_constraints(&cell.constraints)?;
		let mut vertices = Vec::new();
		for &vertex in &cell.vertices {
			if self.side(vertex, plane)? == Sign::Zero {
				self.push_vertex(&mut vertices, vertex)?;
			}
		}
		let face = Cell {
			constraints,
			vertices,
			flat: Some(plane),
		};
		Ok(self.full_dimension(&face)?.then_some(face))
	}

	/// Checks feasibility and positive interior in the designated affine hull.
	/// Strict witnesses for every non-affine constraint can be averaged to
	/// produce a jointly strict point, hence an open ball or planar disk.
	fn full_dimension(&mut self, cell: &Cell) -> Result<bool, Error> {
		if cell.vertices.is_empty() {
			return Ok(false);
		}
		if let Some(flat) = cell.flat {
			for &vertex in &cell.vertices {
				if self.side(vertex, flat)? != Sign::Zero {
					return Err(Error::Invariant);
				}
			}
		}
		let mut full = true;
		for &plane in &cell.constraints {
			let affine = if let Some(flat) = cell.flat {
				self.parallel(flat, plane, true)?
			} else {
				false
			};
			let mut strict = false;
			for &vertex in &cell.vertices {
				match self.side(vertex, plane)? {
					Sign::Positive => return Err(Error::Invariant),
					Sign::Negative => strict = true,
					Sign::Zero => (),
				}
			}
			full &= affine || strict;
		}
		Ok(full)
	}

	/// Geometric overlap of opposing facets; closed solids are removed afterward.
	fn interface(
		&mut self,
		left: &Cell,
		right: &Cell,
		left_plane: usize,
		right_plane: usize,
	) -> Result<Option<Cell>, Error> {
		if !self.parallel(left_plane, right_plane, true)? {
			return Ok(None);
		}
		let a = self.planes[left_plane].normal;
		let b = self.planes[right_plane].normal;
		let pivot = (0..3)
			.find(|&axis| a[axis].to_bits() & 0x7fff_ffff != 0)
			.ok_or(Error::Invariant)?;
		if a[pivot].is_sign_positive() == b[pivot].is_sign_positive() {
			return Ok(None);
		}
		let Some(mut polygon) = self.facet(left, left_plane)? else {
			return Ok(None);
		};
		for &plane in &right.constraints {
			let Some(clipped) = self.clip(polygon, plane)? else {
				return Ok(None);
			};
			polygon = clipped;
		}
		Ok(Some(polygon))
	}

	/// Products of two original f32 values are exactly representable in f64.
	fn parallel(
		&mut self,
		left: usize,
		right: usize,
		include_distance: bool,
	) -> Result<bool, Error> {
		self.budget.work(4)?;
		let a = coefficients(self.planes[left]).map(predicates::widen);
		let b = coefficients(self.planes[right]).map(predicates::widen);
		let pivot = (0..3)
			.find(|&axis| a[axis] != 0.0)
			.ok_or(Error::Invariant)?;
		let width = if include_distance { 4 } else { 3 };
		Ok((0..width).all(|axis| a[pivot] * b[axis] == b[pivot] * a[axis]))
	}

	fn plane(&mut self, plane: Plane) -> Result<usize, Error> {
		if !coefficients(plane).into_iter().all(f32::is_finite)
			|| plane
				.normal
				.into_iter()
				.all(|value| value.to_bits() & 0x7fff_ffff == 0)
		{
			return Err(Error::InvalidPlane);
		}
		self.budget.constraints(1)?;
		reserve(&mut self.planes, 1)?;
		let index = self.planes.len();
		self.planes.push(plane);
		Ok(index)
	}

	fn push_vertex(&mut self, vertices: &mut Vec<Vertex>, vertex: Vertex) -> Result<(), Error> {
		for &existing in vertices.iter() {
			if self.same_vertex(existing, vertex)? {
				return Ok(());
			}
		}
		self.budget.vertices(1)?;
		reserve(vertices, 1)?;
		vertices.push(vertex);
		Ok(())
	}

	fn reverse(&mut self, plane: usize) -> Result<usize, Error> {
		let source = self.planes[plane];
		self.plane(Plane {
			normal: source.normal.map(flip),
			distance: flip(source.distance),
		})
	}

	fn same_vertex(&mut self, left: Vertex, right: Vertex) -> Result<bool, Error> {
		for plane in left.support {
			if self.side(right, plane)? != Sign::Zero {
				return Ok(false);
			}
		}
		Ok(true)
	}

	fn side(&mut self, vertex: Vertex, plane: usize) -> Result<Sign, Error> {
		if vertex.support.contains(&plane) {
			self.budget.predicate()?;
			return Ok(Sign::Zero);
		}
		let rows = [
			vertex.support[0],
			vertex.support[1],
			vertex.support[2],
			plane,
		]
		.map(|index| coefficients(self.planes[index]));
		let sign = det4(rows, &mut self.budget)?;
		Ok(if vertex.denominator == Sign::Positive {
			negate(sign)
		} else {
			sign
		})
	}

	/// Subtracts a closed convex solid. Emitted cells are closures of strict
	/// outside pieces; solid surfaces must still be excluded in point queries.
	fn subtract(&mut self, cell: Cell, solid: &[usize]) -> Result<Vec<Cell>, Error> {
		let mut output = Vec::new();
		let mut remaining = Some(cell);
		for &plane in solid {
			self.budget.work(1)?;
			let Some(inside) = remaining.take() else {
				break;
			};
			let mut has_positive = false;
			for &vertex in &inside.vertices {
				has_positive |= self.side(vertex, plane)? == Sign::Positive;
			}
			if !has_positive {
				remaining = Some(inside);
				continue;
			}
			let copied = self.copy_cell(&inside)?;
			let reverse = self.reverse(plane)?;
			if let Some(outside) = self.clip(copied, reverse)? {
				reserve(&mut output, 1)?;
				output.push(outside);
			}
			remaining = self.clip(inside, plane)?;
		}
		Ok(output)
	}

	fn vertex(&mut self, support: [usize; 3]) -> Result<Vertex, Error> {
		let denominator = det3(
			support.map(|index| self.planes[index].normal),
			&mut self.budget,
		)?;
		if denominator == Sign::Zero {
			return Err(Error::Invariant);
		}
		Ok(Vertex {
			support,
			denominator,
		})
	}
}

/// Cumulative limits include discarded intermediate geometry and temporary copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
	pub work: usize,
	pub predicates: usize,
	pub exact_predicates: usize,
	pub cells: usize,
	pub vertices: usize,
	pub constraints: usize,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			work: 20_000_000,
			predicates: 5_000_000,
			exact_predicates: 100_000,
			cells: 100_000,
			vertices: 2_000_000,
			constraints: 2_000_000,
		}
	}
}

/// Geometry-phase work and cumulative output, excluding input-BSP traversal phases.
/// This is not an aggregate memory, time, decode or source-enumeration bound.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Statistics {
	pub work: usize,
	pub predicates: usize,
	pub exact_predicates: usize,
	pub cells: usize,
	pub vertices: usize,
	pub constraints: usize,
}

#[derive(Debug, Clone, Copy)]
struct Vertex {
	support: [usize; 3],
	denominator: Sign,
}

fn coefficients(plane: Plane) -> [f32; 4] {
	[
		plane.normal[0],
		plane.normal[1],
		plane.normal[2],
		plane.distance,
	]
}

fn flip(value: f32) -> f32 {
	f32::from_bits(value.to_bits() ^ 0x8000_0000)
}

fn negate(sign: Sign) -> Sign {
	match sign {
		Sign::Negative => Sign::Positive,
		Sign::Zero => Sign::Zero,
		Sign::Positive => Sign::Negative,
	}
}

fn reserve<T>(values: &mut Vec<T>, additional: usize) -> Result<(), Error> {
	values
		.try_reserve(additional)
		.map_err(|_| Error::Allocation)
}
