//! Hand-written ABI of TF2 players' chat: the vtable slots of
//! `CBasePlayer::CheckChatText`, which the game calls once for each message a
//! player says in chat, and of `CanHearAndReadChatFrom`, which it asks of each
//! player who might read the message, and their signatures.
//!
//! The game's `Host_Say` (`game/server/client.cpp`), which runs the `say` and
//! `say_team` commands of every player, bots included, calls `CheckChatText`
//! once the player may speak (`CanSpeak`), before it sends the message. TF2's
//! players keep `CBasePlayer`'s own, which does nothing. It then sends the
//! message to each other player, of the speaker's team for `say_team`, whose
//! `CanHearAndReadChatFrom` allows the speaker, and who does not ignore them
//! through the voice manager. TF2's own method lets coaches and their students
//! read each other, and otherwise follows the reader's chat settings, keeps
//! the dead from the living unless `tf_gravetalk` is on or the round is over,
//! and keeps each Mann vs. Machine team's chat to itself.

use crate::abi::CppDestructors;
use crate::vtable_slot;
use std::ffi::{c_char, c_int};

/// The signature of `CBasePlayer::CanHearAndReadChatFrom`,
/// `bool (CBasePlayer *)`, with the player who might read a message as its
/// receiver: the player who says it, or null for the server's console, and
/// whether the reader gets it.
///
/// The generated method takes `CTFPlayer` and `CBasePlayer` pointers. A
/// player's primary base, `CBaseEntity`, is at the same address as either, so
/// the method can be called and hooked with entities.
#[doc(alias("CanHearAndReadChatFrom"))]
pub type CanHearAndReadChatFromFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, speaker: *mut sys::CBaseEntity) -> bool;

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

// In the same vtables, `CanHearAndReadChatFrom` is second after
// `PlayerRunCommand`, with `ChangeTeam`'s overload between them. The generated
// method has the signature of `CanHearAndReadChatFromFn`, with `CTFPlayer` and
// `CBasePlayer` pointers.
const _: () = {
	assert!(CAN_HEAR_AND_READ_CHAT_FROM_SLOT == 432 + CppDestructors::VTABLE_SLOTS);
	assert!(CAN_HEAR_AND_READ_CHAT_FROM_SLOT + 3 == CHECK_CHAT_TEXT_SLOT);

	let _: fn(
		&sys::CTFPlayer__bindgen_vtable,
	) -> unsafe extern "C" fn(*mut sys::CTFPlayer, *mut sys::CBasePlayer) -> bool =
		|vtable| vtable.CTFPlayer_CanHearAndReadChatFrom;
};

/// The slot of `CBasePlayer::CanHearAndReadChatFrom` in a TF2 player's
/// primary vtable, from the generated binding.
#[doc(alias("CanHearAndReadChatFrom"))]
pub const CAN_HEAR_AND_READ_CHAT_FROM_SLOT: usize = vtable_slot!(
	sys::CTFPlayer__bindgen_vtable,
	CTFPlayer_CanHearAndReadChatFrom
);

/// The slot of `CBasePlayer::CheckChatText` in a TF2 player's primary vtable,
/// from the generated binding.
#[doc(alias("CheckChatText"))]
pub const CHECK_CHAT_TEXT_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_CheckChatText);
