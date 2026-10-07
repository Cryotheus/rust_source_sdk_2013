//! Player commands (`CUserCmd`): what a player's client sends the server each
//! time it samples the player's input, which the server runs as the player's
//! movement, attacks and weapon switches.
//!
//! [`UserCmd`] views one, such as the command `metamod_source`'s run-command
//! hooks are given as a player runs it, and [`Buttons`] holds the inputs it
//! holds down. The values in a command are the client's, so a server should
//! not trust them further than the game does.

#[cfg(test)]
#[path = "tests/user_cmd.rs"]
mod tests;

use crate::math::QAngle;
use sdk_raw::user_cmd as raw;
use std::ffi::c_int;
use std::fmt::{self, Debug, Formatter};
use std::ptr::NonNull;

bitflags::bitflags! {
	/// The inputs a player command holds down, the `IN_*` bits from
	/// `game/shared/in_buttons.h`.
	///
	/// Buttons read from a command keep every bit, including those without a
	/// name here.
	#[doc(alias("IN_"))]
	#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
	pub struct Buttons: c_int {
		/// An input no game of the SDK binds.
		#[doc(alias("IN_ALT1"))]
		const ALT1 = raw::IN_ALT1;

		/// An input no game of the SDK binds.
		#[doc(alias("IN_ALT2"))]
		const ALT2 = raw::IN_ALT2;

		/// The primary attack.
		#[doc(alias("IN_ATTACK"))]
		const ATTACK = raw::IN_ATTACK;

		/// The secondary attack.
		#[doc(alias("IN_ATTACK2"))]
		const ATTACK2 = raw::IN_ATTACK2;

		/// The special attack, such as TF2's `+attack3`.
		#[doc(alias("IN_ATTACK3"))]
		const ATTACK3 = raw::IN_ATTACK3;

		/// Moving back.
		#[doc(alias("IN_BACK"))]
		const BACK = raw::IN_BACK;

		/// An input no game of the SDK binds.
		#[doc(alias("IN_BULLRUSH"))]
		const BULLRUSH = raw::IN_BULLRUSH;

		/// Cancelling, which no client of the SDK's games sends.
		#[doc(alias("IN_CANCEL"))]
		const CANCEL = raw::IN_CANCEL;

		/// Ducking.
		#[doc(alias("IN_DUCK"))]
		const DUCK = raw::IN_DUCK;

		/// Moving forward.
		#[doc(alias("IN_FORWARD"))]
		const FORWARD = raw::IN_FORWARD;

		/// The first grenade, which TF2 does not bind.
		#[doc(alias("IN_GRENADE1"))]
		const GRENADE1 = raw::IN_GRENADE1;

		/// The second grenade, which TF2 does not bind.
		#[doc(alias("IN_GRENADE2"))]
		const GRENADE2 = raw::IN_GRENADE2;

		/// Jumping.
		#[doc(alias("IN_JUMP"))]
		const JUMP = raw::IN_JUMP;

		/// Turning left, with the keyboard.
		#[doc(alias("IN_LEFT"))]
		const LEFT = raw::IN_LEFT;

		/// Strafing left.
		#[doc(alias("IN_MOVELEFT"))]
		const MOVE_LEFT = raw::IN_MOVELEFT;

		/// Strafing right.
		#[doc(alias("IN_MOVERIGHT"))]
		const MOVE_RIGHT = raw::IN_MOVERIGHT;

		/// Reloading.
		#[doc(alias("IN_RELOAD"))]
		const RELOAD = raw::IN_RELOAD;

		/// Turning right, with the keyboard.
		#[doc(alias("IN_RIGHT"))]
		const RIGHT = raw::IN_RIGHT;

		/// Running.
		#[doc(alias("IN_RUN"))]
		const RUN = raw::IN_RUN;

		/// Showing the scoreboard.
		#[doc(alias("IN_SCORE"))]
		const SCORE = raw::IN_SCORE;

		/// The speed key, held to sprint where a game has sprinting.
		#[doc(alias("IN_SPEED"))]
		const SPEED = raw::IN_SPEED;

		/// Using what the player looks at, which TF2 does not bind.
		#[doc(alias("IN_USE"))]
		const USE = raw::IN_USE;

		/// Walking.
		#[doc(alias("IN_WALK"))]
		const WALK = raw::IN_WALK;

		/// A bit weapons define.
		#[doc(alias("IN_WEAPON1"))]
		const WEAPON1 = raw::IN_WEAPON1;

		/// A bit weapons define.
		#[doc(alias("IN_WEAPON2"))]
		const WEAPON2 = raw::IN_WEAPON2;

		/// Zooming the HUD.
		#[doc(alias("IN_ZOOM"))]
		const ZOOM = raw::IN_ZOOM;

		const _ = !0;
	}
}

/// A player command, as the server runs it: the view angles, movement,
/// buttons and weapon selection the player's client sampled, which the
/// setters change for the game's code that runs after.
///
/// The game runs a command's movement from its angles and moves, then its
/// buttons' attacks, and switches to its selected weapon first. Changing
/// these only changes what the server runs: the client predicted the command
/// as it sent it, and corrects to the server's results.
#[doc(alias("CUserCmd"))]
#[repr(transparent)]
pub struct UserCmd(sys::CUserCmd);

