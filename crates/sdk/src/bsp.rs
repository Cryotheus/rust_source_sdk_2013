//! Checked, owned structural partitions from a Source BSP file.
//!
//! Accepts little-endian BSP 19 and 20 with version-zero planes, nodes and
//! models, and leaf versions zero and one. Required lumps may be raw or Source
//! LZMA. Unsupported layouts and malformed compressed streams are errors.
//! [`entity_lump`] separately copies the version-zero entity payload as bytes.
//! No Source NAV file, native interface or engine context is used.
//!
//! A leaf is a structural partition, not a collision-free room. Brush details,
//! displacements, props, portals, semantic roles and body clearance are not
//! decoded. Unused record fields and other lump contents are not validated.
//! All data here can be owned by an asynchronous worker.

mod decode;
mod lumps;
mod partitions;
#[cfg(test)]
mod tests;

use sdk_raw::bsp as raw;
use thiserror::Error as ThisError;

/// Structural geometry supplied by this decoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Coverage {
	/// Plane-tree constraints only; not complete collision, enclosed rooms,
	/// adjacency, semantic classification or traversability.
	PartitionConstraints,
}

/// Resource limits applied independently to decoding and each later query.
///
/// Counts bound logical storage and work, not exact process RSS or elapsed time.
/// This module reports failures of its fallible reservations. The LZMA dependency
/// allocates its bounded decoder state infallibly, so not every process allocation
/// failure can become an error. The caller already owns the input allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
	/// Maximum input file size in bytes.
	pub input_bytes: usize,
	/// Sum of expanded bytes of all selected lumps, including raw lumps.
	pub expanded_bytes: usize,
	/// Maximum effective dictionary size of each compressed lump.
	pub dictionary_bytes: usize,
	/// Maximum bytes in each LZMA literal-probability table, excluding fixed state.
	pub probability_bytes: usize,
	/// Twice the sum of compressed payload bytes, declared output and one bounded
	/// dictionary chunk of output lookahead, reserving both possible codec passes.
	/// Bounds input/output work, not individual bit operations. Raw lumps consume none.
	pub codec_bytes: usize,
	/// Total decoded plane, node, leaf and model records.
	pub records: usize,
	/// Maximum number of nodes along a path in any decoded model tree.
	pub tree_depth: usize,
	/// Elementary directory, record, tree-event or constraint operations per call.
	/// Codec work has its separate byte bound.
	pub work: usize,
	/// Maximum unfolded world-tree leaf occurrences.
	pub partitions: usize,
	/// Maximum total half-space constraints in the unfolded output.
	pub constraints: usize,
}

impl Default for Limits {
	fn default() -> Self {
		Self {
			input_bytes: 64 * 1024 * 1024,
			expanded_bytes: 64 * 1024 * 1024,
			dictionary_bytes: 64 * 1024 * 1024,
			probability_bytes: 1024 * 1024,
			codec_bytes: 192 * 1024 * 1024,
			records: 262_144,
			tree_depth: 1_024,
			work: 2_000_000,
			partitions: 131_072,
			constraints: 1_000_000,
		}
	}
}

/// The bounded resource an operation needed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resource {
	/// Input file bytes.
	InputBytes,
	/// Sum of expanded selected lump bytes.
	ExpandedBytes,
	/// Effective LZMA dictionary bytes.
	DictionaryBytes,
	/// LZMA literal-probability table bytes.
	ProbabilityBytes,
	/// Compressed input, output and bounded lookahead bytes.
	CodecBytes,
	/// Decoded records and their validation storage.
	Records,
	/// Nodes on a tree path.
	TreeDepth,
	/// Per-call elementary work.
	Work,
	/// Unfolded leaf occurrences.
	Partitions,
	/// Stored path constraints.
	Constraints,
}

