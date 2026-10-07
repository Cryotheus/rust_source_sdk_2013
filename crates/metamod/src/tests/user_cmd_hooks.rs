//! Tests of `crate::user_cmd_hooks`: hooks around players' commands, on a
//! mock player class, through the mock SourceHook and KHook.

use super::*;
use crate::test_support::harness::on_both;
use crate::test_support::server::{no_interfaces, tf2_binding};
use source_sdk_2013::sys;
use source_sdk_2013::tf2::class_targets::ClassTarget;
use source_sdk_2013::user_cmd::Buttons;
use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ptr::{self, NonNull};

thread_local! {
	/// What ran during the calls since the last check, in order, with the
	/// buttons of the command it saw.
	static CALLS: RefCell<Vec<(&'static str, Buttons)>> = const { RefCell::new(Vec::new()) };
}

/// The game's `PlayerRunCommand`, which notes the command's buttons, and
/// stops the player's movement, as for a taunt.
unsafe extern "C" fn game_run_command(
	_: *mut sys::CBaseEntity,
	command: *mut sys::CUserCmd,
	_: *mut sys::IMoveHelper,
) {
	// SAFETY: The test passes a live command.
	let command = unsafe { &mut *command };

	CALLS.with_borrow_mut(|calls| {
		calls.push(("game", Buttons::from_bits_retain(command.buttons)));
	});

	command.forwardmove = 0.0;
}

#[test]
fn commands_are_changed_before_the_game_runs_them() {
	/// A player of a C++ class, as far as hooks know it.
	#[repr(C)]
	struct Mock {
		vtable: *mut *mut c_void,
	}

	on_both(|harness| {
		let api = harness.api();
		let slots = Vec::leak(vec![
			game_run_command as RunCommand as *mut c_void;
			PLAYER_RUN_COMMAND_SLOT + 1
		]);
		let mut player = Mock {
			vtable: slots.as_mut_ptr(),
		};
		let this = ptr::from_mut(&mut player).cast::<sys::CBaseEntity>();
		// SAFETY: The test only calls the hooked slot, which holds a method of
		// its signature.
		let target =
			unsafe { ClassTarget::<TfPlayer>::from_raw(NonNull::new(player.vtable).unwrap()) };
		let hooks = api.hook_run_commands(tf2_binding(no_interfaces), on_run_command);

		// SAFETY: Every field of a command is a number, a flag or a pointer, of
		// which all-zero bytes are a value.
		let mut command: sys::CUserCmd = unsafe { MaybeUninit::zeroed().assume_init() };
		command.buttons = (Buttons::ATTACK | Buttons::DUCK).bits();
		command.forwardmove = 450.0;

		assert_eq!(hooks.cover(api, target), Ok(true));

		harness.call::<RunCommand>(
			this,
			PLAYER_RUN_COMMAND_SLOT,
			(&raw mut command, ptr::null_mut()),
		);

		// The hook before the game took the attack away, and the one after it
		// saw the command as the game ran it.
		assert_eq!(
			CALLS.take(),
			[
				("before", Buttons::ATTACK | Buttons::DUCK),
				("game", Buttons::DUCK),
				("after", Buttons::DUCK),
			]
		);
		assert_eq!(command.buttons, Buttons::DUCK.bits() | Buttons::JUMP.bits());
		assert_eq!(command.forwardmove, 0.0);
	});
}

/// The callback, which notes the buttons it saw, then takes the attack away
/// before the game, and adds a jump after it.
fn on_run_command(
	_server: Server<'_>,
	timing: HookTiming,
	_player: Entity<'_>,
	command: &mut UserCmd,
) {
	let name = match timing {
		HookTiming::Pre => "before",
		HookTiming::Post => "after",
	};

	CALLS.with_borrow_mut(|calls| calls.push((name, command.buttons())));

	match timing {
		HookTiming::Pre => command.set_buttons(command.buttons() - Buttons::ATTACK),
		HookTiming::Post => command.set_buttons(command.buttons() | Buttons::JUMP),
	}
}
