//! Hand-written ABI of TF2 players' chat: the vtable slot of
//! `CBasePlayer::CheckChatText`, which the game calls once for each message a
//! player says in chat, and its signature.
//!
//! The game's `Host_Say` (`game/server/client.cpp`), which runs the `say` and
//! `say_team` commands of every player, bots included, calls it once the
//! player may speak (`CanSpeak`), before it sends the message. TF2's players
//! keep `CBasePlayer`'s own, which does nothing.

use crate::abi::CppDestructors;
use crate::vtable_slot;
use std::ffi::{c_char, c_int};

/// The signature of `CBasePlayer::CheckChatText`, `void (char *, int)`, with
/// the player as its receiver: the message's text, which the method can change
/// in place, and the size it can grow to.
///
/// The generated method takes a `CTFPlayer` receiver. It is the player's
/// primary base, `CBaseEntity`, at the same address, so the method can be
/// called and hooked with an entity receiver.
#[doc(alias("CheckChatText"))]
pub type CheckChatTextFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, text: *mut c_char, size: c_int);

// The slot lies between those of `PlayerRunCommand` and `RemoveWearable`,
// which SourceMod's `gamedata/sdktools.games/game.tf.txt` and
// `gamedata/sm-tf2.games.txt` list as 431 and 440 on Windows, one more on
// Linux, as the generated vtable has them. The `CTFPlayer` vtable of an older
// 32-bit Linux `server.so`, as dumped in sigsegv's `mvm-reversed`, has the
// same ten methods in the same order from `PlayerRunCommand` to
// `RemoveWearable`, with `CheckChatText` fifth after `PlayerRunCommand`. The
// generated method has the signature of `CheckChatTextFn`, with a `CTFPlayer`
// receiver.
const _: () = {
	assert!(CHECK_CHAT_TEXT_SLOT == 435 + CppDestructors::VTABLE_SLOTS);

	let _: fn(
		&sys::CTFPlayer__bindgen_vtable,
	) -> unsafe extern "C" fn(*mut sys::CTFPlayer, *mut c_char, c_int) =
		|vtable| vtable.CTFPlayer_CheckChatText;
};

/// The slot of `CBasePlayer::CheckChatText` in a TF2 player's primary vtable,
/// from the generated binding.
#[doc(alias("CheckChatText"))]
pub const CHECK_CHAT_TEXT_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_CheckChatText);