/// An invalid field or structural relationship in the decoded subset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ThisError)]
pub enum Invalid {
	/// A selected Source LZMA lump has malformed framing, parameters or data.
	#[error("lump {lump} has invalid compression: {reason}")]
	Compression {
		/// Selected lump.
		lump: usize,
		/// Failure in its compressed representation.
		reason: Compression,
	},
	/// The fixed header is incomplete.
	#[error("the BSP header is incomplete")]
	Header,
	/// The identifier is not VBSP.
	#[error("the BSP identifier is not VBSP")]
	Identifier,
	/// A directory entry is negative, overlaps the header or exceeds the input.
	#[error("lump {lump} has an invalid byte range")]
	LumpRange {
		/// Directory entry.
		lump: usize,
	},
	/// A required lump is not a whole number of records.
	#[error("lump {lump} has an incomplete record")]
	RecordSize {
		/// Required lump.
		lump: usize,
	},
	/// A plane has a non-finite coefficient or a zero normal.
	#[error("plane {record} is not finite or has a zero normal")]
	Plane {
		/// Plane index.
		record: usize,
	},
	/// Bounds are reversed or contain a non-finite value.
	#[error("record {record} in lump {lump} has invalid bounds or origin")]
	Bounds {
		/// Required lump.
		lump: usize,
		/// Record index.
		record: usize,
	},
	/// A decoded tree index is invalid.
	#[error("record {record} in lump {lump} refers outside the decoded arrays")]
	Reference {
		/// Required lump.
		lump: usize,
		/// Record index.
		record: usize,
	},
	/// The mandatory world model or leaf array is empty.
	#[error("the BSP has no world model or leaves")]
	MissingWorld,
	/// A directed node cycle prevents finite tree traversal.
	#[error("node {node} is part of a cycle")]
	Cycle {
		/// Node reached while it was still on the validation path.
		node: usize,
	},
	/// A query position is not finite.
	#[error("the query point is not finite")]
	Point,
	/// A supplied constraint's plane is absent from this BSP.
	#[error("constraint plane {plane} is outside this BSP")]
	Constraint {
		/// Plane index.
		plane: usize,
	},
}

/// A decoder or bounded query failure.
#[derive(Debug, Clone, PartialEq, Eq, ThisError)]
pub enum Error {
	/// This file layout has not been qualified.
	#[error("BSP version {0} is unsupported; versions 19 and 20 are qualified")]
	UnsupportedVersion(i32),
	/// This required lump layout has not been qualified.
	#[error("lump {lump} version {version} is unsupported")]
	UnsupportedLumpVersion {
		/// Required lump.
		lump: usize,
		/// Version recorded in its directory entry.
		version: i32,
	},
	/// A selected compressed lump does not use the supported Source LZMA framing.
	#[error("compressed lump {lump} is unsupported")]
	UnsupportedCompression {
		/// Required lump.
		lump: usize,
	},
	/// Malformed input or an invalid query.
	#[error(transparent)]
	Invalid(#[from] Invalid),
	/// A caller-selected resource bound was exceeded.
	#[error("BSP {resource:?} budget {limit} was exceeded")]
	Budget {
		/// Resource being consumed.
		resource: Resource,
		/// Configured maximum.
		limit: usize,
	},
	/// This module's fallible reservation of bounded owned storage failed.
	/// The LZMA dependency's internal allocation failures are not recoverable here.
	#[error("allocation for BSP {resource:?} failed")]
	Allocation {
		/// Kind of data being allocated.
		resource: Resource,
	},
}

/// Bounds recorded for culling, not an exact free-space boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
	/// Componentwise minimum.
	pub min: [f32; 3],
	/// Componentwise maximum.
	pub max: [f32; 3],
}

/// A serialized splitting plane.
///
/// Query evaluation uses these coefficients without normalization. The unused
/// native plane-type hint is not decoded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plane {
	/// Finite, nonzero plane normal.
	pub normal: [f32; 3],
	/// Finite plane distance from the origin.
	pub distance: f32,
}

impl Plane {
	fn signed_distance(self, point: [f32; 3]) -> f64 {
		let [x, y, z] = self.normal.map(f64::from);
		let [px, py, pz] = point.map(f64::from);
		((x * px + y * py) + z * pz) - f64::from(self.distance)
	}
}

/// A checked reference to another node or a source leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Child {
	/// Index in this BSP's node array.
	Node(usize),
	/// Index in this BSP's leaf array.
	Leaf(usize),
}

/// One splitting node. Child zero is front; child one is back.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Node {
	/// Index in this BSP's plane array.
	pub plane: usize,
	/// Front and back destinations.
	pub children: [Child; 2],
	/// Stored culling bounds.
	pub bounds: Bounds,
}

/// Structural leaf facts, which do not include detailed collision.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Leaf {
	/// Native contents bits; not a body-specific collision decision.
	pub contents: u32,
	/// Native visibility cluster, including its negative sentinels.
	pub cluster: i16,
	/// BSP area, not a semantic room or connectivity certificate.
	pub area: u16,
	/// Native leaf flags, separate from semantic skybox membership.
	pub flags: u16,
	/// Stored culling bounds.
	pub bounds: Bounds,
}

/// The decoded structural subset of a world or brush model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Model {
	/// Stored model bounds.
	pub bounds: Bounds,
	/// Stored origin; no runtime entity transform is applied.
	pub origin: [f32; 3],
	/// Checked tree root. Model zero is the world.
	pub head: Child,
}

