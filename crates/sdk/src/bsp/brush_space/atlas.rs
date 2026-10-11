//! Owned bounded-domain source geometry; see BrushSpace for the coverage contract.

use super::super::{brushes::BrushGeometry, partitions::Side};
use super::bounds::{Bounds, Index, heap_sort};
use super::predicates::widen;
use super::{
	AnalysisDomain, Budget, Cell, Error, Kernel, Limits, Plane, Sign, Statistics, coefficients,
	det4, negate, reserve,
};
use sdk_raw::interfaces::engine_trace::CONTENTS_SOLID;

/// Provenance of one original closed set in the conservative solid union.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SolidSource {
	WorldBrush(usize),
	StructuralPartition { partition: usize, leaf: usize },
}

/// All original constraints survive, including after an intermediate rank drop.
#[derive(Debug)]
pub struct Solid {
	source: SolidSource,
	constraints: Vec<usize>,
}

impl Solid {
	pub fn source(&self) -> SolidSource {
		self.source
	}
	pub fn constraints(&self) -> &[usize] {
		&self.constraints
	}
}

#[derive(Debug)]
struct Source {
	partition: usize,
	leaf: usize,
	constraints: Vec<usize>,
	strict: Vec<bool>,
}

/// Closure of a full-dimensional free piece. Solid surfaces are excluded by locate.
#[derive(Debug)]
pub struct Fragment {
	source: usize,
	partition: usize,
	leaf: usize,
	cell: Cell,
	bounds: Bounds,
	component: usize,
	touches_domain: bool,
}

impl Fragment {
	pub fn partition(&self) -> usize {
		self.partition
	}
	pub fn leaf(&self) -> usize {
		self.leaf
	}
	pub fn constraints(&self) -> &[usize] {
		&self.cell.constraints
	}
	pub fn component(&self) -> usize {
		self.component
	}
	/// Includes face, edge and point contact with the artificial domain boundary.
	pub fn touches_domain(&self) -> bool {
		self.touches_domain
	}
}

/// Positive-area geometric aperture after subtraction of every relevant original solid.
/// This is neither an authored visibility areaportal nor a controller movement link.
/// Several convex pieces may connect the same pair of fragments. Stored polygons
/// are closures whose relative interiors exclude the original closed solid union.
/// Structural ownership is separate: BrushSpace::locate may report Unrepresented
/// even in that relative interior. Stored boundary points may also be solid.
#[derive(Debug)]
pub struct Interface {
	fragments: [usize; 2],
	cell: Cell,
}

impl Interface {
	pub fn fragments(&self) -> [usize; 2] {
		self.fragments
	}
	pub fn constraints(&self) -> &[usize] {
		&self.cell.constraints
	}
	pub fn support_plane(&self) -> usize {
		self.cell.flat.expect("interface has an affine plane")
	}
}

/// Connectivity of the represented open free volume inside the artificial domain.
#[derive(Debug)]
pub struct Component {
	fragments: Vec<usize>,
	touches_domain: bool,
}

impl Component {
	pub fn fragments(&self) -> &[usize] {
		&self.fragments
	}
	pub fn touches_domain(&self) -> bool {
		self.touches_domain
	}
}

#[derive(Debug, PartialEq, Eq)]
pub enum PointRelation {
	/// Includes the artificial boundary: connectivity is only claimed in its interior.
	OutsideDomain,
	/// In the original closed union, including structural solid occurrences.
	Solid,
	/// No retained full-dimensional nonsolid source occurrence owns this point.
	Unrepresented,
	/// All incident closures whose structural half-open occurrence owns the point.
	Free { fragments: Vec<usize> },
}

