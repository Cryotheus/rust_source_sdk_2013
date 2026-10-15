//! Ambient light from a BSP's leaf ambient lighting lumps: the light Source
//! sampled at points in each leaf as the map was compiled, which lights the
//! models standing there.
//!
//! Each sample is a cube of six linear colours. HDR samples are read where the
//! map has them, LDR ones otherwise. A map whose leaves embed their ambient
//! cube (leaf version zero) and that lacks the separate lumps has no samples
//! here.

use super::lumps::{Budget, Directory};
use super::{Error, Invalid, Leaf, Limits, Work};
use crate::math::glam::Vec3;
use sdk_raw::bsp as raw;
use std::ops::Range;

/// The ambient light samples of one map, by leaf.
#[derive(Debug, Clone, Default)]
pub struct AmbientLighting {
	samples: Vec<AmbientSample>,
	leaves: Vec<Range<usize>>,
	hdr: bool,
}

impl AmbientLighting {
	/// Decodes the HDR ambient lighting lumps, or the LDR ones where there
	/// are none, for the leaves of `leaves`, which [`super::Bsp::leaves`] gives.
	/// A map without either has no samples.
	pub fn decode(bytes: &[u8], limits: Limits, leaves: &[Leaf]) -> Result<Self, Error> {
		let mut work = Work::new(limits.work);
		let directory = Directory::read(bytes, limits, &mut work)?;
		let mut budget = Budget::new(limits);
		let lumps = [
			(
				raw::LUMP_LEAF_AMBIENT_INDEX_HDR,
				raw::LUMP_LEAF_AMBIENT_LIGHTING_HDR,
				true,
			),
			(
				raw::LUMP_LEAF_AMBIENT_INDEX,
				raw::LUMP_LEAF_AMBIENT_LIGHTING,
				false,
			),
		];
		for (index_lump, lighting_lump, hdr) in lumps {
			let index = directory
				.prepare(index_lump, &[0, 1], &mut budget, &mut work)?
				.decode()?;
			if index.is_empty() {
				continue;
			}
			let lighting = directory
				.prepare(lighting_lump, &[0, 1], &mut budget, &mut work)?
				.decode()?;
			let mut decoded = parse(
				&index,
				&lighting,
				[index_lump, lighting_lump],
				leaves,
				&mut work,
			)?;
			decoded.hdr = hdr;
			return Ok(decoded);
		}
		Ok(Self::default())
	}

	/// Whether the samples are HDR.
	pub fn hdr(&self) -> bool {
		self.hdr
	}

	/// The samples of leaf `leaf`, none for a leaf it doesn't know.
	pub fn in_leaf(&self, leaf: usize) -> &[AmbientSample] {
		self.leaves
			.get(leaf)
			.map_or(&[], |range| &self.samples[range.clone()])
	}

	/// The sample of leaf `leaf` nearest to `point`, as Source lights a model
	/// standing there, if the leaf has any.
	pub fn nearest(&self, leaf: usize, point: Vec3) -> Option<&AmbientSample> {
		self.in_leaf(leaf).iter().min_by(|a, b| {
			a.position
				.distance_squared(point)
				.total_cmp(&b.position.distance_squared(point))
		})
	}

	/// Every sample, leaf by leaf.
	pub fn samples(&self) -> &[AmbientSample] {
		&self.samples
	}
}

/// The light Source sampled at one point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AmbientSample {
	/// Where it was sampled.
	pub position: Vec3,
	/// The light on a surface facing +X, -X, +Y, -Y, +Z and -Z, as linear RGB.
	pub cube: [Vec3; 6],
}

impl AmbientSample {
	/// How bright it is: the mean luminance of its six faces, in linear units
	/// where 1 is full white.
	pub fn brightness(&self) -> f32 {
		self.cube
			.iter()
			.map(|color| color.dot(Vec3::new(0.2126, 0.7152, 0.0722)))
			.sum::<f32>()
			/ 6.0
	}
}

/// One linear colour of a `ColorRGBExp32`: each channel times two to the
/// exponent, over 255.
fn color(bytes: &[u8]) -> Vec3 {
	Vec3::new(
		f32::from(bytes[0]),
		f32::from(bytes[1]),
		f32::from(bytes[2]),
	) * 2f32.powi(i32::from(bytes[3] as i8))
		/ 255.0
}

