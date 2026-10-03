//! Hand-written ABI of `IServerPluginHelpers` that the generated bindings do
//! not describe.

/// The cookie `IServerPluginHelpers::StartQueryCvarValue` returns when it
/// refuses a query, as it does for an invalid entity.
///
/// This is `InvalidQueryCvarCookie` from `public/engine/iserverplugin.h`.
#[doc(alias = "InvalidQueryCvarCookie")]
pub const INVALID_QUERY_CVAR_COOKIE: sys::QueryCvarCookie_t = -1;