/// A complete result within one finite artificial domain and one explicit source model.
///
/// The solid model is the union of every CONTENTS_SOLID model-zero brush and the
/// closed path constraints of every CONTENTS_SOLID world partition occurrence.
/// Brush detail, empty intersections, sheets, lines, points and unbounded original
/// sets are retained uniformly. This is conservative source geometry, not native
/// collision parity: displacement surfaces, moving/submodel solids, props, actor
/// hulls, jump/climb capability and semantic roles are not resolved here.
///
/// Only full-dimensional nonsolid structural occurrences are represented. Their
/// identities survive shared BSP DAGs; queries retain front-inclusive/back-strict
/// ownership. Fragment closures are a storage representation: membership ALWAYS
/// excludes the original closed union. Interfaces additionally re-subtract that
/// union in their common plane. No culling bounds, largest component or NAV data
/// imply enclosure, playability or actor reachability. Unrepresented points remain
/// explicit, and a domain-touching component cannot establish whole-world enclosure.
///
/// Predicates widen f32 encodings using integer bits, then use a certified f64
/// filter and bounded exact BigInt fallback. All four IEEE rounding modes and
/// DAZ/FTZ are supported without changing host FP state. The rounding mode must
/// remain fixed during a build/query (canonical candidate keys use division).
/// BigInt internals have bounded operand sizes but do not expose fallible allocation;
/// owned vectors use fallible reservation. Partition unfolding and model-brush
/// enumeration are separate phases under the input BSP's work/output limits.
/// Geometry limits cumulatively charge owned copies, indices, clipping, subtraction,
/// interfaces and components; Statistics excludes the source traversal work.
/// SourceStatistics reports their checked output counts. Neither ledger is an
/// aggregate RSS/time bound. Either phase fails atomically with its typed error.
#[derive(Debug)]
pub struct BrushSpace {
	domain: AnalysisDomain,
	source_statistics: SourceStatistics,
	kernel: Kernel,
	solids: Vec<Solid>,
	solid_index: Index,
	sources: Vec<Source>,
	fragments: Vec<Fragment>,
	fragment_index: Index,
	interfaces: Vec<Interface>,
	components: Vec<Component>,
}

impl BrushSpace {
	pub fn build(
		geometry: &BrushGeometry,
		domain: AnalysisDomain,
		limits: Limits,
	) -> Result<Self, Error> {
		// Preflight geometry limits before the separately bounded source phases.
		let mut kernel = Kernel::new(limits);
		let domain_cell = kernel.domain(domain)?;
		let bsp = geometry.bsp();
		let partitions = bsp.world_partitions()?;
		let brushes = geometry.model_brushes(0)?;
		let source_statistics = SourceStatistics {
			partitions: partitions.partitions().len(),
			world_brushes: brushes.len(),
		};
		let mut plane_ids = Vec::new();
		kernel.budget.constraints(bsp.planes().len())?;
		reserve(&mut plane_ids, bsp.planes().len())?;
		for &plane in bsp.planes() {
			plane_ids.push(kernel.plane(plane)?);
		}
		let mut reverses = Vec::new();
		kernel.budget.constraints(plane_ids.len())?;
		reserve(&mut reverses, plane_ids.len())?;
		reverses.resize(plane_ids.len(), None);
		let mut solids = Vec::new();
		for brush_id in brushes {
			kernel.budget.work(1)?;
			let brush = &geometry.brushes()[brush_id];
			if brush.contents & CONTENTS_SOLID == 0 {
				continue;
			}
			let mut constraints = Vec::new();
			kernel.budget.constraints(brush.sides.len())?;
			reserve(&mut constraints, brush.sides.len())?;
			for side in &geometry.sides()[brush.sides.clone()] {
				constraints.push(plane_ids[side.plane]);
			}
			push_solid(
				&mut kernel,
				&mut solids,
				SolidSource::WorldBrush(brush_id),
				constraints,
			)?;
		}
		let mut sources = Vec::new();
		for (partition, occurrence) in partitions.partitions().iter().enumerate() {
			kernel.budget.work(1)?;
			let path = partitions.constraints(partition).ok_or(Error::Invariant)?;
			let mut constraints = Vec::new();
			let mut strict = Vec::new();
			kernel
				.budget
				.constraints(path.len().checked_mul(2).ok_or(Error::Arithmetic)?)?;
			reserve(&mut constraints, path.len())?;
			reserve(&mut strict, path.len())?;
			for half in path {
				kernel.budget.work(1)?;
				let plane = match half.side {
					Side::Back => plane_ids[half.plane],

					Side::Front => {
						if let Some(plane) = reverses[half.plane] {
							plane
						} else {
							let plane = kernel.reverse(plane_ids[half.plane])?;
							reverses[half.plane] = Some(plane);
							plane
						}
					}
				};
				constraints.push(plane);
				strict.push(half.side == Side::Back);
			}
			if bsp.leaves()[occurrence.leaf].contents & CONTENTS_SOLID != 0 {
				push_solid(
					&mut kernel,
					&mut solids,
					SolidSource::StructuralPartition {
						partition,
						leaf: occurrence.leaf,
					},
					constraints,
				)?;
			} else {
				kernel.budget.cell()?;
				reserve(&mut sources, 1)?;
				sources.push(Source {
					partition,
					leaf: occurrence.leaf,
					constraints,
					strict,
				});
			}
		}
		finish(
			kernel,
			domain,
			domain_cell,
			solids,
			sources,
			source_statistics,
		)
	}

