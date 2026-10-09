# Metamod:Source (High-level Rust Bindings)

The core crate provides plugin descriptors, callback context, and loader API
selection. The opt-in `sdk` feature adds Source SDK command and hook integration;
`tf2` enables `sdk` and TF2-specific hooks. Engine handles remain callback-scoped
and main-thread-only, including when hook registration persists between calls.

Use nightly Rust on `x86_64-pc-windows-msvc` or `x86_64-unknown-linux-gnu`.
The dependency [`metamod_source_sys`](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/crates/metamod_sys/README.md)
builds C++17 shells for both supported Metamod channels, so both
`METAMOD_SOURCE_STABLE` and `METAMOD_SOURCE_DEV` must be set even when the plugin
will use only one channel. See that crate's native build prerequisites.

```toml
[dependencies]
metamod_source = { version = "0.1.0-alpha.8", features = ["tf2"] }
```

Omit `tf2` for the core API, or select `sdk` for game-independent SDK hooks.
docs.rs builds document the Rust API without compiling the native shells;
ordinary plugin builds still require both external checkouts.

# License

For Valve's Source SDK 2013, see the [SOURCE 1 SDK LICENSE](https://github.com/ValveSoftware/source-sdk-2013/blob/master/LICENSE).
For AlliedModders' Metamod:Source, see their ["zLib/libpng" license](https://github.com/alliedmodders/metamod-source/blob/master/LICENSE.txt)

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/LICENSE-APACHE) or
  https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/LICENSE-MIT) or
  https://opensource.org/licenses/MIT)