impl UserCmd {
	/// The command as the engine stores it.
	pub const fn as_raw(&self) -> &sys::CUserCmd {
		&self.0
	}

	/// The command as the engine stores it, to change.
	pub const fn as_raw_mut(&mut self) -> &mut sys::CUserCmd {
		&mut self.0
	}

	/// The buttons the command holds down (`buttons`).
	pub const fn buttons(&self) -> Buttons {
		Buttons::from_bits_retain(self.0.buttons)
	}

	/// The number of the command, which counts the commands the client
	/// created (`command_number`).
	pub const fn command_number(&self) -> c_int {
		self.0.command_number
	}

	/// How fast the command moves forward, or back if negative, in units per
	/// second, which the game caps at the player's speed (`forwardmove`).
	#[doc(alias("forwardmove"))]
	pub const fn forward_move(&self) -> f32 {
		self.0.forwardmove
	}

	/// Views the command at `command`.
	///
	/// # Safety
	///
	/// `command` must point to a live command, valid for reads and writes
	/// for `'a`, which nothing else accesses meanwhile.
	pub const unsafe fn from_raw_mut<'a>(command: NonNull<sys::CUserCmd>) -> &'a mut Self {
		// SAFETY: `Self` is a transparent wrapper of the command, which the
		// caller promises is live and exclusive for `'a`.
		unsafe { command.cast::<Self>().as_mut() }
	}

	/// The command's impulse, a number that runs a command of the game, such
	/// as 101 for `impulse 101`, or 0 for none (`impulse`).
	pub const fn impulse(&self) -> u8 {
		self.0.impulse
	}

	/// How far the client's mouse moved, horizontally and vertically, while
	/// the client created the command (`mousedx`, `mousedy`).
	#[doc(alias("mousedx", "mousedy"))]
	pub const fn mouse_delta(&self) -> (i16, i16) {
		(self.0.mousedx, self.0.mousedy)
	}

	/// The seed of the random numbers the client and server share in the
	/// command, such as for bullet spread (`random_seed`).
	pub const fn random_seed(&self) -> c_int {
		self.0.random_seed
	}

	/// Holds `buttons` down instead.
	pub const fn set_buttons(&mut self, buttons: Buttons) {
		self.0.buttons = buttons.bits();
	}

	/// Moves forward at `speed` instead, or back if negative.
	pub const fn set_forward_move(&mut self, speed: f32) {
		self.0.forwardmove = speed;
	}

	/// Runs the impulse `impulse` instead, or none for 0.
	pub const fn set_impulse(&mut self, impulse: u8) {
		self.0.impulse = impulse;
	}

	/// Strafes right at `speed` instead, or left if negative.
	pub const fn set_side_move(&mut self, speed: f32) {
		self.0.sidemove = speed;
	}

	/// Moves up at `speed` instead, such as while swimming, or down if
	/// negative.
	pub const fn set_up_move(&mut self, speed: f32) {
		self.0.upmove = speed;
	}

	/// Looks along `angles` instead.
	pub fn set_view_angles(&mut self, angles: QAngle) {
		self.0.viewangles = angles.into();
	}

	/// Selects the weapon of the entity index `index` instead, and its
	/// subtype `subtype`, or no weapon for index 0.
	pub const fn set_weapon_select(&mut self, index: c_int, subtype: c_int) {
		self.0.weaponselect = index;
		self.0.weaponsubtype = subtype;
	}

	/// How fast the command strafes right, or left if negative, in units per
	/// second (`sidemove`).
	#[doc(alias("sidemove"))]
	pub const fn side_move(&self) -> f32 {
		self.0.sidemove
	}

	/// The tick the client created the command in (`tick_count`).
	pub const fn tick_count(&self) -> c_int {
		self.0.tick_count
	}

	/// How fast the command moves up, such as while swimming, or down if
	/// negative, in units per second (`upmove`).
	#[doc(alias("upmove"))]
	pub const fn up_move(&self) -> f32 {
		self.0.upmove
	}

	/// The angles the command looks along (`viewangles`).
	#[doc(alias("viewangles"))]
	pub fn view_angles(&self) -> QAngle {
		self.0.viewangles.into()
	}

	/// The entity index of the weapon the command switches to, or 0 for none
	/// (`weaponselect`). The game switches only to a weapon the player
	/// carries.
	#[doc(alias("weaponselect"))]
	pub const fn weapon_select(&self) -> c_int {
		self.0.weaponselect
	}

	/// The subtype of the weapon the command switches to, which tells weapons
	/// of one classname apart, such as TF2's builders of each building
	/// (`weaponsubtype`).
	#[doc(alias("weaponsubtype"))]
	pub const fn weapon_subtype(&self) -> c_int {
		self.0.weaponsubtype
	}
}

impl Debug for UserCmd {
	fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
		f.debug_struct("UserCmd")
			.field("command_number", &self.command_number())
			.field("tick_count", &self.tick_count())
			.field("view_angles", &self.view_angles())
			.field("forward_move", &self.forward_move())
			.field("side_move", &self.side_move())
			.field("up_move", &self.up_move())
			.field("buttons", &self.buttons())
			.field("impulse", &self.impulse())
			.field("weapon_select", &self.weapon_select())
			.field("weapon_subtype", &self.weapon_subtype())
			.finish_non_exhaustive()
	}
}
