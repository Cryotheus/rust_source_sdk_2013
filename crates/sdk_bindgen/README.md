# Source SDK 2013 (Rust Bindings Generator)

Used by `source_sdk_2013_sys` to generate its contents.

The translation unit uses TF2's server configuration from Valve's
`server_tf.vpc`, `server_econ_base.vpc`, and `nav_mesh.vpc`, with the common
include directories also listed in AlliedModders' `tf2.json` manifest. It
includes the complete `tf_player.h` and `tf_player_shared.h` headers; their
types and transitive dependencies are generated together with typed primary
vtables for `CTFPlayer`, `CTFPlayerShared`, and `CTFWeaponBase`.

Bindings contain no directly linked engine functions. Virtual functions are
called through the generated tables, using the matching Windows MSVC or Linux
Itanium ABI. The generated base entity/player layouts describe TF2's build.

The TF2 headers depend on generated protocol buffer headers. The generator
uses the SDK's bundled protobuf 2.6.1 compiler for the host platform and writes
these headers under Cargo's `OUT_DIR`, leaving the SDK checkout untouched.
Standalone API calls without `OUT_DIR` use temporary headers that are removed
after generation.
The original `.proto` files and compiler are tracked as build inputs, and
unchanged generated headers retain their timestamps. The matching bundled
compiler is required; newer system `protoc` versions do not match the SDK's
protobuf runtime.

# License

For Valve's Source SDK 2013, see the [SOURCE 1 SDK LICENSE](https://github.com/ValveSoftware/source-sdk-2013/blob/master/LICENSE).

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](/LICENSE-APACHE) or
  https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](/LICENSE-MIT) or
  https://opensource.org/licenses/MIT)