/// An immutable, owned structural BSP snapshot.
///
/// Accessors expose immutable arrays. Record copies may be freely inspected or
/// changed by callers without modifying the validated snapshot.
#[derive(Debug, Clone)]
pub struct Bsp {
	version: i32,
	revision: i32,
	planes: Vec<Plane>,
	nodes: Vec<Node>,
	leaves: Vec<Leaf>,
	models: Vec<Model>,
	world_depth: usize,
	limits: Limits,
}

impl Bsp {
	/// Decodes the qualified subset, validating byte ranges and tree indices.
	///
	/// Every node is checked for cycles, including nodes outside model zero.
	/// Shared nodes and leaves are accepted; later world-partition enumeration
	/// unfolds each occurrence with its own constraints under explicit budgets.
	pub fn decode(bytes: &[u8], limits: Limits) -> Result<Self, Error> {
		decode::decode(bytes, limits)
	}

	/// File version.
	pub fn version(&self) -> i32 {
		self.version
	}

	/// Opaque map revision from the header.
	pub fn revision(&self) -> i32 {
		self.revision
	}

	/// The limited geometry this snapshot describes.
	pub fn coverage(&self) -> Coverage {
		Coverage::PartitionConstraints
	}

	/// Splitting planes.
	pub fn planes(&self) -> &[Plane] {
		&self.planes
	}

	/// Checked splitting nodes.
	pub fn nodes(&self) -> &[Node] {
		&self.nodes
	}

	/// Source leaves. Multiple unfolded partitions can refer to one leaf.
	pub fn leaves(&self) -> &[Leaf] {
		&self.leaves
	}

	/// Models, beginning with the world.
	pub fn models(&self) -> &[Model] {
		&self.models
	}
}

struct Work {
	remaining: usize,
	limit: usize,
}

impl Work {
	fn new(limit: usize) -> Self {
		Self {
			remaining: limit,
			limit,
		}
	}

	fn spend(&mut self, amount: usize) -> Result<(), Error> {
		self.remaining = self.remaining.checked_sub(amount).ok_or(Error::Budget {
			resource: Resource::Work,
			limit: self.limit,
		})?;
		Ok(())
	}
}

fn bound(value: usize, limit: usize, resource: Resource) -> Result<(), Error> {
	if value > limit {
		return Err(Error::Budget { resource, limit });
	}
	Ok(())
}

fn reserve<T>(values: &mut Vec<T>, additional: usize, resource: Resource) -> Result<(), Error> {
	values
		.try_reserve(additional)
		.map_err(|_| Error::Allocation { resource })
}

fn finite(point: [f32; 3]) -> bool {
	point.into_iter().all(f32::is_finite)
}

pub use partitions::{HalfSpace, Partition, Side, WorldPartitions};

/// Why a selected Source LZMA representation is malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ThisError)]
pub enum Compression {
	/// A signed expanded-size field is negative.
	#[error("negative expanded size")]
	ExpandedSize,
	/// The complete 17-byte Source header is missing.
	#[error("incomplete Source LZMA header")]
	Header,
	/// The directory and Source header disagree on expanded size.
	#[error("inconsistent expanded size")]
	SizeMismatch,
	/// The stored bytes do not exactly contain the declared encoded payload.
	#[error("inconsistent encoded payload length")]
	PayloadLength,
	/// Invalid LZMA properties or nonzero initial range-code byte.
	#[error("invalid LZMA properties or range initialization")]
	Properties,
	/// The codec rejected the stream, including truncation or an invalid tail.
	#[error("invalid or incomplete LZMA stream")]
	Stream,
	/// The decoded bytes differ in length from the declared output.
	#[error("decoded length differs from the declared output")]
	OutputLength,
	/// The codec did not consume the complete declared payload.
	#[error("unconsumed LZMA payload")]
	TrailingData,
}

/// Copies the version-zero entity lump of a qualified BSP 19 or 20 file.
///
/// Shares the geometry decoder's directory, Source LZMA and resource checks.
/// Preserves arbitrary bytes and terminal NULs; it does not validate UTF-8,
/// KeyValues syntax, entity references or semantics. Other lump contents and
/// geometry are not decoded. Empty raw entity lumps return an empty vector.
pub fn entity_lump(bytes: &[u8], limits: Limits) -> Result<Vec<u8>, Error> {
	let mut work = Work::new(limits.work);
	let directory = lumps::Directory::read(bytes, limits, &mut work)?;
	let mut budget = lumps::Budget::new(limits);
	let lump = directory.prepare(raw::LUMP_ENTITIES, &[0], &mut budget, &mut work)?;
	lump.owned()
}
