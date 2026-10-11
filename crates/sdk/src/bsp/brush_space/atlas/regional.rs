//! One privately constructed two-region cover and exact positive-volume stitches.

mod tests;

#[cfg(test)]
mod measurements;

use super::super::{Budget, reserve};
use super::*;

/// One actual fragment-pair witness for a positive three-dimensional overlap.
/// IDs are scoped to this RegionalPair; array positions are [left, right].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentPairCertificate {
	components: [usize; 2],
	fragments: [usize; 2],
}

impl ComponentPairCertificate {
	pub fn components(&self) -> [usize; 2] {
		self.components
	}

	pub fn fragments(&self) -> [usize; 2] {
		self.fragments
	}
}

#[derive(Debug)]
struct Overlap {
	certificates: Vec<ComponentPairCertificate>,
	statistics: RegionalOverlapStatistics,
}

/// One fresh allowance for the entire overlap query, including discarded work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionalOverlapLimits {
	/// Plane imports, candidate storage, cell copies, clipping, lookup and sorting.
	pub geometry: Limits,
	/// All conservative candidates, including occurrence and witnessed-tuple skips.
	pub pairs: usize,
	/// Distinct retained (left component, right component) certificates.
	pub certificates: usize,
}

/// Query-only accounting; both atlas builds and source traversals remain separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionalOverlapStatistics {
	pub geometry: Statistics,
	pub left_planes: usize,
	pub right_planes: usize,
	pub candidate_pairs: usize,
	pub certificates: usize,
}

/// An owned, complete fixed two-region cover of one artificial target.
///
/// Both atlases are built privately from the same immutable BrushGeometry. No
/// caller-supplied atlas or source identity can be attached. All components and
/// their original domain-contact flags survive; all positive component pairs
/// have one deterministic witness, sorted lexicographically. Face, edge and
/// point contact cannot certify a stitch. Atlas point ownership is unchanged,
/// including equality-only Unrepresented points.
///
/// Construction performs at most two serial atlas builds and one overlap query,
/// without retries. Each build has its own geometry allowance. Its world-tree
/// unfolding and model-zero brush enumeration each use the input BSP's existing
/// independent source limits; SourceStatistics are checked output counts, not
/// traversal-work counters. Geometry Statistics exclude those traversals. There
/// is no caller-selected aggregate retention/work cap across these operations
/// or repeated calls, and no memory/time or recoverable BigInt-OOM guarantee.
///
/// The target and child boundaries are artificial. This proves bounded source
/// geometry overlap, not native collision parity, actor movement, gameplay roles,
/// whole-map completion or whole-world enclosure. A general cover manifest and
/// negative-connectivity API are separate work.
#[derive(Debug)]
pub struct RegionalPair {
	split: RegionalSplit,
	left: BrushSpace,
	right: BrushSpace,
	overlap: Overlap,
}

impl RegionalPair {
	pub fn build(
		geometry: &BrushGeometry,
		split: RegionalSplit,
		left_limits: Limits,
		right_limits: Limits,
		overlap_limits: RegionalOverlapLimits,
	) -> Result<Self, RegionalPairError> {
		// Revalidate the split against the target it owns before any atlas build.
		let [left_domain, right_domain] = split.domains()?;
		let left = BrushSpace::build(geometry, left_domain, left_limits)
			.map_err(RegionalPairError::LeftBuild)?;
		let right = BrushSpace::build(geometry, right_domain, right_limits)
			.map_err(RegionalPairError::RightBuild)?;
		let overlap =
			enumerate_overlap(&left, &right, overlap_limits).map_err(RegionalPairError::Overlap)?;
		Ok(Self {
			split,
			left,
			right,
			overlap,
		})
	}

	pub fn certificates(&self) -> &[ComponentPairCertificate] {
		&self.overlap.certificates
	}

	pub fn left(&self) -> &BrushSpace {
		&self.left
	}

	pub fn overlap_statistics(&self) -> RegionalOverlapStatistics {
		self.overlap.statistics
	}

	pub fn right(&self) -> &BrushSpace {
		&self.right
	}

	pub fn split(&self) -> RegionalSplit {
		self.split
	}

	pub fn target(&self) -> AnalysisDomain {
		self.split.target
	}
}