/// The samples `lighting` holds for each leaf of `leaves` that `index` gives
/// a range of, placed in its bounds. `lumps` names the index and lighting
/// lumps in errors.
fn parse(
	index: &[u8],
	lighting: &[u8],
	lumps: [usize; 2],
	leaves: &[Leaf],
	work: &mut Work,
) -> Result<AmbientLighting, Error> {
	let [index_lump, lighting_lump] = lumps;
	if index.len() % raw::LEAF_AMBIENT_INDEX_BYTES != 0
		|| index.len() / raw::LEAF_AMBIENT_INDEX_BYTES != leaves.len()
	{
		return Err(Invalid::RecordSize { lump: index_lump }.into());
	}
	if lighting.len() % raw::LEAF_AMBIENT_LIGHTING_BYTES != 0 {
		return Err(Invalid::RecordSize {
			lump: lighting_lump,
		}
		.into());
	}
	let count = lighting.len() / raw::LEAF_AMBIENT_LIGHTING_BYTES;
	let mut decoded = AmbientLighting::default();
	decoded
		.samples
		.try_reserve(count)
		.map_err(|_| Invalid::RecordSize {
			lump: lighting_lump,
		})?;
	decoded
		.leaves
		.try_reserve(leaves.len())
		.map_err(|_| Invalid::RecordSize { lump: index_lump })?;
	for (record, (entry, leaf)) in index
		.chunks_exact(raw::LEAF_AMBIENT_INDEX_BYTES)
		.zip(leaves)
		.enumerate()
	{
		work.spend(1)?;
		let samples = usize::from(u16::from_le_bytes([entry[0], entry[1]]));
		let first = usize::from(u16::from_le_bytes([entry[2], entry[3]]));
		if first + samples > count {
			return Err(Invalid::Reference {
				lump: index_lump,
				record,
			}
			.into());
		}
		let (min, max) = (Vec3::from(leaf.bounds.min), Vec3::from(leaf.bounds.max));
		let start = decoded.samples.len();
		for sample in first..first + samples {
			work.spend(1)?;
			let bytes = &lighting[sample * raw::LEAF_AMBIENT_LIGHTING_BYTES
				..(sample + 1) * raw::LEAF_AMBIENT_LIGHTING_BYTES];
			let fraction = Vec3::new(
				f32::from(bytes[24]),
				f32::from(bytes[25]),
				f32::from(bytes[26]),
			) / 255.0;
			decoded.samples.push(AmbientSample {
				position: min + (max - min) * fraction,
				cube: std::array::from_fn(|face| color(&bytes[face * 4..face * 4 + 4])),
			});
		}
		decoded.leaves.push(start..decoded.samples.len());
	}
	Ok(decoded)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::bsp::Bounds;

	#[test]
	fn samples_are_placed_in_their_leaf_and_decoded_to_linear_light() {
		let leaf = |min: [f32; 3], max: [f32; 3]| Leaf {
			contents: 0,
			cluster: 0,
			area: 0,
			flags: 0,
			bounds: Bounds { min, max },
		};
		let leaves = [
			leaf([0.0; 3], [255.0; 3]),
			leaf([-100.0, 0.0, 0.0], [100.0, 510.0, 255.0]),
		];
		let index = [[0u16, 0], [2, 0]]
			.iter()
			.flat_map(|&[count, first]| [count.to_le_bytes(), first.to_le_bytes()].concat())
			.collect::<Vec<u8>>();
		let mut lighting = Vec::new();
		for (face, fraction) in [(255u8, [0u8, 51, 255]), (128, [255, 255, 0])] {
			for _ in 0..6 {
				lighting.extend([face, face, face, 0]);
			}
			lighting.extend(fraction);
			lighting.push(0);
		}
		lighting[4..8].copy_from_slice(&[255, 0, 0, 1]);

		let decoded =
			parse(&index, &lighting, [52, 56], &leaves, &mut Work::new(100)).expect("decodes");
		assert!(decoded.in_leaf(0).is_empty());
		let samples = decoded.in_leaf(1);
		assert_eq!(samples.len(), 2);
		assert_eq!(samples[0].position, Vec3::new(-100.0, 102.0, 255.0));
		assert_eq!(samples[1].position, Vec3::new(100.0, 510.0, 0.0));
		assert_eq!(samples[0].cube[0], Vec3::ONE);
		assert_eq!(samples[0].cube[1], Vec3::new(2.0, 0.0, 0.0));
		assert!((samples[1].brightness() - 128.0 / 255.0).abs() < 1e-5);
		assert_eq!(
			decoded.nearest(1, Vec3::new(90.0, 500.0, 0.0)),
			Some(&samples[1])
		);

		assert!(
			parse(
				&index[..4],
				&lighting,
				[52, 56],
				&leaves,
				&mut Work::new(100)
			)
			.is_err()
		);
		assert!(
			parse(
				&index,
				&lighting[..27],
				[52, 56],
				&leaves,
				&mut Work::new(100)
			)
			.is_err()
		);
	}
}
