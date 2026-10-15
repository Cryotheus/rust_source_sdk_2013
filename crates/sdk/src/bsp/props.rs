//! Static props from a BSP's `sprp` game lump: which model stands where, and
//! how it collides.
//!
//! Only placements are decoded. A prop's collision shape lives in its model's
//! `.phy` file, outside the map, which the caller reads and applies with
//! [`StaticProp::transform`].

use super::lumps::{Budget, Directory};
use super::{Error, Invalid, Limits, Work};
use crate::math::glam::{Affine3A, Quat, Vec3};
use sdk_raw::bsp as raw;

/// Bytes of a static prop record shared by every supported version.
const COMMON_BYTES: usize = 56;

/// Bytes of a model name in the dictionary.
const NAME_BYTES: usize = 128;

/// How a static prop collides, from its record's solid field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropSolid {
	/// Not solid.
	None,
	/// Its model's bounding box.
	BoundingBox,
	/// Its model's `.phy` collision.
	Physics,
	/// A value this decoder does not interpret.
	Other(u8),
}

/// One placed static prop.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StaticProp {
	/// Index into [`StaticProps::models`].
	pub model: u16,
	pub origin: Vec3,
	/// Pitch, yaw and roll in degrees, as Source stores a `QAngle`.
	pub angles: Vec3,
	pub solid: PropSolid,
	pub skin: i32,
	/// Serialized static prop flags.
	pub flags: u8,
}

impl StaticProp {
	/// Model space to world space, rotating like Source's `AngleMatrix`.
	pub fn transform(&self) -> Affine3A {
		let [pitch, yaw, roll] = self.angles.to_array().map(f32::to_radians);
		let rotation =
			Quat::from_rotation_z(yaw) * Quat::from_rotation_y(pitch) * Quat::from_rotation_x(roll);
		Affine3A::from_rotation_translation(rotation, self.origin)
	}
}

/// The static props of one map.
#[derive(Debug, Clone, Default)]
pub struct StaticProps {
	/// Model paths, such as `models/props_farm/wooden_barrel.mdl`.
	pub models: Vec<String>,
	pub props: Vec<StaticProp>,
	/// The `sprp` lump version.
	pub version: u16,
}

impl StaticProps {
	/// Decodes the `sprp` game lump. A map without one has no static props.
	pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self, Error> {
		let mut work = Work::new(limits.work);
		let directory = Directory::read(bytes, limits, &mut work)?;
		let mut budget = Budget::new(limits);
		let lump = directory
			.prepare(raw::LUMP_GAME_LUMP, &[0], &mut budget, &mut work)?
			.decode()?;
		let record_size = || {
			Error::from(Invalid::RecordSize {
				lump: raw::LUMP_GAME_LUMP,
			})
		};
		let count = usize::try_from(integer(&lump, 0).ok_or_else(record_size)?)
			.map_err(|_| record_size())?;
		let file = directory.file();
		for index in 0..count {
			work.spend(1)?;
			let at = 4 + index * raw::GAME_LUMP_ENTRY_BYTES;
			let entry = lump
				.get(at..at + raw::GAME_LUMP_ENTRY_BYTES)
				.ok_or_else(record_size)?;
			if u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]])
				!= raw::GAME_LUMP_STATIC_PROPS
			{
				continue;
			}
			let flags = u16::from_le_bytes([entry[4], entry[5]]);
			let version = u16::from_le_bytes([entry[6], entry[7]]);
			let range = || {
				Error::from(Invalid::LumpRange {
					lump: raw::LUMP_GAME_LUMP,
				})
			};
			let start =
				usize::try_from(integer(entry, 8).ok_or_else(range)?).map_err(|_| range())?;
			let length =
				usize::try_from(integer(entry, 12).ok_or_else(range)?).map_err(|_| range())?;
			let data = if flags & raw::GAME_LUMP_COMPRESSED != 0 {
				// The declared length is the expanded size; the stored size
				// is in the LZMA header.
				let header = file
					.get(start..start + raw::LZMA_HEADER_BYTES)
					.ok_or_else(range)?;
				let encoded =
					u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
				let stored = file
					.get(start..start + raw::LZMA_HEADER_BYTES + encoded)
					.ok_or_else(range)?;
				super::lumps::Lump::framed(
					raw::LUMP_GAME_LUMP,
					stored,
					length,
					i32::from(version),
					&mut budget,
				)?
				.decode()?
			} else {
				std::borrow::Cow::Borrowed(file.get(start..start + length).ok_or_else(range)?)
			};
			return parse(&data, version, &mut work);
		}
		Ok(Self::default())
	}
}

fn integer(bytes: &[u8], at: usize) -> Option<i32> {
	Some(i32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn parse(data: &[u8], version: u16, work: &mut Work) -> Result<StaticProps, Error> {
	if !(4..=11).contains(&version) {
		return Err(Error::UnsupportedLumpVersion {
			lump: raw::LUMP_GAME_LUMP,
			version: i32::from(version),
		});
	}
	let invalid = || {
		Error::from(Invalid::RecordSize {
			lump: raw::LUMP_GAME_LUMP,
		})
	};
	let count = |at: usize| -> Result<usize, Error> {
		usize::try_from(integer(data, at).ok_or_else(invalid)?).map_err(|_| invalid())
	};
	let names = count(0)?;
	let mut models = Vec::new();
	models
		.try_reserve(names.min(data.len() / NAME_BYTES))
		.map_err(|_| invalid())?;
	for index in 0..names {
		work.spend(1)?;
		let name = data
			.get(4 + index * NAME_BYTES..4 + (index + 1) * NAME_BYTES)
			.ok_or_else(invalid)?;
		let end = name
			.iter()
			.position(|&byte| byte == 0)
			.unwrap_or(NAME_BYTES);
		models.push(String::from_utf8_lossy(&name[..end]).into_owned());
	}
	let leaves_at = 4 + names * NAME_BYTES;
	let props_at = leaves_at + 4 + count(leaves_at)? * 2;
	let props = count(props_at)?;
	let records = &data.get(props_at + 4..).ok_or_else(invalid)?;
	if props == 0 {
		return Ok(StaticProps {
			models,
			props: Vec::new(),
			version,
		});
	}
	// Record sizes differ by version and game, so they come from the lump.
	let stride = records.len() / props;
	if stride < COMMON_BYTES || stride * props != records.len() {
		return Err(invalid());
	}
	let float = |record: &[u8], at: usize| {
		f32::from_le_bytes([record[at], record[at + 1], record[at + 2], record[at + 3]])
	};
	let vector = |record: &[u8], at: usize| {
		Vec3::new(
			float(record, at),
			float(record, at + 4),
			float(record, at + 8),
		)
	};
	let mut placed = Vec::new();
	placed.try_reserve(props).map_err(|_| invalid())?;
	for record in records.chunks_exact(stride) {
		work.spend(1)?;
		let model = u16::from_le_bytes([record[24], record[25]]);
		if usize::from(model) >= models.len() {
			return Err(Invalid::Reference {
				lump: raw::LUMP_GAME_LUMP,
				record: placed.len(),
			}
			.into());
		}
		placed.push(StaticProp {
			model,
			origin: vector(record, 0),
			angles: vector(record, 12),
			solid: match record[30] {
				0 => PropSolid::None,
				2 => PropSolid::BoundingBox,
				6 => PropSolid::Physics,
				other => PropSolid::Other(other),
			},
			skin: i32::from_le_bytes([record[32], record[33], record[34], record[35]]),
			flags: record[31],
		});
	}
	Ok(StaticProps {
		models,
		props: placed,
		version,
	})
}
