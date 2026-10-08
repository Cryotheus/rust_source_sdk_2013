//! Hand-written ABI of `IServerPluginHelpers` that the generated bindings do
//! not describe, and the statuses a client answers its console variable
//! queries with.
//!
//! The generated `EQueryCvarValueStatus` is `c_int` on Windows but `c_uint` on
//! Linux, so the statuses are given here as `c_int`, as the mirror of
//! `CLC_RespondCvarValue` holds them.

use std::ffi::{CStr, c_int};

/// The cookie `IServerPluginHelpers::StartQueryCvarValue` returns when it
/// refuses a query, as it does for an invalid entity.
///
/// This is `InvalidQueryCvarCookie` from `public/engine/iserverplugin.h`.
#[doc(alias("InvalidQueryCvarCookie"))]
pub const INVALID_QUERY_CVAR_COOKIE: sys::QueryCvarCookie_t = -1;

/// The status of an answer from a client that has a console command, not a
/// variable, of the queried name.
#[doc(alias("eQueryCvarValueStatus_NotACvar"))]
#[allow(
	clippy::unnecessary_cast,
	reason = "`EQueryCvarValueStatus` is `c_int` on Windows but `c_uint` on Linux"
)]
pub const QUERY_CVAR_NOT_A_CVAR: c_int =
	sys::EQueryCvarValueStatus_eQueryCvarValueStatus_NotACvar as c_int;

/// The status of an answer from a client that has no console variable of the
/// queried name.
#[doc(alias("eQueryCvarValueStatus_CvarNotFound"))]
#[allow(
	clippy::unnecessary_cast,
	reason = "`EQueryCvarValueStatus` is `c_int` on Windows but `c_uint` on Linux"
)]
pub const QUERY_CVAR_NOT_FOUND: c_int =
	sys::EQueryCvarValueStatus_eQueryCvarValueStatus_CvarNotFound as c_int;

/// The status of an answer from a client whose variable of the queried name
/// does not allow queries, and whose value it therefore withholds.
#[doc(alias("eQueryCvarValueStatus_CvarProtected"))]
#[allow(
	clippy::unnecessary_cast,
	reason = "`EQueryCvarValueStatus` is `c_int` on Windows but `c_uint` on Linux"
)]
pub const QUERY_CVAR_PROTECTED: c_int =
	sys::EQueryCvarValueStatus_eQueryCvarValueStatus_CvarProtected as c_int;

/// The status of an answer that carries the queried variable's value.
#[doc(alias("eQueryCvarValueStatus_ValueIntact"))]
#[allow(
	clippy::unnecessary_cast,
	reason = "`EQueryCvarValueStatus` is `c_int` on Windows but `c_uint` on Linux"
)]
pub const QUERY_CVAR_VALUE_INTACT: c_int =
	sys::EQueryCvarValueStatus_eQueryCvarValueStatus_ValueIntact as c_int;

/// The version string `IServerPluginHelpers` is exported and requested under.
///
/// This is `INTERFACEVERSION_ISERVERPLUGINHELPERS` from
/// `public/engine/iserverplugin.h`.
#[doc(alias("INTERFACEVERSION_ISERVERPLUGINHELPERS"))]
pub const VERSION: &CStr = c"ISERVERPLUGINHELPERS001";
