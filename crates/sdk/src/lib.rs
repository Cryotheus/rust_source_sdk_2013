//! Safer access to a running Source SDK 2013 server from a plugin.
//!
//! # Entry point
//!
//! Everything starts from a [`Server`], which [`Server::new`] creates from the
//! engine's and game server's interface factories for the duration of one
//! callback. That constructor is the only `unsafe` function needed to reach
//! the engine: its contract covers what every other type relies on, and each
//! interface is then a safe accessor away, such as [`Server::valve_engine`] or
//! [`Server::server_game_dll`].
//!
//! # Handles and aliasing
//!
//! The engine and game own their objects and mutate them through their own
//! pointers at any time, including in the middle of calls made from Rust. A
//! Rust reference into such an object would promise an immutability (`&T`) or
//! exclusivity (`&mut T`) that Rust cannot guarantee, so this crate never
//! creates one. Engine objects are reached through handles instead, which
//! store the pointer the engine returned and access the object only through
//! virtual calls and raw reads and writes, like a `Cell`:
//!
//! - Handles are `Copy`, and their methods take `self`. Sharing a handle is
//!   like sharing `&Cell<T>`: exclusivity would protect nothing, since the
//!   engine keeps its own pointers.
//! - Handles carry the scope lifetime of the `Server` they came from, so none
//!   outlives the callback in which the engine keeps the object alive.
//! - Handles are `!Send` and `!Sync`, since the engine is single-threaded.
//! - Data read out of the engine is copied unless it stays unchanged for the
//!   whole scope, such as class names or static strings.
//!
//! Uniqueness is reserved for what the plugin owns: an [`OwnedGameEvent`] is
//! freed exactly once, and a [`GameEventListener`] is registered by pinned
//! reference and is `!Unpin`, since the engine keeps its address.
//!
//! [`OwnedGameEvent`]: interfaces::game_event::OwnedGameEvent
//! [`GameEventListener`]: interfaces::game_event::GameEventListener

#![cfg_attr(docsrs, feature(doc_cfg))]

mod ffi;

pub mod abi;
pub mod datatables;
pub mod edicts;
pub mod entities;
pub mod interfaces;
pub mod math;
pub mod players;
pub mod server;

pub use server::{Game, InterfaceError, InterfaceFactory, Module, Server};
pub use sys;
