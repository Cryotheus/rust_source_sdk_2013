//! User message payloads, and the recipients the engine sends messages and
//! sounds to.

use crate::bitbuf::BitWriter;
use crate::user_messages::UserMessage;

/// The payload `message` writes, as the game's own sender would.
///
/// For tests only.
///
/// # Panics
///
/// If the message cannot be encoded.
pub fn payload(message: &impl UserMessage) -> BitWriter {
	let mut out = BitWriter::new();

	message.write(&mut out).unwrap();
	out
}

/// Recipients with exactly these player indices, in order, sent reliably if
/// `reliable` is set.
///
/// # Panics
///
/// If an index is listed twice, which recipients keep once, or is negative.
#[cfg(test)]
pub(crate) fn recipients(
	players: &[std::ffi::c_int],
	reliable: bool,
) -> crate::user_messages::Recipients {
	use crate::edicts::Edict;
	use crate::user_messages::Recipients;
	use sdk_raw::test_support::edicts::mock_edict;

	let mut recipients = Recipients::new();

	for &index in players {
		let edict = std::ptr::NonNull::from(Box::leak(Box::new(mock_edict(index, false))));

		// SAFETY: The edict is leaked, so it outlives the handle, which
		// `add` only reads the index of.
		recipients.add(unsafe { Edict::from_raw(edict) });
	}

	assert_eq!(recipients.players(), players, "each player is listed once");

	match reliable {
		true => recipients.reliable(),
		false => recipients,
	}
}
