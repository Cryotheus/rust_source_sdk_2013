//! Helpers for tests that fake the engine and the game: mock servers and the
//! interfaces they export, and builders of the engine's objects, which this
//! crate's tests share.
//!
//! These exist for tests only, of this crate and of crates built on it, which
//! enable the `test-support` feature as a dev-dependency. They are not part of
//! the crate's API. The mocks keep their state in thread-locals, which every
//! test starts afresh on its own thread, and leak what they hand the engine
//! wrappers, so it stays valid, at the same address, for the rest of the test
//! process.
//!
//! Helpers that need this crate's private constructors, such as
//! `Entity::from_raw`, only exist in its own unit tests.

pub mod datatables;
pub mod edicts;
pub mod entities;
pub mod interfaces;
pub mod net;
pub mod net_leases;
pub mod players;
pub mod server;
pub mod user_messages;

#[cfg(feature = "tf2")]
pub mod tf2;

#[cfg(test)]
pub(crate) mod sdk_core;

/// Leaks a value, so pointers into it stay valid however the test moves what
/// it keeps.
///
/// For tests only.
pub fn leak<T>(value: T) -> *mut T {
	Box::into_raw(Box::new(value))
}
