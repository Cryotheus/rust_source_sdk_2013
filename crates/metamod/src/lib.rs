//! Plugin descriptors, callback context, and hooks for Metamod:Source.
//!
//! The core API selects the supported stable or dev loader ABI. Enable `sdk`
//! for Source SDK command and hook integration, or `tf2` for the TF2 hooks;
//! `tf2` also enables `sdk`.
//!
//! Engine access remains main-thread-only and callback-scoped. Registered
//! callbacks and their state must stay valid until the plugin unloads; see the
//! safety contracts on the individual operations.
//!
//! Normal builds compile both native C++17 shells and require the supported
//! stable and dev source checkouts. Hosted docs.rs builds omit the native
//! shells and are for documentation only, not linking a plugin.
#![cfg_attr(docsrs, feature(doc_cfg))]

mod api;
mod context;
pub mod hook;
#[cfg(feature = "tf2")]
pub mod input_hooks;
mod plugin;
mod plugins;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod airblast_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod autobalance_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod bot_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod building_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod channel_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod chat_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod class_hooks;

#[cfg(feature = "sdk")]
mod client_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod collision_hooks;

#[cfg(feature = "sdk")]
mod commands;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod connect_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod crit_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod damage_effect_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod damage_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod death_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod duel_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod delivery_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod entity_factory_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod event_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod fake_client_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod gc_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod give_item_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod key_values_hooks;

#[cfg(feature = "logger")]
#[cfg_attr(docsrs, doc(cfg(feature = "logger")))]
pub mod logger;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod max_health_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod observer_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod physics_collision_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod placement_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod player_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod recipients;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod removal_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod respawn_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod round_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod rules_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod scoreboard_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod script_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod sent_message_hooks;

#[cfg(feature = "sdk")]
mod server_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod sound_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod tag_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod spawn_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod touch_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod team_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod temp_entity_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod transmit_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod think_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod user_cmd_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod voice_chat_hooks;

#[cfg(feature = "tf2")]
mod vote_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod weapon_hooks;

#[cfg(test)]
#[path = "tests/support/mod.rs"]
mod test_support;

pub use api::{
	LoaderVersionInfo, MetamodApi, MetamodApiBinding, MetamodFeature, MetamodVersion,
	SourceHookVersions, UnsupportedFeature,
};

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub use client_hooks::{ClientEvents, ClientFn};

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub use commands::MetamodRegistrar;

pub use context::{CachedContext, ContextKey, cached_context_key};

/// Used by the [`plugin_meta`] macro.
#[doc(hidden)]
pub use crys_bricks::env_cstr as __private_env_cstr;

pub use hook::HookError;
pub use plugin::{ErrorBuffer, PluginCallbacks, PluginDescriptor, PluginMetadata};
pub use plugins::{PluginEntry, PluginId, PluginManager, PluginSource, PluginState};

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub use server_hooks::{
	GameFrameFn, HibernationFn, LevelEvents, NetMessageHookError, ServerThinkFn,
};

pub use sys;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub use vote_hooks::{VoteHookError, VoteHooks};
