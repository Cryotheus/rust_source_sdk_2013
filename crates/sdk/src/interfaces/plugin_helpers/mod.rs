//! `IServerPluginHelpers`, the engine's services for server plugins.

use crate::edicts::Edict;
use crate::ffi::vcall;
use std::ffi::{CStr, c_int};

/// `InvalidQueryCvarCookie` from `public/engine/iserverplugin.h`.
const INVALID_QUERY_CVAR_COOKIE: c_int = -1;

interface! {
	/// The engine's services for server plugins (`IServerPluginHelpers`).
	#[doc(alias = "IServerPluginHelpers")]
	pub struct PluginHelpers(sys::IServerPluginHelpers) = Engine c"ISERVERPLUGINHELPERS001";
}

/// Identifies a pending client console variable query.
#[doc(alias = "QueryCvarCookie_t")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueryCvarCookie(pub c_int);

impl<'s> PluginHelpers<'s> {
	/// Runs a command on a client, as though its player entered it.
	#[doc(alias = "ClientCommand")]
	pub fn client_command(self, client: Edict<'_>, command: &CStr) {
		// SAFETY: `Server::new` guarantees the interface is live, and the edict is live.
		unsafe {
			vcall!(self.as_ptr() => IServerPluginHelpers_ClientCommand(client.as_ptr(), command.as_ptr()))
		};
	}

	/// Asks a client for the value of one of its console variables.
	///
	/// The answer arrives through the plugin's `OnQueryCvarValueFinished`
	/// callback, carrying the returned cookie.
	#[doc(alias = "StartQueryCvarValue")]
	pub fn start_query_cvar_value(self, client: Edict<'_>, name: &CStr) -> Option<QueryCvarCookie> {
		// SAFETY: As for `client_command`.
		let cookie = unsafe {
			vcall!(self.as_ptr() => IServerPluginHelpers_StartQueryCvarValue(client.as_ptr(), name.as_ptr()))
		};

		(cookie != INVALID_QUERY_CVAR_COOKIE).then_some(QueryCvarCookie(cookie))
	}
}