	pub fn domain(&self) -> AnalysisDomain {
		self.domain
	}
	/// Owned original and reversed support planes; constraint indices refer here.
	pub fn planes(&self) -> &[Plane] {
		&self.kernel.planes
	}
	pub fn solids(&self) -> &[Solid] {
		&self.solids
	}
	pub fn fragments(&self) -> &[Fragment] {
		&self.fragments
	}
	pub fn interfaces(&self) -> &[Interface] {
		&self.interfaces
	}
	pub fn components(&self) -> &[Component] {
		&self.components
	}
	pub fn statistics(&self) -> Statistics {
		self.kernel.budget.stats
	}

	/// Exact finite-f32 point membership, with a fresh copy of the build work limits.
	/// Failure is atomic and does not change the atlas or its construction statistics.
	pub fn locate(&self, point: [f32; 3]) -> Result<PointRelation, Error> {
		if !point.into_iter().all(f32::is_finite) {
			return Err(Error::Point);
		}
		let widened = point.map(widen);
		if (0..3).any(|axis| {
			widened[axis] <= widen(self.domain.min[axis])
				|| widened[axis] >= widen(self.domain.max[axis])
		}) {
			return Ok(PointRelation::OutsideDomain);
		}
		let mut budget = Budget::new(self.kernel.budget.limits);
		let bounds = Bounds {
			min: widened,
			max: widened,
		};
		for solid in self.solid_index.query(bounds, &mut budget)? {
			let mut inside = true;
			for &plane in &self.solids[solid].constraints {
				if point_side(point, self.kernel.planes[plane], &mut budget)? == Sign::Positive {
					inside = false;
					break;
				}
			}
			if inside {
				return Ok(PointRelation::Solid);
			}
		}
		let mut fragments = Vec::new();
		for id in self.fragment_index.query(bounds, &mut budget)? {
			let fragment = &self.fragments[id];
			let source = &self.sources[fragment.source];
			let mut inside = true;
			for (&plane, &strict) in source.constraints.iter().zip(&source.strict) {
				let sign = point_side(point, self.kernel.planes[plane], &mut budget)?;
				if sign == Sign::Positive || (strict && sign == Sign::Zero) {
					inside = false;
					break;
				}
			}
			if !inside {
				continue;
			}
			for &plane in &fragment.cell.constraints {
				if point_side(point, self.kernel.planes[plane], &mut budget)? == Sign::Positive {
					inside = false;
					break;
				}
			}
			if inside {
				budget.constraints(1)?;
				reserve(&mut fragments, 1)?;
				fragments.push(id);
			}
		}
		Ok(if fragments.is_empty() {
			PointRelation::Unrepresented
		} else {
			PointRelation::Free { fragments }
		})
	}
}