/// Failure returns no partial pair, atlas or certificate list.
#[derive(Debug, thiserror::Error)]
pub enum RegionalPairError {
	#[error("regional split must have a valid axis and finite min < a < seam < b < max encodings")]
	Split,
	#[error("left regional atlas construction failed: {0}")]
	LeftBuild(#[source] Error),
	#[error("right regional atlas construction failed: {0}")]
	RightBuild(#[source] Error),
	#[error("regional overlap query failed: {0}")]
	Overlap(#[source] Error),
}

/// An exact overlapping split of one finite open target box.
///
/// The encoded boundaries satisfy min < a < seam < b < max on the selected
/// axis. The left box ends at b and the right box starts at a; their open
/// union covers the target, and both stay inside it. No margin is rounded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionalSplit {
	target: AnalysisDomain,
	axis: usize,
	boundaries: [f32; 3],
}

impl RegionalSplit {
	pub fn new(
		target: AnalysisDomain,
		axis: usize,
		overlap_min: f32,
		seam: f32,
		overlap_max: f32,
	) -> Result<Self, RegionalPairError> {
		let split = Self {
			target,
			axis,
			boundaries: [overlap_min, seam, overlap_max],
		};
		split.domains()?;
		Ok(split)
	}

	pub fn axis(self) -> usize {
		self.axis
	}

	/// [overlap minimum, seam, overlap maximum], all original f32 encodings.
	pub fn boundaries(self) -> [f32; 3] {
		self.boundaries
	}

	fn domains(self) -> Result<[AnalysisDomain; 2], RegionalPairError> {
		if self.axis >= 3 || !self.boundaries.into_iter().all(f32::is_finite) {
			return Err(RegionalPairError::Split);
		}
		let [a, seam, b] = self.boundaries.map(widen);
		if !(widen(self.target.min[self.axis]) < a
			&& a < seam
			&& seam < b
			&& b < widen(self.target.max[self.axis]))
		{
			return Err(RegionalPairError::Split);
		}
		let mut left_max = self.target.max;
		left_max[self.axis] = self.boundaries[2];
		let mut right_min = self.target.min;
		right_min[self.axis] = self.boundaries[0];
		Ok([
			AnalysisDomain::new(self.target.min, left_max).map_err(|_| RegionalPairError::Split)?,
			AnalysisDomain::new(right_min, self.target.max)
				.map_err(|_| RegionalPairError::Split)?,
		])
	}

	pub fn target(self) -> AnalysisDomain {
		self.target
	}
}

/// Only the public builder can establish the same-source precondition.
/// Both atlases are immutable; any error discards all temporary certificates.
fn enumerate_overlap(
	left: &BrushSpace,
	right: &BrushSpace,
	limits: RegionalOverlapLimits,
) -> Result<Overlap, Error> {
	let mut kernel = Kernel::new(limits.geometry);
	kernel.budget.work(1)?;
	let right_offset = import_planes(&mut kernel, left, right)?;
	let mut certificates: Vec<ComponentPairCertificate> = Vec::new();
	let mut candidate_pairs = 0usize;
	let mut certificate_count = 0usize;
	for (left_id, left_fragment) in left.fragments.iter().enumerate() {
		kernel.budget.work(1)?;
		if left_fragment.cell.flat.is_some() || left_fragment.component >= left.components.len() {
			return Err(Error::Invariant);
		}
		let candidates = right
			.fragment_index
			.query(left_fragment.bounds, &mut kernel.budget)?;
		Budget::add(
			&mut candidate_pairs,
			candidates.len(),
			limits.pairs,
			"regional pairs",
		)?;
		for right_id in candidates {
			kernel.budget.work(1)?;
			let right_fragment = right.fragments.get(right_id).ok_or(Error::Invariant)?;
			if right_fragment.cell.flat.is_some()
				|| right_fragment.component >= right.components.len()
			{
				return Err(Error::Invariant);
			}
			// Build assigns this ordinal before any domain filtering, by the
			// same immutable front-before-back world-tree unfolding. Different
			// occurrences diverge at opposite halfspaces, even for shared DAGs.
			// Equality/leaf IDs and bounds can never establish positive volume.
			if left_fragment.partition != right_fragment.partition {
				continue;
			}
			let components = [left_fragment.component, right_fragment.component];
			let mut seen = false;
			for certificate in &certificates {
				kernel.budget.work(1)?;
				if certificate.components == components {
					seen = true;
					break;
				}
			}
			if seen {
				continue;
			}
			let mut current = Some(kernel.copy_cell(&left_fragment.cell)?);
			for &plane in &right_fragment.cell.constraints {
				kernel.budget.work(1)?;
				let Some(cell) = current.take() else {
					break;
				};
				if plane >= right.kernel.planes.len() {
					return Err(Error::Invariant);
				}
				let mapped = right_offset.checked_add(plane).ok_or(Error::Arithmetic)?;
				current = kernel.clip(cell, mapped)?;
			}
			let Some(cell) = current else {
				continue;
			};
			if cell.flat.is_some() || !kernel.full_dimension(&cell)? {
				return Err(Error::Invariant);
			}
			Budget::add(
				&mut certificate_count,
				1,
				limits.certificates,
				"regional certificates",
			)?;
			kernel.budget.constraints(4)?;
			reserve(&mut certificates, 1)?;
			certificates.push(ComponentPairCertificate {
				components,
				fragments: [left_id, right_id],
			});
		}
	}
	heap_sort(
		&mut certificates,
		|a, b| a.components.cmp(&b.components),
		&mut kernel.budget,
	)?;
	Ok(Overlap {
		statistics: RegionalOverlapStatistics {
			geometry: kernel.budget.stats,
			left_planes: left.kernel.planes.len(),
			right_planes: right.kernel.planes.len(),
			candidate_pairs,
			certificates: certificate_count,
		},
		certificates,
	})
}

/// Complete prefix import preserves every left support triple and denominator.
/// Right vertices are not copied; every right constraint uses the checked offset.
fn import_planes(
	kernel: &mut Kernel,
	left: &BrushSpace,
	right: &BrushSpace,
) -> Result<usize, Error> {
	if !kernel.planes.is_empty() {
		return Err(Error::Invariant);
	}
	let offset = left.kernel.planes.len();
	let total = offset
		.checked_add(right.kernel.planes.len())
		.ok_or(Error::Arithmetic)?;
	kernel.budget.constraints(total)?;
	reserve(&mut kernel.planes, total)?;
	kernel.planes.extend_from_slice(&left.kernel.planes);
	kernel.planes.extend_from_slice(&right.kernel.planes);
	Ok(offset)
}
