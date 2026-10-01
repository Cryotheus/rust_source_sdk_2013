#![cfg_attr(docsrs, feature(doc_cfg))]

mod api;
mod context;
pub mod hook;
mod plugin;

#[cfg(feature = "sdk")]
mod commands;

#[cfg(feature = "sdk")]
mod server_hooks;

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
pub use plugin::{ErrorBuffer, PluginCallbacks, PluginDescriptor, PluginMetadata};

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub use server_hooks::{GameFrameFn, LevelEvents, NetMessageHookError};

pub use sys;

#[cfg(feature = "sdk")]
pub mod damage_hooks;

#[cfg(feature = "sdk")]
mod vote_hooks;
#[cfg(feature = "sdk")]
pub use vote_hooks::{VoteHookError, VoteHooks};
