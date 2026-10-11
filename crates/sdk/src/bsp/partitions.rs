//! Bounded world-tree queries and unfolded path constraints.

use super::{Bsp, Child, Coverage, Error, Invalid, Resource, Work, bound, finite, reserve};
use std::ops::Range;

/// Side of a splitting plane under the query's exact boundary convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
	/// Signed distance greater than or equal to zero, including negative zero.
	Front,
	/// Signed distance strictly below zero.
	Back,
}

impl Side {
	fn accepts(self, distance: f64) -> bool {
		match self {
			Self::Front => distance >= 0.0,
			Self::Back => distance < 0.0,
		}
	}
}

/// A half-space using a plane index in the BSP that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HalfSpace {
	/// Source plane index, meaningful only with the same BSP snapshot.
	pub plane: usize,
	/// Included side; the back side excludes points on the plane.
	pub side: Side,
}

/// One occurrence of a leaf in the unfolded world tree.
///
/// The path's constraints may be unbounded or even mutually inconsistent.
/// They are not a proof of an enclosed, nonempty, collision-free volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
	/// Source leaf index. Shared leaves can have several distinct partitions.
	pub leaf: usize,
	constraints: Range<usize>,
}

/// Owned, deterministic world-tree occurrences and their plane constraints.
///
/// Order is depth first, front before back. A shared node is expanded once for
/// each incoming path, under explicit output and work limits. Source indices
/// must be used with the same BSP snapshot; this is not a map-identity cache.
#[derive(Debug, Clone)]
pub struct WorldPartitions {
	partitions: Vec<Partition>,
	constraints: Vec<HalfSpace>,
}

impl WorldPartitions {
	/// All unfolded leaf occurrences, in deterministic traversal order.
	pub fn partitions(&self) -> &[Partition] {
		&self.partitions
	}

	/// The constraints for an occurrence, or None for an invalid index.
	pub fn constraints(&self, partition: usize) -> Option<&[HalfSpace]> {
		self.partitions
			.get(partition)
			.map(|partition| &self.constraints[partition.constraints.clone()])
	}

	/// These are structural constraints, not complete collision volumes.
	pub fn coverage(&self) -> Coverage {
		Coverage::PartitionConstraints
	}
}

impl Bsp {
	/// Locates a finite point in model zero, returning its source leaf index.
	///
	/// Coefficients and query coordinates are promoted from f32 to f64 before
	/// a fixed-order dot product. A signed distance of zero goes to the front.
	/// This avoids finite-f32 product overflow, but does not promise bitwise
	/// agreement with the engine near a plane. Bounds are not clipping planes;
	/// a distant point can reach a leaf without proving it is inside the map.
	pub fn locate(&self, point: [f32; 3]) -> Result<usize, Error> {
		if !finite(point) {
			return Err(Invalid::Point.into());
		}
		let mut work = Work::new(self.limits.work);
		let mut child = self.models[0].head;
		loop {
			work.spend(1)?;
			match child {
				Child::Leaf(leaf) => return Ok(leaf),

				Child::Node(node) => {
					let node = self.nodes[node];
					let distance = self.planes[node.plane].signed_distance(point);
					child = node.children[usize::from(distance < 0.0)];
				}
			}
		}
	}

	/// Tests all supplied constraints against a finite point.
	///
	/// Constraints are relative to this snapshot's plane array; their source
	/// map identity remains the caller's responsibility. All indices are
	/// checked even when an earlier half-space excludes the point.
	pub fn matches(&self, constraints: &[HalfSpace], point: [f32; 3]) -> Result<bool, Error> {
		if !finite(point) {
			return Err(Invalid::Point.into());
		}
		bound(
			constraints.len(),
			self.limits.constraints,
			Resource::Constraints,
		)?;
		let mut work = Work::new(self.limits.work);
		let mut included = true;
		for constraint in constraints {
			work.spend(1)?;
			let plane = self
				.planes
				.get(constraint.plane)
				.ok_or(Invalid::Constraint {
					plane: constraint.plane,
				})?;
			included &= constraint.side.accepts(plane.signed_distance(point));
		}
		Ok(included)
	}

	/// Unfolds the world tree into bounded owned path constraints.
	///
	/// No artificial bounding box is added. Solid leaves are retained, shared
	/// leaves remain separate occurrences, and no adjacency, emptiness or
	/// semantic room classification is inferred.
	pub fn world_partitions(&self) -> Result<WorldPartitions, Error> {
		let mut output = WorldPartitions {
			partitions: Vec::new(),
			constraints: Vec::new(),
		};
		let mut path = Vec::new();
		reserve(&mut path, self.world_depth, Resource::Constraints)?;
		let mut stack = Vec::new();
		let capacity = self
			.world_depth
			.checked_mul(3)
			.and_then(|value| value.checked_add(1))
			.ok_or(Error::Allocation {
				resource: Resource::Work,
			})?;
		reserve(&mut stack, capacity, Resource::Work)?;
		stack.push(Event::Enter(self.models[0].head, None));
		let mut work = Work::new(self.limits.work);

		while let Some(event) = stack.pop() {
			work.spend(1)?;
			match event {
				Event::Leave => {
					path.pop();
				}

				Event::Enter(child, constraint) => {
					if let Some(constraint) = constraint {
						path.push(constraint);
						stack.push(Event::Leave);
					}
					match child {
						Child::Node(node) => {
							let node = self.nodes[node];
							stack.push(Event::Enter(
								node.children[1],
								Some(HalfSpace {
									plane: node.plane,
									side: Side::Back,
								}),
							));
							stack.push(Event::Enter(
								node.children[0],
								Some(HalfSpace {
									plane: node.plane,
									side: Side::Front,
								}),
							));
						}

						Child::Leaf(leaf) => {
							grow(
								output.partitions.len(),
								1,
								self.limits.partitions,
								Resource::Partitions,
							)?;
							let start = output.constraints.len();
							let end = grow(
								start,
								path.len(),
								self.limits.constraints,
								Resource::Constraints,
							)?;
							work.spend(path.len())?;
							reserve(&mut output.partitions, 1, Resource::Partitions)?;
							reserve(&mut output.constraints, path.len(), Resource::Constraints)?;
							output.constraints.extend_from_slice(&path);
							output.partitions.push(Partition {
								leaf,
								constraints: start..end,
							});
						}
					}
				}
			}
		}
		Ok(output)
	}
}

enum Event {
	Enter(Child, Option<HalfSpace>),
	Leave,
}

fn grow(
	current: usize,
	additional: usize,
	limit: usize,
	resource: Resource,
) -> Result<usize, Error> {
	let next = current
		.checked_add(additional)
		.ok_or(Error::Budget { resource, limit })?;
	bound(next, limit, resource)?;
	Ok(next)
}
