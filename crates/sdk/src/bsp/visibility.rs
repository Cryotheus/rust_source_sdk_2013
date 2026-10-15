//! A BSP's potentially visible set (PVS), from its visibility lump: which
//! clusters of leaves each cluster may see, as `vvis` worked it out while
//! compiling the map. A leaf names its cluster in [`super::Leaf::cluster`].
//!
//! Each cluster's row is run-length encoded: a zero byte is followed by how
//! many zero bytes it stands for. A map compiled without `vvis` has no
//! clusters here.

use super::lumps::{Budget, Directory};
use super::{Error, Invalid, Limits, Work};
use sdk_raw::bsp as raw;

/// The potentially visible set of one map.
#[derive(Debug, Clone, Default)]
pub struct Visibility {
	clusters: usize,
	/// Each cluster's row of bits, [`Visibility::words`] words long, one bit
	/// for each cluster it may see.
	rows: Vec<u64>,
}

impl Visibility {
	/// Decodes the visibility lump's potentially visible set. A map without
	/// one has no clusters.
	pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self, Error> {
		let mut work = Work::new(limits.work);
		let directory = Directory::read(bytes, limits, &mut work)?;
		let mut budget = Budget::new(limits);
		let lump = directory
			.prepare(raw::LUMP_VISIBILITY, &[0], &mut budget, &mut work)?
			.decode()?;
		parse(&lump, &mut work)
	}

	/// How many clusters it has.
	pub fn cluster_count(&self) -> usize {
		self.clusters
	}

	/// The clusters cluster `from` may see.
	pub fn seen_from(&self, from: usize) -> impl Iterator<Item = usize> + '_ {
		(0..self.clusters).filter(move |&to| self.sees(from, to))
	}

	/// Whether cluster `from` may see cluster `to`. Clusters it doesn't
	/// have, as the negative ones of solid leaves, see nothing.
	pub fn sees(&self, from: usize, to: usize) -> bool {
		from < self.clusters
			&& to < self.clusters
			&& self.rows[from * self.words() + to / 64] & (1 << (to % 64)) != 0
	}

	fn words(&self) -> usize {
		self.clusters.div_ceil(64)
	}
}

/// The rows of `lump`: a count of clusters, each cluster's offset to its
/// PVS row and to its PAS row (which is left unread), and the rows.
fn parse(lump: &[u8], work: &mut Work) -> Result<Visibility, Error> {
	if lump.is_empty() {
		return Ok(Visibility::default());
	}
	let int = |at: usize| -> Option<i32> {
		Some(i32::from_le_bytes(lump.get(at..at + 4)?.try_into().ok()?))
	};
	let size = || Invalid::RecordSize {
		lump: raw::LUMP_VISIBILITY,
	};
	let clusters = int(0)
		.and_then(|count| usize::try_from(count).ok())
		.ok_or_else(size)?;
	if clusters
		.checked_mul(8)
		.and_then(|bytes| bytes.checked_add(4))
		.is_none_or(|end| end > lump.len())
	{
		return Err(size().into());
	}
	let mut decoded = Visibility {
		clusters,
		rows: Vec::new(),
	};
	let words = decoded.words();
	decoded
		.rows
		.try_reserve(clusters * words)
		.map_err(|_| size())?;
	decoded.rows.resize(clusters * words, 0);
	let bytes = clusters.div_ceil(8);
	for cluster in 0..clusters {
		let reference = || Invalid::Reference {
			lump: raw::LUMP_VISIBILITY,
			record: cluster,
		};
		let mut at = int(4 + cluster * 8)
			.and_then(|offset| usize::try_from(offset).ok())
			.ok_or_else(reference)?;
		let row = &mut decoded.rows[cluster * words..(cluster + 1) * words];
		let mut byte = 0;
		while byte < bytes {
			work.spend(1)?;
			match *lump.get(at).ok_or_else(reference)? {
				0 => {
					byte += usize::from(*lump.get(at + 1).ok_or_else(reference)?);
					at += 2;
				}

				bits => {
					row[byte / 8] |= u64::from(bits) << (byte % 8 * 8);
					byte += 1;
					at += 1;
				}
			}
		}
	}
	Ok(decoded)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Rows decode with their zero runs, and a row past the lump's end is
	/// refused.
	#[test]
	fn rows_decode_with_their_zero_runs() {
		// Ten clusters, two bytes a row. Cluster 0 sees 0, 1 and 9; 1 sees
		// 0 and 1; the rest see only themselves, written with zero runs.
		let mut lump = Vec::new();
		lump.extend(10_i32.to_le_bytes());
		let header = 4 + 10 * 8;
		let rows: Vec<Vec<u8>> = (0..10)
			.map(|cluster: usize| match cluster {
				0 => vec![0b11, 0b10],
				1 => vec![0b11, 0, 1],
				cluster if cluster < 8 => vec![1 << cluster, 0, 1],
				cluster => vec![0, 1, 1 << (cluster - 8)],
			})
			.collect();
		let mut offset = header;
		for row in &rows {
			lump.extend((offset as i32).to_le_bytes());
			lump.extend(0_i32.to_le_bytes());
			offset += row.len();
		}
		for row in &rows {
			lump.extend(row);
		}
		let mut work = Work::new(usize::MAX);
		let visibility = parse(&lump, &mut work).expect("decodes");
		assert_eq!(visibility.cluster_count(), 10);
		assert_eq!(visibility.seen_from(0).collect::<Vec<_>>(), [0, 1, 9]);
		assert_eq!(visibility.seen_from(1).collect::<Vec<_>>(), [0, 1]);
		for cluster in 2..10 {
			assert_eq!(visibility.seen_from(cluster).collect::<Vec<_>>(), [cluster]);
		}
		assert!(!visibility.sees(0, 10), "there is no cluster 10");

		lump.truncate(lump.len() - 1);
		assert!(parse(&lump, &mut Work::new(usize::MAX)).is_err());
		assert_eq!(parse(&[], &mut work).expect("decodes").cluster_count(), 0);
	}
}
