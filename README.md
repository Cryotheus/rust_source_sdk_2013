# Source SDK 2013 for Rust

[![crates.io](https://img.shields.io/crates/v/source_sdk_2013)](https://crates.io/crates/source_sdk_2013)
[![docs.rs](https://img.shields.io/docsrs/source_sdk_2013)](https://docs.rs/source_sdk_2013)  
*"Somewhat Higher" level Rust Bindings*

For now, this is:

- Only for Windows and Linux, on x86-64 targets (no 32bit support)
- Focused for Team Fortress 2 server plugin development

To generate the bindings yourself, check [`source_sdk_2013_sys`](/crates/sdk_sys)

A small, non-generated, crate for Metamod:Source bindings ([`metamod_source`](/crates/metamod)) is also provided.

## TF2 gameplay APIs

All entity access stays inside a main-thread `Server` callback. Keep an
`EntityHandle` between callbacks and resolve it again before using it.

| Capability | API |
| --- | --- |
| Player conditions | `conditions::PlayerConditions::{add, remove, in_cond, remove_all}`; `Condition` validates generated `ETFCond` IDs and `ConditionDuration` validates lifetimes. |
| Damage | `damage::DamageInfo` edits amount, type, and critical classification. With Metamod's `sdk` feature, `MetamodApi::hook_player_damage` intercepts incoming or already-scaled damage. |
| Voting | Listen to `voting::VoteEvent::EVENTS` and decode with `VoteEvent::from_event`. `MetamodApi::hook_vote_starts` can veto built-in player, server, and coordinator requests. Ballots contain entity indices, not user IDs. |
| Weapons | `weapons::PlayerWeapons` queries inventory slots, gives stock weapons by classname or non-stock weapons by `ItemDefinitionIndex`, equips/detaches, and replaces with rollback on failure. |
| Attributes | `attributes::Attributes` reads, sets, and removes default-type numeric gameplay attributes on weapons, wearables, and players; player attributes optionally expire. Names are item-schema names such as `damage bonus`. |

Condition and attribute operations call TF2's native typed binding adapters,
without executing script text or requiring a script VM. Their effects follow
the game's duration, cache invalidation, and cleanup rules. Unknown native
signatures return errors instead of guessing object layouts.

Weapon creation/replacement is unsafe for the same reason as manual entity
spawning: the caller must ensure the game and other plugins do not immediately
delete entities during the callback. Attribute setters are unsafe because the
caller must verify the schema attribute uses the default 32-bit numeric type
and that its value is valid for that attribute. The native getter does not
support explicit `float`, string, blob, or 64-bit schema types; string/blob
attributes cannot be placed in the game's 32-bit runtime list.
These APIs do not change a Steam inventory or save changes across respawns.

`PlayerWeapons::give_item` and `replace_item` use the game's item generator,
which initializes the selected definition, models, and built-in attributes
before spawning. `ItemDefinitionIndex::IRON_BOMBER` is 1151 and
`ItemDefinitionIndex::BRASS_BEAST` is 312; `ItemDefinitionIndex::new` accepts
other schema indices. These items use Unique quality and level 1. For example,
inside a callback where the spawning safety contract is satisfied:

```rust,ignore
use source_sdk_2013::weapons::{ItemDefinitionIndex, PlayerWeapons, WeaponSlot};

let weapons = PlayerWeapons::new(server, player)?;
unsafe { weapons.replace_item(WeaponSlot::PRIMARY, ItemDefinitionIndex::IRON_BOMBER)? };
```

The `give_item_as` and `replace_item_as` variants accept a compatible concrete
classname for schema definitions with generic names, such as
`tf_weapon_shotgun_soldier` for a shotgun. Unknown definitions and unsupported
game binaries return errors. Rejected items are removed; failed replacements
attempt to restore the old weapon.

Install damage hooks on each distinct player class, including bots. Hooks are
managed across pause and unload; Metamod 2.0 can activate them asynchronously,
so an installation followed immediately by a test call may precede activation.
`DamageAction::Apply` invokes the original with an owned copy and supersedes
the intercepted call; later hooks on that same call do not edit the copy.
The late critical policy removes the recorded damage bonus, but does not undo
earlier audiovisual effects or statistics. See the API docs for these limits.

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
