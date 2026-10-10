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

## Optional Logging

`logger` provides a direct `log::Log` implementation with an injected Rust sink
and message-only output. `logger_pretty` adds console level/target formatting
and the existing severity palette. The plain logger has no styling dependency.
Neither feature installs a global logger or retains engine state.

Use `logger::queue::LogQueue` for worker messages. It limits retained records and
UTF-8 text bytes, rejects new records on overflow, and exposes drop counts.
Draining takes an owned batch and releases the lock before forwarding. The host
must drain through a current main-thread callback, reject interior NULs before
calling `MetamodApi::log_cstr`, restore its own callback TLS, and join/retire worker
and global forwarding state before unloading. Close each load's queue and create
a new one for the next generation; closing does not make unloadable global logger
references safe. See the module's callback forwarding example.

Pretty output needs an independently routed console destination. Unknown,
replicated engine, file and RCON destinations stay message-only, even when the
console override is `always`. Console precedence is explicit `always`/`never`,
then presence of `NO_COLOR`, presence of `CLICOLOR_FORCE`, `CLICOLOR` zero/nonzero,
then supplied Windows VT-enabled or Unix TTY state. Presence includes empty
values and `CLICOLOR_FORCE=0`, following this policy literally. Inputs are
injected; rendering does not read or change process environment or console modes.
The optional `anstyle` dependency renders the indexed severity palette and ANSI
resets identically on Windows and Unix; it has no terminal detection or shared
color policy. The plain `logger` feature does not depend on this color library.
Per-RCON socket identity, response routing and a client policy command are not
provided by this first logging foundation.

# License

For Valve's Source SDK 2013, see the [SOURCE 1 SDK LICENSE](https://github.com/ValveSoftware/source-sdk-2013/blob/master/LICENSE).
For AlliedModders' Metamod:Source, see their ["zLib/libpng" license](https://github.com/alliedmodders/metamod-source/blob/master/LICENSE.txt)

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/LICENSE-APACHE) or
  https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/LICENSE-MIT) or
  https://opensource.org/licenses/MIT)
