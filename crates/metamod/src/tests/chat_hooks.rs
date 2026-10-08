//! Tests of `crate::chat_hooks`: post hooks of `CheckChatText` and
//! `CanHearAndReadChatFrom` on mock player classes, through the mock
//! SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, expect, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::tf2::class_targets::ClassTarget;
use std::cell::RefCell;
use std::ffi::{CString, c_char, c_int};
use std::ptr::null_mut;

thread_local! {
	/// What ran during the calls since the last [`say`], in order, with the
	/// object each ran for and the text it saw.
	static CALLS: RefCell<Vec<(&'static str, usize, CString)>> = const { RefCell::new(Vec::new()) };
}

/// A player of a C++ class, as far as hooks know it.
#[repr(C)]
struct Player {
	vtable: *mut *mut c_void,
}

impl Player {
	/// A player of a new class, whose vtable holds [`game_check_chat_text`] at
	/// [`CHECK_CHAT_TEXT_SLOT`].
	fn of_new_class() -> Box<Self> {
		let slots = Vec::leak(vec![
			game_check_chat_text as CheckChatText as *mut c_void;
			CHECK_CHAT_TEXT_SLOT + 1
		]);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
		NonNull::from(self).cast()
	}
}

/// The game's `CheckChatText`, which notes that it ran, and capitalizes the
/// text's first letter in place, as a method may change the text.
unsafe extern "C" fn game_check_chat_text(
	this: *mut sys::CBaseEntity,
	text: *mut c_char,
	size: c_int,
) {
	expect(size == 127, "the game received another size");

	if text.is_null() {
		CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr(), CString::default())));
		return;
	}

	// SAFETY: The tests pass a terminated, writable text.
	let seen = unsafe { CStr::from_ptr(text) }.to_owned();

	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr(), seen)));

	// SAFETY: As above; the first byte is in the text, or its terminator.
	unsafe { *text.cast::<u8>() = (*text.cast::<u8>()).to_ascii_uppercase() };
}

#[test]
fn messages_reach_each_class_hook_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class();
		let mut bot = Player::of_new_class();
		let (player_address, bot_address) = (player.ptr().addr().get(), bot.ptr().addr().get());

		for object in [player.ptr(), bot.ptr()] {
			// SAFETY: The mock classes have `CheckChatText` at the slot, and are
			// leaked.
			unsafe { api.install_chat(object, tf2_binding(no_interfaces), on_chat) }.unwrap();
		}

		// A second hook of a class is refused, so that each message is reported
		// once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_chat(player.ptr(), tf2_binding(no_interfaces), on_chat) },
			Err(ChatHookError::Hook(HookError::AlreadyInstalled))
		));

		// The callback sees the text as the game's method left it.
		assert_eq!(
			say(harness, &mut player, Some(c"hello there")),
			[
				("game", player_address, c"hello there".to_owned()),
				("chat", player_address, c"Hello there".to_owned()),
			]
		);
		assert_eq!(
			say(harness, &mut bot, Some(c"gg")),
			[
				("game", bot_address, c"gg".to_owned()),
				("chat", bot_address, c"Gg".to_owned()),
			]
		);
	});
}

#[test]
fn null_text_reaches_no_callback() {
	on_both(|harness| {
		let mut player = Player::of_new_class();
		let address = player.ptr().addr().get();

		// SAFETY: The mock class has `CheckChatText` at the slot, and is leaked.
		unsafe {
			harness
				.api()
				.install_chat(player.ptr(), tf2_binding(no_interfaces), on_chat)
		}
		.unwrap();

		assert_eq!(
			say(harness, &mut player, None),
			[("game", address, CString::default())]
		);
	});
}

/// The callback, which notes the player and the text it was given.
fn on_chat(_server: Server<'_>, player: Entity<'_>, text: &CStr) {
	CALLS.with_borrow_mut(|calls| calls.push(("chat", player.as_ptr().addr(), text.to_owned())));
}