fn point_side(point: [f32; 3], plane: Plane, budget: &mut Budget) -> Result<Sign, Error> {
	Ok(negate(det4(
		[
			[1.0, 0.0, 0.0, point[0]],
			[0.0, 1.0, 0.0, point[1]],
			[0.0, 0.0, 1.0, point[2]],
			coefficients(plane),
		],
		budget,
	)?))
}

fn push_solid(
	kernel: &mut Kernel,
	solids: &mut Vec<Solid>,
	source: SolidSource,
	constraints: Vec<usize>,
) -> Result<(), Error> {
	kernel.budget.cell()?;
	reserve(solids, 1)?;
	solids.push(Solid {
		source,
		constraints,
	});
	Ok(())
}

fn cell_bounds(kernel: &mut Kernel, domain: AnalysisDomain, cell: &Cell) -> Result<Bounds, Error> {
	Bounds::from_planes(
		domain,
		cell.constraints.iter().map(|&plane| kernel.planes[plane]),
		&mut kernel.budget,
	)?
	.ok_or(Error::Invariant)
}

fn entry(
	kernel: &mut Kernel,
	entries: &mut Vec<(usize, Bounds)>,
	id: usize,
	bounds: Bounds,
) -> Result<(), Error> {
	kernel.budget.constraints(1)?;
	reserve(entries, 1)?;
	entries.push((id, bounds));
	Ok(())
}

/// Bounds only reject impossible overlap. Every admitted original set is subtracted
/// in full; a dropped 3D interior never removes the original set from this index.
fn subtract_solids(
	kernel: &mut Kernel,
	domain: AnalysisDomain,
	solids: &[Solid],
	index: &Index,
	cell: Cell,
) -> Result<Vec<Cell>, Error> {
	let bounds = cell_bounds(kernel, domain, &cell)?;
	let candidates = index.query(bounds, &mut kernel.budget)?;
	let mut pieces = Vec::new();
	reserve(&mut pieces, 1)?;
	pieces.push(cell);
	for id in candidates {
		let mut next = Vec::new();
		for piece in pieces {
			kernel.budget.work(1)?;
			let outside = kernel.subtract(piece, &solids[id].constraints)?;
			reserve(&mut next, outside.len())?;
			next.extend(outside);
		}
		pieces = next;
		if pieces.is_empty() {
			break;
		}
	}
	Ok(pieces)
}

fn finish(
	mut kernel: Kernel,
	domain: AnalysisDomain,
	domain_cell: Cell,
	solids: Vec<Solid>,
	sources: Vec<Source>,
	source_statistics: SourceStatistics,
) -> Result<BrushSpace, Error> {
	let mut entries = Vec::new();
	for (id, solid) in solids.iter().enumerate() {
		if let Some(bounds) = Bounds::from_planes(
			domain,
			solid.constraints.iter().map(|&plane| kernel.planes[plane]),
			&mut kernel.budget,
		)? {
			entry(&mut kernel, &mut entries, id, bounds)?;
		}
	}
	let solid_index = Index::new(entries, &mut kernel.budget)?;
	let mut fragments = Vec::new();
	let mut fragment_entries = Vec::new();
	for (source_id, source) in sources.iter().enumerate() {
		if Bounds::from_planes(
			domain,
			source.constraints.iter().map(|&plane| kernel.planes[plane]),
			&mut kernel.budget,
		)?
		.is_none()
		{
			continue;
		}
		let mut current = Some(kernel.copy_cell(&domain_cell)?);
		for &plane in &source.constraints {
			let Some(cell) = current.take() else {
				break;
			};
			current = kernel.clip(cell, plane)?;
		}
		let Some(cell) = current else {
			continue;
		};
		for cell in subtract_solids(&mut kernel, domain, &solids, &solid_index, cell)? {
			let bounds = cell_bounds(&mut kernel, domain, &cell)?;
			let mut touches_domain = false;
			for &plane in &domain_cell.constraints {
				for &vertex in &cell.vertices {
					touches_domain |= kernel.side(vertex, plane)? == Sign::Zero;
				}
			}
			kernel.budget.cell()?;
			reserve(&mut fragments, 1)?;
			entry(&mut kernel, &mut fragment_entries, fragments.len(), bounds)?;
			fragments.push(Fragment {
				source: source_id,
				partition: source.partition,
				leaf: source.leaf,
				cell,
				bounds,
				component: 0,
				touches_domain,
			});
		}
	}
	let fragment_index = Index::new(fragment_entries, &mut kernel.budget)?;
	let interfaces = interfaces(&mut kernel, domain, &solids, &solid_index, &fragments)?;
	let components = components(&mut kernel, &mut fragments, &interfaces)?;
	Ok(BrushSpace {
		domain,
		source_statistics,
		kernel,
		solids,
		solid_index,
		sources,
		fragments,
		fragment_index,
		interfaces,
		components,
	})
}

