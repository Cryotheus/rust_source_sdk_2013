//! Tests of `crate::user_cmd`: views of player commands.

use super::*;
use std::mem::MaybeUninit;

/// A command as a client sends one, with every input set.
fn command() -> sys::CUserCmd {
	// SAFETY: Every field of a command is a number, a flag or a pointer, of
	// which all-zero bytes are a value.
	let mut command: sys::CUserCmd = unsafe { MaybeUninit::zeroed().assume_init() };

	command.command_number = 812;
	command.tick_count = 66_150;
	command.viewangles = sys::QAngle {
		x: 12.5,
		y: -90.0,
		z: 0.0,
	};
	command.forwardmove = 450.0;
	command.sidemove = -225.0;
	command.upmove = 0.0;
	command.buttons = raw::IN_ATTACK | raw::IN_DUCK | 1 << 30;
	command.impulse = 201;
	command.weaponselect = 77;
	command.weaponsubtype = 2;
	command.random_seed = 0x3fa9;
	command.mousedx = -14;
	command.mousedy = 3;
	command
}

#[test]
fn buttons_keep_the_engines_bits() {
	assert_eq!(Buttons::ATTACK.bits(), 1);
	assert_eq!(Buttons::ATTACK3.bits(), 1 << 25);
	assert_eq!(Buttons::MOVE_RIGHT.bits(), raw::IN_MOVERIGHT);
	assert_eq!(Buttons::all().bits(), !0);
}

#[test]
fn commands_are_changed_in_place() {
	let mut command = command();
	// SAFETY: The command is live and unaliased while the view is used.
	let view = unsafe { UserCmd::from_raw_mut(NonNull::from(&mut command)) };

	view.set_buttons((view.buttons() - Buttons::ATTACK) | Buttons::JUMP);
	view.set_forward_move(0.0);
	view.set_side_move(110.0);
	view.set_up_move(-30.0);
	view.set_impulse(0);
	view.set_weapon_select(0, 0);
	view.set_view_angles(QAngle {
		pitch: 0.0,
		yaw: 45.0,
		roll: 5.0,
	});

	assert_eq!(command.buttons, raw::IN_DUCK | raw::IN_JUMP | 1 << 30);
	assert_eq!(command.forwardmove, 0.0);
	assert_eq!(command.sidemove, 110.0);
	assert_eq!(command.upmove, -30.0);
	assert_eq!(command.impulse, 0);
	assert_eq!((command.weaponselect, command.weaponsubtype), (0, 0));
	assert_eq!(
		(
			command.viewangles.x,
			command.viewangles.y,
			command.viewangles.z
		),
		(0.0, 45.0, 5.0)
	);
}

#[test]
fn commands_read_what_the_client_sent() {
	let mut command = command();
	// SAFETY: The command is live and unaliased while the view is used.
	let view = unsafe { UserCmd::from_raw_mut(NonNull::from(&mut command)) };

	assert_eq!(view.command_number(), 812);
	assert_eq!(view.tick_count(), 66_150);
	assert_eq!(
		view.view_angles(),
		QAngle {
			pitch: 12.5,
			yaw: -90.0,
			roll: 0.0
		}
	);
	assert_eq!(
		(view.forward_move(), view.side_move(), view.up_move()),
		(450.0, -225.0, 0.0)
	);

	// Bits without a name are kept.
	assert_eq!(
		view.buttons(),
		Buttons::ATTACK | Buttons::DUCK | Buttons::from_bits_retain(1 << 30)
	);

	assert_eq!(view.impulse(), 201);
	assert_eq!((view.weapon_select(), view.weapon_subtype()), (77, 2));
	assert_eq!(view.random_seed(), 0x3fa9);
	assert_eq!(view.mouse_delta(), (-14, 3));
	assert!(format!("{view:?}").starts_with("UserCmd { command_number: 812,"));
}
