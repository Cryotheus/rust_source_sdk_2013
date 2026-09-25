//! `IEngineSound`, the server's sound system.

use crate::ffi::vcall;
use std::ffi::CStr;

interface! {
	/// The server's sound system (`IEngineSound`).
	#[doc(alias = "IEngineSound")]
	pub struct EngineSound(sys::IEngineSound) = Engine c"IEngineSoundServer003";
}

impl<'s> EngineSound<'s> {
	/// Adds a sound to the precache table, which clients load before playing
	/// it. Returns whether the sound could be precached.
	#[doc(alias = "PrecacheSound")]
	pub fn precache_sound(self, sample: &CStr, preload: bool) -> bool {
		// SAFETY: `Server::new` guarantees the interface is live.
		unsafe {
			vcall!(self.as_ptr() => IEngineSound_PrecacheSound(sample.as_ptr(), preload, false))
		}
	}

	#[doc(alias = "IsSoundPrecached")]
	pub fn is_sound_precached(self, sample: &CStr) -> bool {
		// SAFETY: As for `precache_sound`.
		unsafe { vcall!(self.as_ptr() => IEngineSound_IsSoundPrecached(sample.as_ptr())) }
	}
}
