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
for TF2 compressed/version-20 decoding, full collision, enclosed-room or skybox
classification, portal connectivity, body clearance, or live navigation.

Keep the game map outside the source checkout. Only these small provenance and
golden records are stored here.
