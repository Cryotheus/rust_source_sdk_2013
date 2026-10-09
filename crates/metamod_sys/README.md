# Metamod:Source (Raw Rust Bindings)

Hand-written ABI declarations and C++ plugin shells for the supported 64-bit
Metamod:Source builds. Use nightly Rust and a C++17 compiler for
`x86_64-pc-windows-msvc` or `x86_64-unknown-linux-gnu`.

The bridge layouts support stable 1.12 build 1226 and dev 2.0 builds 1469/1472.
The native shells have been built with the 1.12 build 1226 and 2.0 git1472 source
checkouts; use matching sources, not an arbitrary latest checkout. Initialize
the dev checkout's recursive submodules, including `third_party/khook`.

Both checkouts are required, even when the plugin uses only one channel:

```text
METAMOD_SOURCE_STABLE=path/to/metamod-source-1.12-build-1226
METAMOD_SOURCE_DEV=path/to/metamod-source-2.0-git1472
```

Export these variables before invoking Cargo. A repository checkout can also
provide its root `.env`; an application using the published crate should not
rely on that repository-local file. Neither source checkout is bundled in the
crate. Valve's C++ SDK and libclang are only needed for separate SDK binding
generation, not for these native shells.

## Hosted Documentation

docs.rs has no external Metamod checkouts. When its `DOCS_RS` environment
variable is set, the build script validates the supported target but skips
native shell compilation and source lookup. Those builds only document the
Rust declarations and cannot link a plugin. Leave `DOCS_RS` unset for ordinary
builds, tests, and package verification so both C++ shells are compiled.

# License

For Valve's Source SDK 2013, see the [SOURCE 1 SDK LICENSE](https://github.com/ValveSoftware/source-sdk-2013/blob/master/LICENSE).
For AlliedModders' Metamod:Source, see their ["zLib/libpng" license](https://github.com/alliedmodders/metamod-source/blob/master/LICENSE.txt)

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/LICENSE-APACHE) or
  https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/LICENSE-MIT) or
  https://opensource.org/licenses/MIT)
