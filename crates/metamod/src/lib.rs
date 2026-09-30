#![cfg_attr(docsrs, feature(doc_cfg))]

mod api;
mod context;
mod plugin;

#[cfg(feature = "sdk")]
mod commands;

#[cfg(feature = "sdk")]
mod hooks;

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

#[cfg(feature = "sdk")]
#[cfg_attr(docsrs, doc(cfg(feature = "sdk")))]
pub use hooks::{GameFrameFn, HookError, LevelEvents, NetMessageHookError};

pub use plugin::{ErrorBuffer, PluginCallbacks, PluginDescriptor, PluginMetadata};
pub use sys;
