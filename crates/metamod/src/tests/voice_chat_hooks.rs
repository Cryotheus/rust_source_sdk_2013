//! Tests of `crate::voice_chat_hooks`: hooks changing who hears whom after
//! the game, on a mock voice chat helper, through the mock SourceHook and
//! KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use std::cell::{Cell, RefCell};
use std::ptr;

thread_local! {
	/// What [`on_voice_chat`] decides.
	static ACTION: Cell<VoiceChatAction> = const { Cell::new(VoiceChatAction::Continue) };

	/// The listeners, talkers and decisions [`on_voice_chat`] was given since
	/// the last check.
	static SEEN: RefCell<Vec<(usize, usize, bool)>> = const { RefCell::new(Vec::new()) };
}

/// The listener.
const LISTENER: usize = 0x1150;

/// The talker.
const TALKER: usize = 0x7a1c;

/// The game's `CanPlayerHearPlayer`, which lets the listener hear the talker
/// alone.
unsafe extern "C" fn game_can_hear(
	_: *mut c_void,
	listener: *mut sys::CBasePlayer,
	talker: *mut sys::CBasePlayer,
	_: *mut bool,
) -> bool {
	(listener.addr(), talker.addr()) == (LISTENER, TALKER)
}

#[test]
fn the_helper_is_found_in_the_class_targets() {
	on_both(|harness| {
		let api = harness.api();
		let scope = ();
		let binding = tf2_binding(no_interfaces);
		// SAFETY: The server's game module is this test's executable, which
		// `no_interfaces` is in, and which has no voice chat helper.
		let server = unsafe { binding.server(&scope) };
		let targets = ClassTargets::load(server).unwrap();

		assert!(matches!(
			api.hook_voice_chat(&targets, binding, on_voice_chat),
			Err(VoiceChatHookError::Target(ClassTargetError::NotFound(
				VOICE_GAME_MGR_HELPER_CLASS
			)))
		));
	});
}

#[test]
fn voice_chat_decisions_are_changed_after_the_game() {
	/// The voice chat helper, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_can_hear as CanPlayerHearPlayer as *mut c_void;
			CAN_PLAYER_HEAR_PLAYER_SLOT + 1
		]);
		let mut helper = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut helper).cast::<c_void>();
		let mut proximity = false;
		let mut hears = |listener: usize, talker: usize| {
			SEEN.take();
			let hears = harness.call::<CanPlayerHearPlayer>(
				this,
				CAN_PLAYER_HEAR_PLAYER_SLOT,
				(
					ptr::without_provenance_mut(listener),
					ptr::without_provenance_mut(talker),
					&raw mut proximity,
				),
			);
			(hears, SEEN.take())
		};

		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature, with mock players the callback only takes the
		// addresses of.
		let hooks = unsafe {
			api.install_voice_chat(
				NonNull::new(helper.vtable).unwrap(),
				tf2_binding(no_interfaces),
				on_voice_chat,
			)
		}
		.unwrap();

		ACTION.set(VoiceChatAction::Continue);
		assert_eq!(
			hears(LISTENER, TALKER),
			(true, vec![(LISTENER, TALKER, true)])
		);
		assert_eq!(
			hears(TALKER, LISTENER),
			(false, vec![(TALKER, LISTENER, false)])
		);

		ACTION.set(VoiceChatAction::Mute);
		assert_eq!(
			hears(LISTENER, TALKER),
			(false, vec![(LISTENER, TALKER, true)])
		);

		ACTION.set(VoiceChatAction::Hear);
		assert_eq!(
			hears(TALKER, LISTENER),
			(true, vec![(TALKER, LISTENER, false)])
		);

		// Removed, the hook no longer runs.
		hooks.remove(api);
		assert_eq!(hears(TALKER, LISTENER), (false, vec![]));
	});
}

/// The callback, which notes the call and decides [`ACTION`].
fn on_voice_chat(
	_server: Server<'_>,
	listener: Entity<'_>,
	talker: Entity<'_>,
	hears: bool,
) -> VoiceChatAction {
	SEEN.with_borrow_mut(|seen| {
		seen.push((listener.as_ptr().addr(), talker.as_ptr().addr(), hears));
	});

	ACTION.get()
}