#[derive(Clone, Copy)]
struct Facet {
	key: [u64; 4],
	positive: bool,
	fragment: usize,
	plane: usize,
}

/// Equal exact planes have equal correctly rounded ratios under a fixed rounding
/// mode. Hash/key collisions are harmless: exact proportionality is checked later.
/// All f32 ratios fit in the normal f64 range; signed zero is canonicalized.
fn facet_key(plane: Plane) -> Result<([u64; 4], bool), Error> {
	let values = coefficients(plane).map(widen);
	let pivot = (0..3)
		.find(|&axis| values[axis] != 0.0)
		.ok_or(Error::Invariant)?;
	let divisor = values[pivot];
	Ok((
		values.map(|value| {
			if value == 0.0 {
				0
			} else {
				(value / divisor).to_bits()
			}
		}),
		divisor > 0.0,
	))
}

fn interfaces(
	kernel: &mut Kernel,
	domain: AnalysisDomain,
	solids: &[Solid],
	index: &Index,
	fragments: &[Fragment],
) -> Result<Vec<Interface>, Error> {
	let mut facets = Vec::new();
	for (fragment, piece) in fragments.iter().enumerate() {
		for &plane in &piece.cell.constraints {
			kernel.budget.constraints(1)?;
			let (key, positive) = facet_key(kernel.planes[plane])?;
			reserve(&mut facets, 1)?;
			facets.push(Facet {
				key,
				positive,
				fragment,
				plane,
			});
		}
	}
	heap_sort(
		&mut facets,
		|left, right| {
			left.key
				.cmp(&right.key)
				.then(left.positive.cmp(&right.positive))
				.then(left.fragment.cmp(&right.fragment))
				.then(left.plane.cmp(&right.plane))
		},
		&mut kernel.budget,
	)?;
	let mut output = Vec::new();
	let mut start = 0;
	while start < facets.len() {
		let mut end = start + 1;
		while end < facets.len() && facets[end].key == facets[start].key {
			kernel.budget.work(1)?;
			end += 1;
		}
		let mut middle = start;
		while middle < end && !facets[middle].positive {
			kernel.budget.work(1)?;
			middle += 1;
		}
		if start < middle && middle < end {
			let mut entries = Vec::new();
			for (id, facet) in facets.iter().enumerate().take(middle).skip(start) {
				entry(kernel, &mut entries, id, fragments[facet.fragment].bounds)?;
			}
			let group = Index::new(entries, &mut kernel.budget)?;
			for right in &facets[middle..end] {
				for left in group.query(fragments[right.fragment].bounds, &mut kernel.budget)? {
					kernel.budget.work(1)?;
					let left = facets[left];
					if left.fragment == right.fragment {
						continue;
					}
					let Some(polygon) = kernel.interface(
						&fragments[left.fragment].cell,
						&fragments[right.fragment].cell,
						left.plane,
						right.plane,
					)?
					else {
						continue;
					};
					for cell in subtract_solids(kernel, domain, solids, index, polygon)? {
						kernel.budget.cell()?;
						reserve(&mut output, 1)?;
						output.push(Interface {
							fragments: [left.fragment, right.fragment],
							cell,
						});
					}
				}
			}
		}
		start = end;
	}
	Ok(output)
}

