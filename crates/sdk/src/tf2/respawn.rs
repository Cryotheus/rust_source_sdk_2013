//! TF2's player respawns: how the game brings a player back, dead or not, and
//! where to intercept it.
//!
//! Every respawn of a player goes through `CTFPlayer::ForceRespawn`, which the
//! game calls through the player's vtable: respawn waves, a round's restart,
//! `game_forcerespawn`'s inputs, choosing a class, and the script bindings'
//! `ForceRespawn` included. Only direct calls of the player's `Spawn` bypass
//! it: a player's first spawn as it is put in the server, and scripts'
//! `DispatchSpawn` of a player.
//!
//! [`sdk_raw::tf2::respawn`] holds the function's signature and its vtable
//! slot, [`FORCE_RESPAWN_SLOT`](sdk_raw::tf2::respawn::FORCE_RESPAWN_SLOT).
//! `metamod_source`'s `respawn_hooks` hook it, to see every such respawn
//! before the game and to refuse it.
