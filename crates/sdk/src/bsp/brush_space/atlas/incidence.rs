//! Exact positive-volume queries for owned authored model regions.

mod tests;

use super::super::{Budget, reserve};
use super::*;
use crate::bsp::brushes::ModelRegion;
use num_bigint::{BigInt, Sign as IntegerSign};

const MAX_TRANSLATION_BITS: u64 = 556;
const TRANSLATION_WORK: usize = 64;

struct ImportedBrush {
	source: usize,
	constraints: Vec<usize>,
	bounds: Option<Bounds>,
}

/// A positive-volume pair, not a point sample or an actor movement certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionComponentIncidence {
	component: usize,
	fragment: usize,
	source_region_brush: usize,
}

impl RegionComponentIncidence {
	pub fn component(&self) -> usize {
		self.component
	}

	pub fn fragment(&self) -> usize {
		self.fragment
	}

	/// Serialized source brush ID in the supplied region; not a native engine ID.
	pub fn source_region_brush(&self) -> usize {
		self.source_region_brush
	}
}

/// Owned ascending unique component incidences in one queried atlas.
#[derive(Debug)]
pub struct RegionIncidence {
	incidences: Vec<RegionComponentIncidence>,
	statistics: RegionQueryStatistics,
}

impl RegionIncidence {
	pub fn incidences(&self) -> &[RegionComponentIncidence] {
		&self.incidences
	}

	pub fn statistics(&self) -> RegionQueryStatistics {
		self.statistics
	}
}

#[derive(Debug, thiserror::Error)]
pub enum RegionQueryError {
	#[error(
		"source region brush {source_region_brush} plane {plane} has no exact finite binary32 translated distance"
	)]
	UnsupportedExactTranslation {
		source_region_brush: usize,
		plane: usize,
	},
	#[error(transparent)]
	Geometry(#[from] Error),
}

/// One whole-query allowance; it is never reset per brush or fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionQueryLimits {
	/// Includes plane imports, translation, bounds, copies, clipping and sorting.
	pub geometry: Limits,
	/// All conservative fragment candidates, including already witnessed components.
	pub pairs: usize,
	/// Retained distinct component records.
	pub incidences: usize,
}

/// Query-only work; source acquisition and immutable atlas build counts are separate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegionQueryStatistics {
	pub geometry: Statistics,
	pub region_brushes: usize,
	pub translated_planes: usize,
	pub candidate_pairs: usize,
	pub incidences: usize,
}

impl BrushSpace {
	/// Finds every component with positive three-dimensional region intersection.
	///
	/// Each local plane n.q <= d is translated exactly to n.x <= d + n.origin.
	/// Only exactly representable finite-f32 distances are supported. Every plane
	/// is qualified before a result, even when an earlier bound/intersection is
	/// empty. ModelRegion::stored_origin is metadata and is never applied here.
	/// All region brushes and side constraints participate regardless of contents.
	///
	/// Fragment interiors already exclude the original closed source-solid union.
	/// Exact full-dimensional clipping therefore proves incidence; face, edge and
	/// point contacts do not. Empty output only means no positive volume within
	/// this artificial domain, not that the supplied authored region is absent.
	/// Component connectivity, point ownership and the atlas remain unchanged.
	///
	/// This mathematical query does not prove that the supplied region came from
	/// the atlas's BSP. Witness IDs refer to these two immutable inputs; callers
	/// projecting authored evidence must establish their own private source pairing.
	///
	/// A fresh metered kernel copies the complete atlas plane prefix and every
	/// candidate's constraints/vertex supports. Limits cover the whole query and
	/// failures return no partial list. Owned vectors reserve fallibly; num-bigint
	/// internally allocates infallibly with a separate proved 556-bit translation
	/// bound. Neither this ledger nor its statistics is an RSS/time guarantee.
	/// Translated coefficients re-enter the existing finite-f32 predicate proof;
	/// all four IEEE rounding modes and DAZ/FTZ remain supported, with rounding
	/// mode fixed during the query. Translation itself uses integer bits only.
	pub fn region_incidence(
		&self,
		region: &ModelRegion,
		origin: [f32; 3],
		limits: RegionQueryLimits,
	) -> Result<RegionIncidence, RegionQueryError> {
		let mut kernel = Kernel::new(limits.geometry);
		kernel.budget.work(1)?;
		if !origin.into_iter().all(f32::is_finite) {
			return Err(Error::Point.into());
		}
		// The identical prefix preserves EVERY constraint, support triple and
		// affine-plane index; there is no selective or rounded-vertex remapping.
		kernel.budget.constraints(self.kernel.planes.len())?;
		reserve(&mut kernel.planes, self.kernel.planes.len())?;
		kernel.planes.extend_from_slice(&self.kernel.planes);
		let mut imported = Vec::new();
		let mut translated_planes = 0usize;
		for brush in region.brushes() {
			kernel.budget.cell()?;
			reserve(&mut imported, 1)?;
			let mut constraints = Vec::new();
			kernel.budget.constraints(brush.planes().len())?;
			reserve(&mut constraints, brush.planes().len())?;
			for (side, &plane) in brush.planes().iter().enumerate() {
				let Some(distance) = translated_distance(plane, origin, &mut kernel.budget)? else {
					return Err(RegionQueryError::UnsupportedExactTranslation {
						source_region_brush: brush.source(),
						plane: side,
					});
				};
				constraints.push(kernel.plane(Plane {
					normal: plane.normal,
					distance,
				})?);
				translated_planes = translated_planes.checked_add(1).ok_or(Error::Arithmetic)?;
			}
			let bounds = Bounds::from_planes(
				self.domain,
				constraints.iter().map(|&id| kernel.planes[id]),
				&mut kernel.budget,
			)?;
			imported.push(ImportedBrush {
				source: brush.source(),
				constraints,
				bounds,
			});
		}
		let mut seen = Vec::new();
		kernel.budget.constraints(self.components.len())?;
		reserve(&mut seen, self.components.len())?;
		seen.resize(self.components.len(), false);
		let mut incidences = Vec::new();
		let mut candidate_pairs = 0usize;
		let mut incidence_count = 0usize;
		for brush in imported {
			kernel.budget.work(1)?;
			let Some(bounds) = brush.bounds else {
				continue;
			};
			let candidates = self.fragment_index.query(bounds, &mut kernel.budget)?;
			Budget::add(
				&mut candidate_pairs,
				candidates.len(),
				limits.pairs,
				"region pairs",
			)?;
			for id in candidates {
				kernel.budget.work(1)?;
				let fragment = self.fragments.get(id).ok_or(Error::Invariant)?;
				if *seen.get(fragment.component).ok_or(Error::Invariant)? {
					continue;
				}
				if fragment.cell.flat.is_some() {
					return Err(Error::Invariant.into());
				}
				let mut current = Some(kernel.copy_cell(&fragment.cell)?);
				for &plane in &brush.constraints {
					let Some(cell) = current.take() else {
						break;
					};
					current = kernel.clip(cell, plane)?;
				}
				let Some(cell) = current else {
					continue;
				};
				if !kernel.full_dimension(&cell)? {
					return Err(Error::Invariant.into());
				}
				Budget::add(
					&mut incidence_count,
					1,
					limits.incidences,
					"region incidences",
				)?;
				kernel.budget.constraints(3)?;
				reserve(&mut incidences, 1)?;
				incidences.push(RegionComponentIncidence {
					component: fragment.component,
					fragment: id,
					source_region_brush: brush.source,
				});
				seen[fragment.component] = true;
			}
		}
		heap_sort(
			&mut incidences,
			|left, right| left.component.cmp(&right.component),
			&mut kernel.budget,
		)?;
		Ok(RegionIncidence {
			statistics: RegionQueryStatistics {
				geometry: kernel.budget.stats,
				region_brushes: region.brushes().len(),
				translated_planes,
				candidate_pairs,
				incidences: incidence_count,
			},
			incidences,
		})
	}
}

