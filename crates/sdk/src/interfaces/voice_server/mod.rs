//! `IVoiceServer`, which routes voice between clients.

use crate::ffi::vcall;
use std::ffi::c_int;

interface! {
	/// Routes voice between clients (`IVoiceServer`).
	#[doc(alias = "IVoiceServer")]
	pub struct VoiceServer(sys::IVoiceServer) = Engine c"VoiceServer002";
}

impl<'s> VoiceServer<'s> {
	/// Whether the client with entity index `receiver` hears `sender`.
	#[doc(alias = "GetClientListening")]
	pub fn client_listening(self, receiver: c_int, sender: c_int) -> bool {
		// SAFETY: `Server::new` guarantees the interface is live, and the engine
		// validates the indices.
		unsafe { vcall!(self.as_ptr() => IVoiceServer_GetClientListening(receiver, sender)) }
	}

	/// Sets whether `receiver` hears `sender`, returning whether both are clients.
	#[doc(alias = "SetClientListening")]
	pub fn set_client_listening(self, receiver: c_int, sender: c_int, listen: bool) -> bool {
		// SAFETY: As for `client_listening`.
		unsafe {
			vcall!(self.as_ptr() => IVoiceServer_SetClientListening(receiver, sender, listen))
		}
	}
}
