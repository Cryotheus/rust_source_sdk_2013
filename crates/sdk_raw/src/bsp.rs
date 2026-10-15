//! Serialized BSP facts from Valve's public Source SDK 2013 bspfile.h.
//!
//! These describe little-endian file records, not Rust or C++ memory layouts.
//! Reference: source-sdk-2013 b8cfb12c0e083a2ef5b2f9f9b50f3902fa034474,
//! src/public/bspfile.h. Consumers must check each file and lump version.

/// Bytes in a brush's first-side index, side count and contents.
pub const BRUSH_BYTES: usize = 12;

/// Bytes in a side's plane, texture, displacement and bevel fields.
pub const BRUSH_SIDE_BYTES: usize = 8;

/// `ddispinfo_t`: start position, vertex and triangle starts, power, contents, face.
pub const DISP_INFO_BYTES: usize = 176;

/// `CDispTri`: one u16 tag set per displacement triangle.
pub const DISP_TRI_BYTES: usize = 2;

/// Displacement triangle tag: buildings may be placed on it.
pub const DISP_TRI_TAG_BUILDABLE: u16 = 1 << 2;

/// Displacement triangle tag: a player may stand on it.
pub const DISP_TRI_TAG_WALKABLE: u16 = 1 << 1;

/// `CDispVert`: offset direction, offset distance and alpha.
pub const DISP_VERT_BYTES: usize = 20;

/// `dedge_t`: two u16 vertex indices.
pub const EDGE_BYTES: usize = 4;

/// `dface_t`.
pub const FACE_BYTES: usize = 56;

/// Offset of a face's i16 displacement-info index, or -1.
pub const FACE_DISP_INFO_OFFSET: usize = 12;

/// Offset of a face's i16 edge count.
pub const FACE_EDGE_COUNT_OFFSET: usize = 8;

/// Offset of a face's i32 first surfedge.
pub const FACE_FIRST_EDGE_OFFSET: usize = 4;

/// Offset of a face's u16 plane index.
pub const FACE_PLANE_OFFSET: usize = 0;

/// Offset of a face's u8 side: nonzero faces against the plane's normal.
pub const FACE_SIDE_OFFSET: usize = 2;

/// Game lump flag: this game lump is Source LZMA compressed.
pub const GAME_LUMP_COMPRESSED: u16 = 1;

/// `dgamelumpheader_t` count, then `dgamelump_t` records: id, flags, version, offset, length.
pub const GAME_LUMP_ENTRY_BYTES: usize = 16;

/// Game lump id of the static prop lump, `sprp` as a big-endian four-character code.
pub const GAME_LUMP_STATIC_PROPS: u32 = u32::from_be_bytes(*b"sprp");

/// Bytes in the identifier, version, directory and map revision.
pub const HEADER_BYTES: usize = 8 + LUMP_COUNT * LUMP_ENTRY_BYTES + 4;

/// The four bytes starting a little-endian Source BSP.
pub const IDENT: [u8; 4] = *b"VBSP";

/// Bytes in a `dleafambientindex_t`: u16 sample count, then u16 first sample.
pub const LEAF_AMBIENT_INDEX_BYTES: usize = 4;

/// Bytes in a `dleafambientlighting_t`: six `ColorRGBExp32` faces, then u8 x,
/// y and z fractions of the leaf's bounds and a pad byte.
pub const LEAF_AMBIENT_LIGHTING_BYTES: usize = 28;

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

/// Displacement records.
pub const LUMP_DISP_INFO: usize = 26;

/// Displacement triangle tags.
pub const LUMP_DISP_TRIS: usize = 48;

/// Displacement vertex offsets.
pub const LUMP_DISP_VERTS: usize = 33;

/// Edges between two vertices.
pub const LUMP_EDGES: usize = 12;

/// Entity key/value payload.
pub const LUMP_ENTITIES: usize = 0;

/// Bytes in one lump directory entry.
pub const LUMP_ENTRY_BYTES: usize = 16;

/// Rendered faces, which displacements are built on.
pub const LUMP_FACES: usize = 7;

/// Game-specific lumps, such as static props.
pub const LUMP_GAME_LUMP: usize = 35;

/// Per-leaf ranges of LDR ambient light samples.
pub const LUMP_LEAF_AMBIENT_INDEX: usize = 52;

/// Per-leaf ranges of HDR ambient light samples.
pub const LUMP_LEAF_AMBIENT_INDEX_HDR: usize = 51;

/// LDR ambient light samples.
pub const LUMP_LEAF_AMBIENT_LIGHTING: usize = 56;

/// HDR ambient light samples.
pub const LUMP_LEAF_AMBIENT_LIGHTING_HDR: usize = 55;

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

/// Signed edge references of faces.
pub const LUMP_SURFEDGES: usize = 13;

/// Face vertex positions.
pub const LUMP_VERTEXES: usize = 3;

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

/// Bytes in one signed surfedge.
pub const SURFEDGE_BYTES: usize = 4;

/// The first supported BSP version in the pinned public header.
pub const VERSION_19: i32 = 19;

/// The current BSP version in the pinned public header.
pub const VERSION_20: i32 = 20;

/// Bytes in one vertex position.
pub const VERTEX_BYTES: usize = 12;
