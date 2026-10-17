//! `IBotManager`, which creates bots.

use crate::NotThreadSafe;
use crate::edicts::Edict;
use crate::math::QAngle;
use crate::user_cmd::Buttons;
use sdk_raw::vcall;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

interface! {
	/// Creates and controls bots (`IBotManager`).
	#[doc(alias("IBotManager", "CPluginBotManager"))]
	pub struct BotManager(sys::IBotManager) = GameServer sdk_raw::interfaces::bot_manager::VERSION;
}

/// A command for [`BotController::run_player_move`] (`CBotCmd`): what a
/// client would send as a player's input for one tick.
#[doc(alias("CBotCmd"))]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotCommand {
	/// The command's number, which counts up by one each command.
	pub command_number: c_int,

	/// The tick the command was made for.
	pub tick_count: c_int,

	/// Where the bot looks.
	pub view_angles: QAngle,

	/// The speed forward, negative backward.
	pub forward_move: f32,

	/// The speed to the right, negative to the left.
	pub side_move: f32,

	/// The speed upward, as when swimming, negative downward.
	pub up_move: f32,

	/// The inputs held down.
	pub buttons: Buttons,

	/// An impulse command, such as 100 for the flashlight, or 0 for none.
	pub impulse: u8,

	/// The entity index of a weapon to switch to, or 0 for none.
	pub weapon_select: c_int,

	/// The weapon's subtype, as a builder's object type.
	pub weapon_subtype: c_int,

	/// The seed of the command's random spread.
	pub random_seed: c_int,

	/// How far the mouse moved across and down.
	pub mouse_delta: (i16, i16),
}

/// Runs a bot's commands and changes its items (`IBotController`), as
/// [`BotManager::controller`] gives it.
///
/// The game embeds this in the player entity, so it lives as long as the
/// player.
#[doc(alias("IBotController", "CPlayerInfo"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BotController<'s> {
	raw: NonNull<sys::IBotController>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl BotController<'_> {
	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::IBotController {
		self.raw.as_ptr()
	}

	/// Removes every weapon and wearable the bot has, and its suit too if
	/// `suit` is true.
	#[doc(alias("RemoveAllItems"))]
	pub fn remove_all_items(self, suit: bool) {
		// SAFETY: `BotManager::controller` only gives controllers of live bots,
		// and the game checks again that the player is a bot.
		unsafe { vcall!(self.as_ptr() => IBotController_RemoveAllItems(suit)) }
	}

	/// Runs one command as the bot's, as if its client had sent it: the bot
	/// moves, aims, attacks and switches weapons as the command says, over one
	/// tick.
	///
	/// This runs game code synchronously, including any run-command hooks.
	#[doc(alias("RunPlayerMove"))]
	pub fn run_player_move(self, command: &BotCommand) {
		let mut raw = sys::CBotCmd {
			// `CPlayerInfo::RunPlayerMove` copies the fields into a `CUserCmd`
			// and never calls through the command's own table.
			vtable_: std::ptr::null(),
			command_number: command.command_number,
			tick_count: command.tick_count,
			viewangles: command.view_angles.into(),
			forwardmove: command.forward_move,
			sidemove: command.side_move,
			upmove: command.up_move,
			buttons: command.buttons.bits(),
			impulse: command.impulse,
			weaponselect: command.weapon_select,
			weaponsubtype: command.weapon_subtype,
			random_seed: command.random_seed,
			mousedx: command.mouse_delta.0,
			mousedy: command.mouse_delta.1,
			hasbeenpredicted: false,
		};

		// SAFETY: As for `remove_all_items`. The command outlives the call.
		unsafe { vcall!(self.as_ptr() => IBotController_RunPlayerMove(&raw mut raw)) }
	}

	/// Switches the bot to the weapon it has of a class, such as
	/// `tf_weapon_bat`, if any.
	#[doc(alias("SetActiveWeapon"))]
	pub fn set_active_weapon(self, class: &CStr) {
		// SAFETY: As for `remove_all_items`. The game copies nothing from the
		// name after the call.
		unsafe { vcall!(self.as_ptr() => IBotController_SetActiveWeapon(class.as_ptr())) }
	}
}

impl<'s> BotManager<'s> {
	/// The controller of a bot's player, or `None` if the edict holds no
	/// player, or one that isn't a bot (a fake client).
	///
	/// TF2's own bots are fake clients too, which their own code moves each
	/// tick, so a plugin should only run commands for the bots it created.
	#[doc(alias("GetBotController"))]
	pub fn controller(self, edict: Edict<'s>) -> Option<BotController<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live. The game
		// resolves the edict's entity itself, and returns null unless it is a
		// player that is a bot.
		let controller =
			unsafe { vcall!(self.as_ptr() => IBotManager_GetBotController(edict.as_ptr())) };

		NonNull::new(controller).map(|raw| BotController {
			raw,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		})
	}

	/// Connects a bot, returning its edict, or `None` if the server is full.
	///
	/// The bot's player spawns through the game's usual connection path, which
	/// runs game code and fires game events synchronously.
	#[doc(alias("CreateBot"))]
	pub fn create_bot(self, name: &CStr) -> Option<Edict<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let edict = unsafe { vcall!(self.as_ptr() => IBotManager_CreateBot(name.as_ptr())) };

		// SAFETY: The edict table outlives `'s`.
		NonNull::new(edict).map(|edict| unsafe { Edict::from_raw(edict) })
	}
}
