//! Hand-written low-level FFI and utilities for the Source SDK 2013, for what
//! [`sys`] does not generate.
//!
//! # The three crates
//!
//! - `source_sdk_2013_sys` is the FFI that `source_sdk_2013_bindgen`
//!   generates from Valve's headers: records, enums, constants, and the
//!   vtables of the classes it was asked for. Nothing in it is hand-written.
//! - This crate, `source_sdk_2013_raw`, holds the low-level FFI the generator
//!   does not produce: C++ objects Rust implements for the engine and their
//!   vtables, ABI details such as destructor slots, header values the
//!   generated bindings omit, functions found by signature or symbol, and the
//!   utilities behind them, such as RTTI and signature scanning, in [`util`].
//!   Its modules mirror the module tree of `source_sdk_2013`, the raw half of
//!   `source_sdk_2013::X` living at `source_sdk_2013_raw::X`.
//! - `source_sdk_2013` is the safe, idiomatic API over both, scoped to the
//!   engine callback a plugin runs in.
//!
//! # Soundness
//!
//! Resolving an address, whether through a signature, a symbol, or run-time
//! type information, does not establish its C++ type, ABI, lifetime, or
//! suitability for a native call. Utilities that inspect modules operate on
//! owned byte snapshots, which never borrow mutable engine memory.
//!
//! The contracts of this crate's `unsafe` functions are stated in terms of
//! pointers, threads, and module lifetimes, such as "`this` points to a live
//! `CBaseEntity`", "the module stays loaded", or "on the server's main
//! thread". Higher-level crates discharge them from their own guarantees.

#![cfg_attr(docsrs, feature(doc_cfg))]

#[cfg(not(any(
	all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"),
	all(target_os = "linux", target_arch = "x86_64", target_env = "gnu")
)))]
compile_error!("source_sdk_2013_raw requires Windows x64 MSVC or Linux x64 GNU");

pub mod util;
pub mod weapons;