fn root(parents: &mut [usize], mut id: usize, budget: &mut Budget) -> Result<usize, Error> {
	while parents[id] != id {
		budget.work(1)?;
		parents[id] = parents[parents[id]];
		id = parents[id];
	}
	Ok(id)
}

fn components(
	kernel: &mut Kernel,
	fragments: &mut [Fragment],
	interfaces: &[Interface],
) -> Result<Vec<Component>, Error> {
	let mut parents = Vec::new();
	let mut ids = Vec::new();
	kernel
		.budget
		.constraints(fragments.len().checked_mul(2).ok_or(Error::Arithmetic)?)?;
	reserve(&mut parents, fragments.len())?;
	reserve(&mut ids, fragments.len())?;
	parents.extend(0..fragments.len());
	ids.resize(fragments.len(), usize::MAX);
	for interface in interfaces {
		kernel.budget.work(1)?;
		let left = root(&mut parents, interface.fragments[0], &mut kernel.budget)?;
		let right = root(&mut parents, interface.fragments[1], &mut kernel.budget)?;
		parents[left.max(right)] = left.min(right);
	}
	let mut output = Vec::new();
	for (id, fragment) in fragments.iter_mut().enumerate() {
		let representative = root(&mut parents, id, &mut kernel.budget)?;
		if ids[representative] == usize::MAX {
			kernel.budget.cell()?;
			reserve(&mut output, 1)?;
			ids[representative] = output.len();
			output.push(Component {
				fragments: Vec::new(),
				touches_domain: false,
			});
		}
		let component = ids[representative];
		fragment.component = component;
		kernel.budget.constraints(1)?;
		reserve(&mut output[component].fragments, 1)?;
		output[component].fragments.push(id);
		output[component].touches_domain |= fragment.touches_domain;
	}
	Ok(output)
}

#[cfg(test)]
pub(super) fn test_space(
	domain: AnalysisDomain,
	brushes: Vec<Vec<Plane>>,
	limits: Limits,
) -> Result<BrushSpace, Error> {
	let mut kernel = Kernel::new(limits);
	let cell = kernel.domain(domain)?;
	let mut solids = Vec::new();
	let source_statistics = SourceStatistics {
		partitions: 1,
		world_brushes: brushes.len(),
	};
	for (id, brush) in brushes.into_iter().enumerate() {
		let mut constraints = Vec::new();
		kernel.budget.constraints(brush.len())?;
		reserve(&mut constraints, brush.len())?;
		for plane in brush {
			constraints.push(kernel.plane(plane)?);
		}
		push_solid(
			&mut kernel,
			&mut solids,
			SolidSource::WorldBrush(id),
			constraints,
		)?;
	}
	let mut sources = Vec::new();
	kernel.budget.cell()?;
	reserve(&mut sources, 1)?;
	sources.push(Source {
		partition: 0,
		leaf: 0,
		constraints: Vec::new(),
		strict: Vec::new(),
	});
	finish(kernel, domain, cell, solids, sources, source_statistics)
}

/// Scope of a successfully completed atlas, independent of gameplay role evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
	/// Explicit artificial domain; nonsolid structural volume minus the closed
	/// union of world solid brushes and solid structural occurrences. Missing
	/// displacement, prop, submodel and controller geometry remains unresolved.
	BoundedSourceSolids,
}

impl BrushSpace {
	pub fn coverage(&self) -> Coverage {
		Coverage::BoundedSourceSolids
	}
}

/// Checked outputs of the two separately bounded input-BSP traversal phases.
/// These are counts, not estimates of their work or allocation totals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceStatistics {
	pub partitions: usize,
	pub world_brushes: usize,
}
impl BrushSpace {
	pub fn source_statistics(&self) -> SourceStatistics {
		self.source_statistics
	}
}
