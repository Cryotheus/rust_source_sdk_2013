# Source SDK 2013 (Generated Rust Bindings)

The game-side records use TF2's server build configuration, including economy
items and NextBot. Full player and shared-player declarations are available in
`headers::game::server::tf::tf_player` and
`headers::game::shared::tf::tf_player_shared`, and re-exported at the crate root.
Their generated vtables use the target's C++ ABI. Nonvirtual engine symbols
are not linked by these bindings.

Ordinary builds select the checked-in bindings for `x86_64-pc-windows-msvc` or
`x86_64-unknown-linux-gnu`. Use nightly Rust with Rust 1.99.0 or newer.
There is no `generate_bindings` feature and compilation does not regenerate
the bindings.

To regenerate them intentionally from the repository root, set
`SOURCE_SDK_2013` to a [Valve SDK checkout](https://github.com/ValveSoftware/source-sdk-2013)
and provide libclang and the target's native headers and toolchain. Run the
alias for the matching build environment:

```text
cargo +nightly bind-windows
cargo +nightly bind-linux
```

From a configured Windows/WSL checkout, `cargo +nightly bind-wsl` uses `wsl.env`.
These aliases run `source_sdk_2013_bindgen_runner` and rewrite the selected
platform crate's generated files. See the [generator prerequisites](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/crates/sdk_bindgen/README.md),
including the SDK's bundled protobuf compiler. Generation is not necessary to
use the published crates.

# License

For Valve's Source SDK 2013, see the [SOURCE 1 SDK LICENSE](https://github.com/ValveSoftware/source-sdk-2013/blob/master/LICENSE).

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/LICENSE-APACHE) or
  https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/LICENSE-MIT) or
  https://opensource.org/licenses/MIT)
