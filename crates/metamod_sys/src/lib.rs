//! Raw Metamod:Source ABI declarations for the supported 64-bit builds.
//!
//! The **1.12 build 1226** and **2.0 build 1469** layouts differ in the middle of
//! `ISmmAPI`, so neither layout can be used before the common prefix has
//! identified the running version.
#![cfg_attr(docsrs, feature(doc_cfg))]

pub mod api;
pub mod plugin;
