//! Hand-written ABI of the engine's networking that the generated bindings do
//! not describe.

pub mod incoming;

use std::ffi::c_int;

/// The traffic from the client to the server, for the statistics
/// `INetChannelInfo` keeps per direction.
///
/// This is `FLOW_INCOMING` from `public/inetchannelinfo.h`.
pub const FLOW_INCOMING: c_int = 1;

/// The traffic from the server to the client, for the statistics
/// `INetChannelInfo` keeps per direction.
///
/// This is `FLOW_OUTGOING` from `public/inetchannelinfo.h`.
pub const FLOW_OUTGOING: c_int = 0;
