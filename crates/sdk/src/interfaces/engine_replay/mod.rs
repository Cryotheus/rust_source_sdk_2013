//! `IEngineReplay`, the engine's services for its replay system, among them
//! the recalculation of the server's tags.
//!
//! The engine keeps the server's tags in `sv_tags` (see
//! [`server_game_tags`](super::server_game_tags)). It recalculates them as
//! levels start and as variables marked
//! [`NOTIFY`](crate::commands::CommandFlags::NOTIFY) change: for each variable
//! the game lists, it adds the variable's tag while the variable is not at its
//! default and removes it otherwise, and leaves every other tag, such as one a
//! plugin added, as it is. A game's hook on that list, such as one that
//! [excludes](super::server_game_tags::TaggedConVar::exclude) a variable, runs
//! for each recalculation.
//!
//! The engine recalculates the tags for a change in the change callback that
//! also announces it, which does nothing for a variable without the flag. A
//! variable set with [`ConVar::set_string_quietly`], whose flag is cleared
//! for the change, so leaves the tags as they were, until the next level or
//! the next announced change. [`EngineReplay::recalculate_tags`] has the engine
//! recalculate them at once instead.
//!
//! The engine is not public. All of the above is inferred, not read in its
//! source: what its recalculation does, that it runs in the callback that
//! announces a change, and that the engine exports this interface. It is
//! inferred from the interface's header, which declares its version, from
//! TF2's client library, which requests it from the engine, from TF2's list
//! of the variables that tag the server, which notes that they need
//! `FCVAR_NOTIFY` "so the tags are recalculated and uploaded to the master
//! server when the convar is changed" (`game/shared/tf/tf_gamerules.cpp`),
//! and from how `sv_tags` changes as those variables do. Should the engine
//! not export the interface, [`Server::engine_replay`] fails.
//!
//! [`ConVar::set_string_quietly`]: super::cvar::ConVar::set_string_quietly
//! [`Server::engine_replay`]: crate::Server::engine_replay

#[cfg(test)]
#[path = "../../tests/interfaces/engine_replay.rs"]
mod tests;

use sdk_raw::interfaces::engine_replay::{IEngineReplay, VERSION};
use sdk_raw::vcall;

interface! {
	/// The engine's services for its replay system (`IEngineReplay`), which
	/// the engine is inferred to export as `EngineReplay001` (see the
	/// [module documentation](self)).
	#[doc(alias("IEngineReplay", "CEngineReplay"))]
	pub struct EngineReplay(IEngineReplay) = Engine VERSION;
}

impl EngineReplay<'_> {
	/// Has the game server recalculate its tags (`sv_tags`) now, as the engine
	/// is inferred to do as a level starts and as a variable marked
	/// [`NOTIFY`](crate::commands::CommandFlags::NOTIFY) changes; see the
	/// [module documentation](self).
	///
	/// The engine sets `sv_tags` as any change does, which runs its change
	/// callbacks. To keep the engine from announcing that change, run this
	/// within [`ConVar::quietly`](super::cvar::ConVar::quietly) of `sv_tags`.
	#[doc(alias("RecalculateTags"))]
	pub fn recalculate_tags(self) {
		// SAFETY: `Server::new` guarantees the interface is live, its vtable is
		// laid out as `IEngineReplayVtable`, and the handle is only used on the
		// server's main thread.
		unsafe { vcall!(self.as_ptr() => recalculate_tags()) }
	}
}
