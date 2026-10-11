//! Plugin descriptors, callback context, and hooks for Metamod:Source.
//!
//! The core API selects the supported stable or dev loader ABI. Enable `sdk`
//! for Source SDK command and hook integration, or `tf2` for the TF2 hooks;
//! `tf2` also enables `sdk`. The hook families live in `hooks` and
//! `hooks::tf2`; [`hook`] provides low-level registration and dispatch.
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
mod plugin;
mod plugins;

#[cfg(feature = "sdk")]
mod commands;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod hooks;

#[cfg(feature = "logger")]
#[cfg_attr(docsrs, doc(cfg(feature = "logger")))]
pub mod logger;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub mod recipients;

#[cfg(test)]
#[path = "tests/support/mod.rs"]
mod test_support;

pub use api::{
	LoaderVersionInfo, MetamodApi, MetamodApiBinding, MetamodFeature, MetamodVersion,
	SourceHookVersions, UnsupportedFeature,
};

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub use commands::MetamodRegistrar;

pub use context::{CachedContext, ContextKey, cached_context_key};

/// Used by the [`plugin_meta`] macro.
#[doc(hidden)]
pub use crys_bricks::env_cstr as __private_env_cstr;

pub use hook::HookError;

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::channel as channel_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub use hooks::client::{ClientEvents, ClientFn};

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::connect as connect_hooks;

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::delivery as delivery_hooks;

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::entity_factory as entity_factory_hooks;

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::event as event_hooks;

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::fake_client as fake_client_hooks;

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::gc as gc_hooks;

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::key_values as key_values_hooks;

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::sent_message as sent_message_hooks;

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub use hooks::server::{
	GameFrameFn, HibernationFn, LevelEvents, NetMessageHookError, ServerThinkFn,
};

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::sound as sound_hooks;

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::tag as tag_hooks;

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::temp_entity as temp_entity_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::airblast as airblast_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::autobalance as autobalance_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::bot as bot_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::building as building_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::chat as chat_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::class as class_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::collision as collision_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::crit as crit_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::damage as damage_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::damage_effect as damage_effect_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::death as death_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::dispenser as dispenser_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::duel as duel_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::give_item as give_item_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::input as input_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::max_health as max_health_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::melee as melee_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::observer as observer_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::physics_collision as physics_collision_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::placement as placement_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::player as player_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::removal as removal_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::respawn as respawn_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::round as round_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::rules as rules_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::scoreboard as scoreboard_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::script as script_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::spawn as spawn_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::team as team_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::think as think_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::touch as touch_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::user_cmd as user_cmd_hooks;

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::voice_chat as voice_chat_hooks;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub use hooks::tf2::vote::{VoteHookError, VoteHooks};

#[cfg(feature = "tf2")]
#[doc(hidden)]
pub use hooks::tf2::weapon as weapon_hooks;

#[cfg(feature = "sdk")]
#[doc(hidden)]
pub use hooks::transmit as transmit_hooks;

pub use plugin::{ErrorBuffer, PluginCallbacks, PluginDescriptor, PluginMetadata};
pub use plugins::{PluginEntry, PluginId, PluginManager, PluginSource, PluginState};
pub use sys;
