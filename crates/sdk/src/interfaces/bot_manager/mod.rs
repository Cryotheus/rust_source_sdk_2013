//! `IBotManager`, which creates bots.

use crate::edicts::Edict;
use crate::ffi::vcall;
use std::ffi::CStr;
use std::ptr::NonNull;

interface! {
	/// Creates and controls bots (`IBotManager`).
	#[doc(alias = "IBotManager")]
	pub struct BotManager(sys::IBotManager) = GameServer c"BotManager001";
}

impl<'s> BotManager<'s> {
	/// Connects a bot, returning its edict, or `None` if the server is full.
	///
	/// The bot's player spawns through the game's usual connection path, which
	/// runs game code and fires game events synchronously.
	#[doc(alias = "CreateBot")]
	pub fn create_bot(self, name: &CStr) -> Option<Edict<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let edict = unsafe { vcall!(self.as_ptr() => IBotManager_CreateBot(name.as_ptr())) };

		// SAFETY: The edict table outlives `'s`.
		NonNull::new(edict).map(|edict| unsafe { Edict::from_raw(edict) })
	}
}
