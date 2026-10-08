//! Mocks shared by the unit tests: Metamod's `ISmmAPI`, its two hooking
//! libraries, other plugins' hooks on them, and a server without an engine.

pub(crate) mod foreign;
pub(crate) mod harness;
pub(crate) mod khook;
pub(crate) mod smm;
pub(crate) mod sourcehook;

#[cfg(feature = "sdk")]
pub(crate) mod server;
