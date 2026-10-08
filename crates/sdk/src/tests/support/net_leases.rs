//! Shorthands for tests of `sv_cheats` leases: a coordinator, a client's
//! answer, and what a lease sends, as [`Decoded`] messages.

use super::net::cheats::{Decoded, response};
use crate::net::cheats::{CheatsOptions, ClientCheats};
use crate::net::incoming::Verdict;
use crate::players::UserId;
use std::ffi::{CStr, c_int};

/// A coordinator with the default options.
///
/// For tests only.
pub const fn cheats() -> ClientCheats {
	ClientCheats::new(CheatsOptions::DEFAULT)
}

/// Passes `cheats` a client's answer, to the query carrying `cookie`, that
/// its `sv_cheats` is set, returning what it decided.
///
/// For tests only.
pub fn confirm(cheats: &mut ClientCheats, user_id: UserId, cookie: c_int) -> Verdict {
	cheats.on_response(user_id, &response(cookie, 0, c"1"))
}

/// What a lease with `cookie` sends, running `commands`: `sv_cheats 1`, the
/// commands, then the query.
///
/// For tests only.
pub fn spoof(cookie: c_int, commands: &[&CStr]) -> Vec<Decoded> {
	let mut messages = vec![Decoded::SetConVar(vec![(c"sv_cheats".into(), c"1".into())])];

	messages.extend(
		commands
			.iter()
			.map(|&command| Decoded::StringCmd(command.into())),
	);
	messages.push(Decoded::Query(cookie, c"sv_cheats".into()));
	messages
}
