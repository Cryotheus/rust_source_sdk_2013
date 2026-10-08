# Source SDK 2013 (Generated Rust Bindings)

The game-side records use TF2's server build configuration, including economy
items and NextBot. Full player and shared-player declarations are available in
`headers::game::server::tf::tf_player` and
`headers::game::shared::tf::tf_player_shared`, and re-exported at the crate root.
Their generated vtables use the target's C++ ABI. Nonvirtual engine symbols
are not linked by these bindings.

To generate the bindings:

- Set environment variable `SOURCE_SDK_2013=/the_path/to_your_repo_for/source-sdk-2013`
  - [Link](https://github.com/ValveSoftware/source-sdk-2013)
- Build with `cargo`, using the`generate_bindings` feature flag
  - E.g. `cargo build -p source_sdk_2013_sys -F generate_bindings`

# License

For Valve's Source SDK 2013, see the [SOURCE 1 SDK LICENSE](https://github.com/ValveSoftware/source-sdk-2013/blob/master/LICENSE).

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](/LICENSE-APACHE) or
  https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](/LICENSE-MIT) or
  https://opensource.org/licenses/MIT)
