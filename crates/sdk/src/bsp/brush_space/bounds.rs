//! Conservative bounds from actual axial inequalities inside the analysis domain.
//!
//! Bounds only reject impossible intersections. Missing axial inequalities keep
//! the domain bound, and closed overlap retains zero-thickness candidates.

use super::{AnalysisDomain, Budget, Error, Plane, reserve};
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy)]
pub(super) struct Bounds {
	pub(super) min: [f64; 3],
	pub(super) max: [f64; 3],
}

impl Bounds {
	/// Division of widened original f32 values is rounded outward by one f64
	/// step. Nonaxial planes cannot tighten this deliberately conservative box.
	pub(super) fn from_planes(
		domain: AnalysisDomain,
		planes: impl IntoIterator<Item = Plane>,
		budget: &mut Budget,
	) -> Result<Option<Self>, Error> {
		budget.work(1)?;
		let mut bounds = Self {
			min: domain.min.map(super::predicates::widen),
			max: domain.max.map(super::predicates::widen),
		};
		for plane in planes {
			budget.work(1)?;
			let normals = plane.normal.map(super::predicates::widen);
			let distance = super::predicates::widen(plane.distance);
			if !distance.is_finite()
				|| !normals.into_iter().all(f64::is_finite)
				|| normals == [0.0; 3]
			{
				return Err(Error::InvalidPlane);
			}
			let mut axis = None;
			let mut axial = true;
			for (index, normal) in normals.into_iter().enumerate() {
				if normal != 0.0 {
					if axis.is_some() {
						axial = false;
						break;
					}
					axis = Some(index);
				}
			}
			if !axial {
				continue;
			}
			let axis = axis.ok_or(Error::Invariant)?;
			let normal = normals[axis];
			let coordinate = distance / normal;
			if normal > 0.0 {
				bounds.max[axis] = bounds.max[axis].min(coordinate.next_up());
			} else {
				bounds.min[axis] = bounds.min[axis].max(coordinate.next_down());
			}
		}
		Ok(if (0..3).any(|axis| bounds.min[axis] > bounds.max[axis]) {
			None
		} else {
			Some(bounds)
		})
	}

	fn valid(self) -> bool {
		(0..3).all(|axis| {
			self.min[axis].is_finite()
				&& self.max[axis].is_finite()
				&& self.min[axis] <= self.max[axis]
		})
	}

	pub(super) fn overlaps(self, other: Self) -> bool {
		(0..3).all(|axis| self.min[axis] <= other.max[axis] && other.min[axis] <= self.max[axis])
	}

	fn hull(self, other: Self) -> Self {
		Self {
			min: std::array::from_fn(|axis| self.min[axis].min(other.min[axis])),
			max: std::array::from_fn(|axis| self.max[axis].max(other.max[axis])),
		}
	}
}

#[derive(Debug, Clone, Copy)]
struct Node {
	id: usize,
	bounds: Bounds,
	hull: Bounds,
}

/// Median nodes are implicit in each sorted slice; hulls summarize that slice.
/// Recursion halves the slice and has at most usize::BITS nonempty frames.
/// Every retained node consumes one cell and one copied constraint reference.
/// Work additionally counts each plane/node visit, heapsort sift iteration,
/// heap extraction, and the linear duplicate pass. A sift has at most two
/// comparisons and one swap; these fixed-size operations allocate nothing.
/// Query output references consume the shared cumulative constraint budget.
#[derive(Debug, Default)]
pub(super) struct Index {
	nodes: Vec<Node>,
}

impl Index {
	pub(super) fn new(entries: Vec<(usize, Bounds)>, budget: &mut Budget) -> Result<Self, Error> {
		budget.constraints(entries.len())?;
		for (_, bounds) in &entries {
			budget.cell()?;
			if !bounds.valid() {
				return Err(Error::Invariant);
			}
		}
		let mut nodes = Vec::new();
		reserve(&mut nodes, entries.len())?;
		for (id, bounds) in entries {
			nodes.push(Node {
				id,
				bounds,
				hull: bounds,
			});
		}
		heap_sort(
			&mut nodes,
			|left, right| {
				left.bounds.min[0]
					.total_cmp(&right.bounds.min[0])
					.then_with(|| left.id.cmp(&right.id))
			},
			budget,
		)?;
		Self::build_hulls(&mut nodes, budget)?;
		Ok(Self { nodes })
	}

	fn build_hulls(nodes: &mut [Node], budget: &mut Budget) -> Result<Option<Bounds>, Error> {
		if nodes.is_empty() {
			return Ok(None);
		}
		budget.work(1)?;
		let middle = nodes.len() / 2;
		let (left, tail) = nodes.split_at_mut(middle);
		let (node, right) = tail.split_first_mut().ok_or(Error::Invariant)?;
		let mut hull = node.bounds;
		if let Some(bounds) = Self::build_hulls(left, budget)? {
			hull = hull.hull(bounds);
		}
		if let Some(bounds) = Self::build_hulls(right, budget)? {
			hull = hull.hull(bounds);
		}
		node.hull = hull;
		Ok(Some(hull))
	}

	/// IDs are ascending and unique. Face, edge, and point contacts all overlap.
	pub(super) fn query(&self, bounds: Bounds, budget: &mut Budget) -> Result<Vec<usize>, Error> {
		budget.work(1)?;
		if !bounds.valid() {
			return Err(Error::Invariant);
		}
		let mut output = Vec::new();
		Self::visit(&self.nodes, bounds, &mut output, budget)?;
		heap_sort(&mut output, usize::cmp, budget)?;
		budget.work(output.len())?;
		output.dedup();
		Ok(output)
	}

	fn visit(
		nodes: &[Node],
		bounds: Bounds,
		output: &mut Vec<usize>,
		budget: &mut Budget,
	) -> Result<(), Error> {
		if nodes.is_empty() {
			return Ok(());
		}
		budget.work(1)?;
		let middle = nodes.len() / 2;
		let node = nodes[middle];
		if !node.hull.overlaps(bounds) {
			return Ok(());
		}
		if node.bounds.overlaps(bounds) {
			budget.constraints(1)?;
			reserve(output, 1)?;
			output.push(node.id);
		}
		Self::visit(&nodes[..middle], bounds, output, budget)?;
		Self::visit(&nodes[middle + 1..], bounds, output, budget)
	}
}

/// Fallible, in-place heapsort makes all data-dependent sort work budgetable.
pub(super) fn heap_sort<T, F>(
	values: &mut [T],
	compare: F,
	budget: &mut Budget,
) -> Result<(), Error>
where
	F: Fn(&T, &T) -> Ordering,
{
	let length = values.len();
	for start in (0..length / 2).rev() {
		sift_down(values, start, length, &compare, budget)?;
	}
	for end in (1..length).rev() {
		budget.work(1)?;
		values.swap(0, end);
		sift_down(values, 0, end, &compare, budget)?;
	}
	Ok(())
}

fn sift_down<T, F>(
	values: &mut [T],
	mut root: usize,
	end: usize,
	compare: &F,
	budget: &mut Budget,
) -> Result<(), Error>
where
	F: Fn(&T, &T) -> Ordering,
{
	while root < end / 2 {
		budget.work(1)?;
		let mut child = root * 2 + 1;
		if child + 1 < end && compare(&values[child], &values[child + 1]) == Ordering::Less {
			child += 1;
		}
		if compare(&values[root], &values[child]) != Ordering::Less {
			break;
		}
		values.swap(root, child);
		root = child;
	}
	Ok(())
}
