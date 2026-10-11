//! Source SDK hooks. Enable `sdk` for these modules and `tf2` for `tf2`.
//!
//! Low-level registration and dispatch remain in [`crate::hook`].

pub mod channel;
pub mod client;
pub mod connect;
pub mod delivery;
pub mod entity_factory;
pub mod event;
pub mod fake_client;
pub mod gc;
pub mod key_values;
pub mod sent_message;
pub mod server;
pub mod sound;
pub mod tag;
pub mod temp_entity;
pub mod transmit;

#[cfg(feature = "tf2")]
#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
pub mod tf2;
