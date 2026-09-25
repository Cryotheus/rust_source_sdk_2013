# Source SDK 2013 ("Somewhat Higher" level Rust Bindings)

Safer and more ergonomic API over `source_sdk_2013_sys`.

Everything starts from a `Server`, created once per engine callback from the
engine's and game server's interface factories. `Server::new` is the one
`unsafe` call needed to reach the engine; every interface is a safe accessor
on it from there.

As of right now, this is designed for developing Team Fortress 2 server plugins only.

More general use cases are planned:

- Standalone source engine games
- Source engine game mods
- Client mods/plugins

Keep in mind:

- Coverage is not 100%, it's probably less than 5%
	- The SDK is huge, I'm adding things as I need them
- Soundness rests on `Server::new`'s contract
	- Binds an unsound code base (cough: written in an unsafe language), so the
	  contract assumes the engine, game, and other plugins behave
- It is unlikely a *completely safe* API will ever be made
	- Writing networked variables stays `unsafe`, since the game trusts their values

# License

For Valve's Source SDK 2013, see the [SOURCE 1 SDK LICENSE](https://github.com/ValveSoftware/source-sdk-2013/blob/master/LICENSE).

This project is licensed under either of

* Apache License, Version 2.0, ([LICENSE-APACHE](/LICENSE-APACHE) or
  https://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](/LICENSE-MIT) or
  https://opensource.org/licenses/MIT)
