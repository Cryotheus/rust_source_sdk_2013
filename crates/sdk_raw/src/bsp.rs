//! Serialized BSP facts from Valve's public Source SDK 2013 bspfile.h.
//!
//! These describe little-endian file records, not Rust or C++ memory layouts.
//! Reference: source-sdk-2013 b8cfb12c0e083a2ef5b2f9f9b50f3902fa034474,
//! src/public/bspfile.h. Consumers must check each file and lump version.

/// Bytes in a brush's first-side index, side count and contents.
pub const BRUSH_BYTES: usize = 12;

/// Bytes in a side's plane, texture, displacement and bevel fields.
pub const BRUSH_SIDE_BYTES: usize = 8;

/// Bytes in the identifier, version, directory and map revision.
pub const HEADER_BYTES: usize = 8 + LUMP_COUNT * LUMP_ENTRY_BYTES + 4;

/// The four bytes starting a little-endian Source BSP.
pub const IDENT: [u8; 4] = *b"VBSP";

/// Bytes in a leaf-brush index.
pub const LEAF_BRUSH_BYTES: usize = 2;

/// Offset of numleafbrushes in both qualified leaf layouts.
pub const LEAF_BRUSH_COUNT_OFFSET: usize = 26;

/// Offset of firstleafbrush in both qualified leaf layouts.
pub const LEAF_FIRST_BRUSH_OFFSET: usize = 24;

/// Bytes in a version-zero leaf, including its ambient cube and padding.
pub const LEAF_V0_BYTES: usize = 56;

/// Bytes in a version-one leaf, without embedded ambient lighting.
pub const LEAF_V1_BYTES: usize = 32;

/// Serialized brush-side constraints.
pub const LUMP_BRUSH_SIDES: usize = 19;

/// Serialized convex brushes.
pub const LUMP_BRUSHES: usize = 18;

/// Number of lump directory entries.
pub const LUMP_COUNT: usize = 64;

/// Entity key/value payload.
pub const LUMP_ENTITIES: usize = 0;

/// Bytes in one lump directory entry.
pub const LUMP_ENTRY_BYTES: usize = 16;

/// Serialized leaf-brush references.
pub const LUMP_LEAF_BRUSHES: usize = 17;

/// Partition leaves.
pub const LUMP_LEAVES: usize = 10;

/// World and brush model records.
pub const LUMP_MODELS: usize = 14;

/// Partition nodes.
pub const LUMP_NODES: usize = 5;

/// Plane records.
pub const LUMP_PLANES: usize = 1;

/// Packed Source LZMA header: magic, output size, encoded size and properties.
pub const LZMA_HEADER_BYTES: usize = 17;

/// Source LZMA magic, from the pinned src/public/tier1/lzmaDecoder.h.
pub const LZMA_IDENT: [u8; 4] = *b"LZMA";

/// Bytes in a model record.
pub const MODEL_BYTES: usize = 48;

/// Bytes in a node record, including its final padding.
pub const NODE_BYTES: usize = 32;

/// Bytes in a plane record: three normal floats, distance and type.
pub const PLANE_BYTES: usize = 20;

/// The first supported BSP version in the pinned public header.
pub const VERSION_19: i32 = 19;

/// The current BSP version in the pinned public header.
pub const VERSION_20: i32 = 20;
