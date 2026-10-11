# BSP partition fixtures

The default tests in `src/bsp/tests.rs` create small authored BSP byte records.
Their wire offsets are independent of the decoder's constants. They exercise
sloped splits and exact boundaries, shared references, cycles, malformed ranges
and coefficients, and resource limits. They require no engine, NAV file, game
installation or network access.

The ignored `bsp::tests::pinned_dm_lockdown_counts_tree_and_points` test accepts
an explicit local path in `SOURCE_BSP_DM_LOCKDOWN`. It never downloads data.
Before running it, verify the file's SHA-256 against the value below; the test's
size and geometry assertions are independent golden facts, not a substitute for
that hash verification. Verify the same file again after the test if it is not
held immutable by the test runner.

For a reproducible qualification, the runner should read and verify the input,
give the test a private read-only copy of those verified bytes, then verify that
copy again after the process exits. Record the command, exit status, expected
digest and both observed digests with the result.

- Public source: ValveSoftware/source-sdk-2013,
  commit `b8cfb12c0e083a2ef5b2f9f9b50f3902fa034474`.
- File: `game/mod_hl2mp/maps/dm_lockdown.bsp`.
- URL: <https://raw.githubusercontent.com/ValveSoftware/source-sdk-2013/b8cfb12c0e083a2ef5b2f9f9b50f3902fa034474/game/mod_hl2mp/maps/dm_lockdown.bsp>.
- Size: 11,384,400 bytes.
- SHA-256: `93e4c7367b99cf35bae30467acc10c31c8bea48ac5199e80f5e73cc490da3523`.
- BSP version 19, leaf-lump version 0, map revision 10781; required structural
  lumps are uncompressed.
- 6,508 planes, 2,682 nodes, 2,725 leaves and 42 models in the file.
- World root: node 0; 2,319 reachable nodes, 2,320 leaf occurrences, no shared
  world-node occurrences, source leaves 1 through 2320, maximum node depth 25.

Independent offline sampling of the pinned bytes gives these world-tree results:

| Point | Leaf | Cluster | Area | Contents |
| --- | ---: | ---: | ---: | ---: |
| (-3667.48, 3439.48, 12) | 1735 | 805 | 1 | 0 |
| (-3893.09, 1413.9, 8) | 2213 | 1071 | 1 | 0 |
| (-2932, 3218, 8) | 1209 | 561 | 1 | 0 |
| (-3194.96, 3768, 2) | 1617 | 727 | 1 | 0 |
| (-4116, 4600, 2) | 1057 | 484 | 1 | 0 |
| (-3079.48, 4656.52, 138) | 747 | 329 | 1 | 0 |
| (-4066.64, 5622, 12) | 617 | 285 | 1 | 0 |
| (-2651, 4225, 141) | 105 | 38 | 1 | 0 |
| (-2739, 2563, 143) | 1322 | 626 | 1 | 0 |

This public HL2DM fixture contains neither `sky_camera` nor `func_respawnroom`.
It qualifies structural format and point queries only. It supplies no evidence
for TF2 compressed/version-20 decoding. None of these fixtures establishes full
collision, enclosed-room or skybox classification, portal connectivity, body
clearance, installed-map identity or live navigation.

Keep the game map outside the source checkout. Only these small provenance and
golden records are stored here.


## TF2 version-20 compressed fixtures

These public FastDL files are external qualification inputs, not redistributed
map assets. The pinned bytes are independent of any installed or loaded map.
The `.bz2` transport contains concatenated bzip2 streams; extract all streams,
then verify the complete BSP size and SHA-256 below. These URLs may change;
only the pinned decoded bytes qualify the corresponding ignored test.

| Map | Public transport URL | BSP bytes | BSP SHA-256 |
| --- | --- | ---: | --- |
| ctf_2fort | <https://fastdl.tf2dl.net/maps/ctf_2fort.bsp.bz2> | 22,751,863 | `cbd191411c0be57099da73458167001ec80d58bf37c71cb3c36b2911b6e80fd7` |
| arena_byre | <https://fastdl.tf2dl.net/maps/arena_byre.bsp.bz2> | 16,843,394 | `8e61224f8bdcc6c461589b85dc4a0823abcc430a66513a1bbcc131777b49ee50` |

Both files have BSP version 20, leaf version 1 and Source LZMA compression in
lumps 0, 1, 5, 10 and 14. Their compression properties are `5d00000001`:
lc/lp/pb 3/0/2, with a 16 MiB declared dictionary. Independent decoding with
Python/liblzma and a separate range-decoder inspector found complete payload
consumption, arithmetic code zero, and no end-of-stream marker.

