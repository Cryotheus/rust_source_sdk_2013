# Source SDK 2013 for Rust (Raw)

Hand-written low-level FFI and utilities for the Source SDK 2013,
for what `source_sdk_bindgen` does not output into `source_sdk_2013_sys`

- `source_sdk_2013_sys`: the FFI generated from Valve's headers.
- `source_sdk_2013_raw` (this crate): hand-written FFI, such as C++ objects
  implemented in Rust, ABI details, header values the `source_sdk_bindgen` omits, and
  functions found by signature or symbol, plus utilities like RTTI and
  signature scanning. Its modules mirror `source_sdk_2013`'s.
- `source_sdk_2013`: the safer, idiomatic API over both.

Resolving an address does not establish its type, ABI, or lifetime. The
`unsafe` contracts here are stated in terms of pointers, threads, and module
lifetimes, which `source_sdk_2013` discharges from its own guarantees.

For now, this is:

- Only for Windows and Linux, on x86-64 targets (no 32bit support)
- Focused for Team Fortress 2 server plugin development

# License

For Valve's Source SDK 2013, see the [SOURCE 1 SDK LICENSE](https://github.com/ValveSoftware/source-sdk-2013/blob/master/LICENSE).

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](/LICENSE-APACHE) or
  https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](/LICENSE-MIT) or
  https://opensource.org/licenses/MIT)

at your option.