fn parts(value: f32) -> (i64, i32) {
	let bits = value.to_bits();
	let exponent = ((bits >> 23) & 0xff) as i32;
	let fraction = i64::from(bits & 0x7f_ffff);
	let (mantissa, exponent) = if exponent == 0 {
		(fraction, -149)
	} else {
		(fraction | 0x80_0000, exponent - 150)
	};
	(if bits >> 31 == 0 { mantissa } else { -mantissa }, exponent)
}

/// S * 2^-298 is the exact translated distance. Each input mantissa has at most
/// 24 bits; products fit i64 and shifts are 0..506, hence at most 554-bit terms.
/// The distance has <=426 bits; three products plus distance stay below 2^556.
/// This proof is separate from the determinant bound: only an exactly encoded
/// finite f32 result is passed to that kernel.
fn translated_distance(
	plane: Plane,
	origin: [f32; 3],
	budget: &mut Budget,
) -> Result<Option<f32>, Error> {
	// Fixed precharge before any BigInt allocation: seven bit decodes, three
	// i64 products, <=12 explicit integer construction/update calls, bounded
	// checks and <=18 u32 limbs for trailing-zero inspection. This is neither
	// an exact determinant fallback nor an allocator-call/time counter.
	budget.work(TRANSLATION_WORK)?;
	if !coefficients(plane).into_iter().all(f32::is_finite) {
		return Err(Error::InvalidPlane);
	}
	if !origin.into_iter().all(f32::is_finite) {
		return Err(Error::Point);
	}
	let (distance, exponent) = parts(plane.distance);
	let mut sum = BigInt::from(distance) << (exponent + 298) as usize;
	translation_width(&sum)?;
	for axis in 0..3 {
		let (normal, normal_exponent) = parts(plane.normal[axis]);
		let (offset, offset_exponent) = parts(origin[axis]);
		let product =
			BigInt::from(normal * offset) << (normal_exponent + offset_exponent + 298) as usize;
		translation_width(&product)?;
		sum += product;
		translation_width(&sum)?;
	}
	let sign = match sum.sign() {
		IntegerSign::NoSign => return Ok(Some(f32::from_bits(0))),
		IntegerSign::Minus => 0x8000_0000,
		IntegerSign::Plus => 0,
	};
	let magnitude = sum.magnitude();
	let length = magnitude.bits();
	let trailing = magnitude.trailing_zeros().ok_or(Error::Invariant)?;
	let (shift, exponent) = if length <= 172 {
		if trailing < 149 {
			return Ok(None);
		}
		(149, 0)
	} else if length <= 426 {
		let shift = length - 24;
		if trailing < shift {
			return Ok(None);
		}
		(shift, (length - 172) as u32)
	} else {
		return Ok(None);
	};
	let significand = magnitude >> shift as usize;
	let mut digits = significand.iter_u32_digits();
	let mantissa = digits.next().ok_or(Error::Invariant)?;
	if digits.next().is_some() {
		return Err(Error::Arithmetic);
	}
	Ok(Some(f32::from_bits(
		sign | (exponent << 23) | (mantissa & 0x7f_ffff),
	)))
}

fn translation_width(value: &BigInt) -> Result<(), Error> {
	if value.bits() > MAX_TRANSLATION_BITS {
		Err(Error::Arithmetic)
	} else {
		Ok(())
	}
}
