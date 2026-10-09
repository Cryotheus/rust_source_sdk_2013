//! Raw Metamod:Source ABI declarations for the supported 64-bit builds.
//!
//! The **1.12 build 1226** and **2.0 build 1469** layouts differ in the middle of
//! `ISmmAPI`, so neither layout can be used before the common prefix has
//! identified the running version. 2.0 build 1472 keeps 1469's layouts.
//!
//! Each version hooks through its own library: [`sourcehook`] in 1.12, and
//! [`khook`] in 2.0.
//!
//! Normal builds require both external source checkouts and compile their
//! C++17 shells. docs.rs omits those shells to document the Rust declarations;
//! its documentation-only builds cannot be used to link a plugin.
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod api;
pub mod khook;
pub mod plugin;
pub mod plugin_manager;
pub mod sourcehook;
