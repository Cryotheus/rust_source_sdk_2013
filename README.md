# Source SDK 2013 for Rust

[![crates.io](https://img.shields.io/crates/v/source_sdk_2013)](https://crates.io/crates/source_sdk_2013)
[![docs.rs](https://img.shields.io/docsrs/source_sdk_2013)](https://docs.rs/source_sdk_2013)  
*"Somewhat Higher" level Rust Bindings*

For now, this is:

- Only for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`
- Focused for Team Fortress 2 server plugin development

## Usage

Use nightly Rust; the runtime crates declare Rust 1.99.0 as their minimum and
use unstable language features. TF2-specific wrappers are opt-in:

```toml
[dependencies]
source_sdk_2013 = { version = "0.1.0-alpha.8", features = ["tf2"] }
```

`tf2_loadout` also enables `tf2` and adds the loadout re-applier. The internal
`_test-support` feature is for tests, not normal plugin builds. See the
[SDK documentation](https://docs.rs/source_sdk_2013) for callback-scoped
`Server` access and the safety contracts of individual operations.

## Build Inputs

Ordinary SDK builds use the checked-in generated bindings and need neither
Valve's C++ SDK nor libclang. To regenerate them intentionally from a repository
checkout, follow the [binding-generation instructions](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/crates/sdk_sys/README.md).

The separate [Metamod:Source bindings](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/crates/metamod/README.md)
also build native C++17 plugin shells and require both supported Metamod source
checkouts. Their build inputs are not required by `source_sdk_2013` itself.

[*SLoC*](https://ghloc.dev/Cryotheus/rust_source_sdk_2013?branch=master&filter=.rs%24%2C.hpp%24%2C.cpp%24%2C%21%5Ecrates%2Fsdk_sys_)

# License

For Valve's Source SDK 2013, see the [SOURCE 1 SDK LICENSE](https://github.com/ValveSoftware/source-sdk-2013/blob/master/LICENSE).
For AlliedModders' Metamod:Source, see their ["zLib/libpng" license](https://github.com/alliedmodders/metamod-source/blob/master/LICENSE.txt)

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or
  https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or
  https://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in `rust_source_sdk_2013` by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