/// Calls `player`'s hooked `CheckChatText` with a writable copy of `text`, or
/// null, as `Host_Say` passes it, and returns what ran.
fn say(
	harness: &Harness,
	player: &mut Player,
	text: Option<&CStr>,
) -> Vec<(&'static str, usize, CString)> {
	let mut buffer = text.map(|text| text.to_bytes_with_nul().to_vec());
	let pointer = buffer
		.as_mut()
		.map_or(null_mut(), |buffer| buffer.as_mut_ptr().cast::<c_char>());

	CALLS.take();
	harness.call::<CheckChatText>(player.ptr().as_ptr(), CHECK_CHAT_TEXT_SLOT, (pointer, 127));
	CALLS.take()
}

thread_local! {
	/// What [`on_chat_reading`] decides.
	static READ_ACTION: Cell<ChatReadAction> = const { Cell::new(ChatReadAction::Continue) };

	/// What the game's `CanHearAndReadChatFrom` decides.
	static GAME_READS: Cell<bool> = const { Cell::new(true) };

	/// The reader, speaker and decision [`on_chat_reading`] was given since the
	/// last check.
	static READS: RefCell<Vec<(usize, usize, bool)>> = const { RefCell::new(Vec::new()) };
}

#[test]
fn chat_reading_decisions_are_changed_after_the_game() {
	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_can_hear_and_read_chat_from as CanHearAndReadChatFrom
				as *mut c_void;
			CAN_HEAR_AND_READ_CHAT_FROM_SLOT + 1
		]);
		let mut reader = Player {
			vtable: slots.as_mut_ptr(),
		};
		let mut speaker = Player {
			vtable: slots.as_mut_ptr(),
		};
		let (reader, speaker) = (reader.ptr().as_ptr(), speaker.ptr().as_ptr());
		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature.
		let target =
			unsafe { ClassTarget::<TfPlayer>::from_raw(NonNull::new(slots.as_mut_ptr()).unwrap()) };
		let hooks = api.hook_chat_reading(tf2_binding(no_interfaces), on_chat_reading);
		let check = |speaker: *mut sys::CBaseEntity| {
			READS.take();
			let reads = harness.call::<CanHearAndReadChatFrom>(
				reader,
				CAN_HEAR_AND_READ_CHAT_FROM_SLOT,
				(speaker,),
			);
			(reads, READS.take())
		};
		let seen = |reads| vec![(reader.addr(), speaker.addr(), reads)];

		assert_eq!(hooks.cover(api, target), Ok(true));

		READ_ACTION.set(ChatReadAction::Continue);
		assert_eq!(check(speaker), (true, seen(true)));

		READ_ACTION.set(ChatReadAction::Withhold);
		assert_eq!(check(speaker), (false, seen(true)));

		GAME_READS.set(false);
		READ_ACTION.set(ChatReadAction::Read);
		assert_eq!(check(speaker), (true, seen(false)));

		// The server's console has no player to give the callback, and keeps the
		// game's decision.
		assert_eq!(check(null_mut()), (false, vec![]));
		GAME_READS.set(true);
	});
}

/// The game's `CanHearAndReadChatFrom`, which decides [`GAME_READS`].
unsafe extern "C" fn game_can_hear_and_read_chat_from(
	_this: *mut sys::CBaseEntity,
	_speaker: *mut sys::CBaseEntity,
) -> bool {
	GAME_READS.get()
}

/// The callback, which notes the reader, the speaker, and the decision, and
/// decides [`READ_ACTION`].
fn on_chat_reading(
	_server: Server<'_>,
	reader: Entity<'_>,
	speaker: Entity<'_>,
	reads: bool,
) -> ChatReadAction {
	READS.with_borrow_mut(|seen| {
		seen.push((reader.as_ptr().addr(), speaker.as_ptr().addr(), reads))
	});
	READ_ACTION.get()
}
