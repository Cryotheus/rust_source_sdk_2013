#![cfg_attr(docsrs, feature(doc_cfg))]

mod api;
mod context;
mod plugin;

pub use api::{
	LoaderVersionInfo, MetamodApi, MetamodApiBinding, MetamodFeature, MetamodVersion,
	SourceHookVersions, UnsupportedFeature,
};
pub use context::{CachedContext, ContextKey, cached_context_key};
pub use plugin::{ErrorBuffer, PluginCallbacks, PluginDescriptor, PluginMetadata};
pub use sys;

/// Used by the [`plugin_meta`] macro.
#[doc(hidden)]
pub use crys_bricks::env_cstr as __private_env_cstr;
