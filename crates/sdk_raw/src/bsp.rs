//! Serialized BSP facts from Valve's public Source SDK 2013 bspfile.h.
//!
//! These describe little-endian file records, not Rust or C++ memory layouts.
//! Reference: source-sdk-2013 b8cfb12c0e083a2ef5b2f9f9b50f3902fa034474,
//! src/public/bspfile.h. Consumers must check each file and lump version.

/// The four bytes starting a little-endian Source BSP.
pub const IDENT: [u8; 4] = *b"VBSP";

/// The first supported BSP version in the pinned public header.
pub const VERSION_19: i32 = 19;

/// Number of lump directory entries.
pub const LUMP_COUNT: usize = 64;

/// Bytes in one lump directory entry.
pub const LUMP_ENTRY_BYTES: usize = 16;

/// Bytes in the identifier, version, directory and map revision.
pub const HEADER_BYTES: usize = 8 + LUMP_COUNT * LUMP_ENTRY_BYTES + 4;

/// Plane records.
pub const LUMP_PLANES: usize = 1;

/// Partition nodes.
pub const LUMP_NODES: usize = 5;

/// Partition leaves.
pub const LUMP_LEAVES: usize = 10;

/// World and brush model records.
pub const LUMP_MODELS: usize = 14;

/// Bytes in a plane record: three normal floats, distance and type.
pub const PLANE_BYTES: usize = 20;

/// Bytes in a node record, including its final padding.
pub const NODE_BYTES: usize = 32;

/// Bytes in a version-zero leaf, including its ambient cube and padding.
pub const LEAF_V0_BYTES: usize = 56;

/// Bytes in a model record.
pub const MODEL_BYTES: usize = 48;
