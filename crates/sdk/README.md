# Source SDK 2013
*"Somewhat Higher" level Rust Bindings*

Safer and more ergonomic API over `source_sdk_2013_sys`.

Everything starts from a `Server`, created once per engine callback from the
engine's and game server's interface factories. `Server::new` is the one
`unsafe` entry point whose contract underpins the wrappers. Interface accessors
are safe within that scope, but operations with additional requirements, such
as entity spawning and schema-dependent attribute writes, have their own
`unsafe` contracts. No frame, level change, shutdown, immediate entity deletion,
or round restart may invalidate the callback's handles.

As of right now, this is designed for developing Team Fortress 2 server plugins only.

More general use cases are planned:

- Standalone source engine games
- Source engine game mods
- Client mods/plugins

## Features and Targets

Use nightly Rust with Rust 1.99.0 or newer. The supported runtime targets are
`x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`; Windows GNU, Linux musl,
and 32-bit runtime builds are not supported. Ordinary builds use pre-generated
bindings without a C++ SDK or libclang installation.

- `tf2` enables TF2-specific wrappers in `tf2`.
- `tf2_loadout` enables `tf2` and re-applies plugin-chosen wearables and weapon
  attributes when TF2 rebuilds a player's loadout.
- `_test-support` exposes doc-hidden fakes for tests only.

Keep in mind:

- Coverage is incomplete and grows with plugin-development needs
- Soundness rests on `Server::new`'s contract
	- Binds an unsound code base (cough: written in an unsafe language), so the
	  contract assumes the engine, game, and other plugins behave
- It is unlikely a *completely safe* API will ever be made
	- Writing networked variables stays `unsafe`, since the game trusts their values

# License

For Valve's Source SDK 2013, see the [SOURCE 1 SDK LICENSE](https://github.com/ValveSoftware/source-sdk-2013/blob/master/LICENSE).

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/LICENSE-APACHE) or
  https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](https://github.com/Cryotheus/rust_source_sdk_2013/blob/master/LICENSE-MIT) or
  https://opensource.org/licenses/MIT)
