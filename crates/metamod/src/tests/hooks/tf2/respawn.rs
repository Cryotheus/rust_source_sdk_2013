//! Tests of `crate::hooks::tf2::respawn`: pre hooks of `ForceRespawn` on mock player
//! classes, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::{Harness, on_both};
use crate::test_support::server::{no_interfaces, tf2_binding};
use std::cell::RefCell;
use std::ffi::c_void;

thread_local! {
	/// What ran during the calls since the last [`respawn`], in order, with
	/// the object each ran for.
	static CALLS: RefCell<Vec<(&'static str, usize)>> = const { RefCell::new(Vec::new()) };

	/// The address of the player the callback refuses to respawn.
	static REFUSED: Cell<usize> = const { Cell::new(0) };
}

/// A player of a C++ class, as far as hooks know it.
#[repr(C)]
struct Player {
	vtable: *mut *mut c_void,
}

impl Player {
	/// A player of a new class, whose vtable holds `respawn` at
	/// [`FORCE_RESPAWN_SLOT`].
	fn of_new_class(respawn: ForceRespawn) -> Box<Self> {
		let slots = Vec::leak(vec![respawn as *mut c_void; FORCE_RESPAWN_SLOT + 1]);

		Box::new(Self {
			vtable: slots.as_mut_ptr(),
		})
	}

	fn ptr(&mut self) -> NonNull<sys::CBaseEntity> {
		NonNull::from(self).cast()
	}
}

/// The game's `ForceRespawn`, which notes that it ran.
unsafe extern "C" fn game_respawn(this: *mut sys::CBaseEntity) {
	CALLS.with_borrow_mut(|calls| calls.push(("game", this.addr())));
}

/// The callback, which notes the player, and refuses to respawn [`REFUSED`].
fn on_respawn(_server: Server<'_>, player: Entity<'_>) -> RespawnAction {
	let address = player.as_ptr().addr();

	CALLS.with_borrow_mut(|calls| calls.push(("callback", address)));

	if address == REFUSED.get() {
		RespawnAction::Refuse
	} else {
		RespawnAction::Allow
	}
}

/// Calls `player`'s hooked `ForceRespawn`, and returns what ran.
fn respawn(harness: &Harness, player: &mut Player) -> Vec<(&'static str, usize)> {
	CALLS.take();
	harness.call::<ForceRespawn>(player.ptr().as_ptr(), FORCE_RESPAWN_SLOT, ());
	CALLS.take()
}

#[test]
fn respawns_run_unless_a_hook_refuses_them() {
	on_both(|harness| {
		let api = harness.api();
		let mut player = Player::of_new_class(game_respawn);
		let mut bot = Player::of_new_class(game_respawn);
		let mut blocked = Player::of_new_class(game_respawn);
		let (player_address, bot_address) = (player.ptr().addr().get(), bot.ptr().addr().get());

		for object in [player.ptr(), bot.ptr()] {
			// SAFETY: The mock classes have `ForceRespawn` at the slot, and are
			// leaked.
			unsafe { api.install_respawn(object, tf2_binding(no_interfaces), on_respawn) }.unwrap();
		}

		// A second hook of a class is refused, so that each respawn is decided
		// once.
		assert!(matches!(
			// SAFETY: As above.
			unsafe { api.install_respawn(player.ptr(), tf2_binding(no_interfaces), on_respawn) },
			Err(RespawnHookError::Hook(HookError::AlreadyInstalled))
		));

		REFUSED.set(bot_address);
		assert_eq!(
			respawn(harness, &mut player),
			[("callback", player_address), ("game", player_address)]
		);
		assert_eq!(respawn(harness, &mut bot), [("callback", bot_address)]);

		// A respawn an earlier hook refused reaches neither the callback nor
		// the game.
		fn refuse(_call: &HookCall<'_, ForceRespawn>) -> HookAction<()> {
			HookAction::Supersede(())
		}

		// SAFETY: As above.
		unsafe {
			api.add_hook(
				FORCE_RESPAWN,
				HookTarget::class_of(blocked.ptr()),
				HookTiming::Pre,
				&refuse,
			)
			.unwrap();
			api.install_respawn(blocked.ptr(), tf2_binding(no_interfaces), on_respawn)
				.unwrap();
		}

		assert_eq!(respawn(harness, &mut blocked), []);
	});
}
