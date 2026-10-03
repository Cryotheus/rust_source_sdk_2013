# Source SDK 2013 for Rust (Raw)

Hand-written low-level FFI and utilities for the Source SDK 2013, for what
`source_sdk_2013_sys` does not generate.

- `source_sdk_2013_sys`: the FFI generated from Valve's headers.
- `source_sdk_2013_raw` (this crate): hand-written FFI, such as C++ objects
  implemented in Rust, ABI details, header values the generator omits, and
  functions found by signature or symbol, plus utilities like RTTI and
  signature scanning. Its modules mirror `source_sdk_2013`'s.
- `source_sdk_2013`: the safe, idiomatic API over both.

Resolving an address does not establish its type, ABI, or lifetime. The
`unsafe` contracts here are stated in terms of pointers, threads, and module
lifetimes, which `source_sdk_2013` discharges from its own guarantees.

The `tf2` feature enables Team Fortress 2's game-specific raw bindings.