| Map | Revision | Planes | Nodes | Leaves | Models | World leaf occurrences | Maximum depth | Entity bytes |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| ctf_2fort | 4067 | 14474 | 7822 | 7971 | 148 | 6337 | 33 | 314566 |
| arena_byre | 2059 | 12348 | 1803 | 1825 | 21 | 1558 | 21 | 129933 |

Both world roots are node zero; world leaf indices are respectively 1 through
6337 and 1 through 1558. Each entity payload ends with one retained NUL.

| Map | Point | Leaf | Cluster | Area | Contents |
| --- | --- | ---: | ---: | ---: | ---: |
| ctf_2fort | (-1888, -1280, 276) | 5974 | 2370 | 17 | 0 |
| ctf_2fort | (1888, 1368, 276) | 1579 | 617 | 8 | 0 |
| ctf_2fort | (-4544, 256, 48) | 3208 | 1287 | 11 | 0 |
| arena_byre | (-1984, -32, -208) | 1216 | 724 | 5 | 0 |
| arena_byre | (-1984, -160, -208) | 1232 | 737 | 5 | 0 |
| arena_byre | (-5120, 0, -1217.83) | 1450 | 843 | 6 | 0 |

The ignored tests are
`bsp::tests::compression::pinned_ctf_2fort_counts_tree_points_and_entities` and
`bsp::tests::compression::pinned_arena_byre_counts_tree_points_and_entities`.
Set `SOURCE_BSP_CTF_2FORT` or `SOURCE_BSP_ARENA_BYRE` to the verified private
snapshot. Apply the same before/after SHA-256 procedure as for `dm_lockdown`.
The last Byre point is first represented as f32, matching the public query API.

## Format and bounded decompression contract

The native format reference is ValveSoftware/source-sdk-2013 at
`b8cfb12c0e083a2ef5b2f9f9b50f3902fa034474`:

- [`src/public/bspfile.h`](https://github.com/ValveSoftware/source-sdk-2013/blob/b8cfb12c0e083a2ef5b2f9f9b50f3902fa034474/src/public/bspfile.h): BSP 19/20; signed directory expanded size; 56-byte leaf v0 and 32-byte leaf v1.
- [`src/public/tier1/lzmaDecoder.h`](https://github.com/ValveSoftware/source-sdk-2013/blob/b8cfb12c0e083a2ef5b2f9f9b50f3902fa034474/src/public/tier1/lzmaDecoder.h): the 17-byte Source header (`LZMA`, actual size, encoded size, five properties).
- [`src/tier1/lzmaDecoder.cpp`](https://github.com/ValveSoftware/source-sdk-2013/blob/b8cfb12c0e083a2ef5b2f9f9b50f3902fa034474/src/tier1/lzmaDecoder.cpp): Valve's size-framed decode.

The decoder validates all directory ranges, then normalizes only selected
lumps. Geometry selects planes/nodes/models version 0 and leaves version 0 or
1. `entity_lump` selects version 0 and returns owned bytes without interpreting
text, KeyValues, references or roles. Unsupported selected versions are errors.
A zero expanded-size directory field denotes raw bytes. A positive value must
match the Source header, whose encoded size must consume the entire stored
lump. Negative expanded sizes, invalid properties and nonzero initial range
bytes fail before codec execution.

The exact dependency is `lzma-rs = 0.3.0`. Its synchronous Finish/unknown-size
mode validates EOF and arithmetic code zero, accepting legitimate markerless
streams. Some valid streams reach this state before their declared final zero
literals: a 32-zero fixture initially yields 31. Only after a successful,
fully consumed first pass may a known-size pass replay the stream to its exact
advertised size. Successful continuation at EOF preserves zero code. An error
in the first pass is final. Both passes must consume the entire payload, and
the final output length must match. An explicit EOS cannot be extended.

Tests include an authored payload whose final byte changes from zero to one:
known-size decoding alone yields the expected five bytes while leaving code
one. The strict first pass rejects it. Markerless known-size representations
can nevertheless be ambiguous for some zero suffixes; validating one declared
size is not a checksum or authenticity guarantee.

Limits cover input bytes, total selected expanded bytes (raw included), record
counts, effective dictionary size (minimum 4096), literal-probability storage,
and twice the compressed-input/output/bounded-lookahead byte work. The two-pass
charge is reserved before codec allocation. Geometry record counts are checked
before decompression. These are logical storage/work bounds, not exact RSS or
time guarantees. This module reserves its output fallibly; the dependency's
bounded internal allocations are infallible, so allocation failure cannot
universally be returned as a typed error.

Default tests use independent authored wire bytes and static compressed vectors,
including explicit-EOS liblzma output and markerless literal-only range output.
They cover the version matrix, mixed compression, byte ownership, framing,
truncation, trailers, size mismatches, early markerless completion, failed-first-
pass behavior, and budget boundaries without downloading or running an engine.
