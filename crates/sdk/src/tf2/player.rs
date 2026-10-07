//! TF2's players: their class, team and state, their view, what they see of
//! the HUD and of the game's menus, their custom models, and the game's own
//! ways to change these.
//!
//! [`TfPlayer`] wraps a player within one engine callback. It reads the
//! player's networked variables, and changes the player through the members
//! `CTFPlayer` exposes to VScript, whose native bindings are found by name and
//! checked against the SDK's signatures before each call, or through the
//! player's vtable, at the slots [`sdk_raw::tf2::player`] checks.
//!
//! Other parts of a player have modules of their own:
//! [`weapons`](crate::tf2::weapons) and [`ammo`](crate::tf2::ammo) for what
//! they carry, [`conditions`](crate::tf2::conditions),
//! [`observer`](crate::tf2::observer) for spectating, and
//! [`respawn`](crate::tf2::respawn).
//!
//! # Unverified
//!
//! No running server has tested this module yet. Its vtable slots agree with
//! SourceMod's TF2 gamedata, and its bindings are checked by name and
//! signature as they are called.

#[cfg(test)]
#[path = "../tests/tf2/player.rs"]
mod tests;

use crate::datatables::{NetProp, NetPropError};
use crate::entities::{Entity, EntityHandle};
use crate::math::{QAngle, Vector};
use crate::tf2::PlayerClass;
use crate::tf2::observer::VIEW_OFFSET;
use crate::tf2::ragdolls::TfRagdoll;
use crate::tf2::script_binding::{self as binding, BindingError};
use crate::tf2::teams::Team;
use crate::user_messages::messages::VguiMenu;
use crate::user_messages::{self, Recipients, UserMessageError};
use crate::{Game, InterfaceError, Server};
use sdk_raw::tf2::player as raw;
use sdk_raw::tf2::script_binding::{BOOL, INT, boolean, int, qangle, vector};
use sdk_raw::{vcall, vcall_by_value};
use std::ffi::{CStr, c_int};

/// The script class that declares the members [`TfPlayer`] calls.
const SCRIPT_CLASS: &CStr = c"CTFPlayer";

/// Whether a player's view is forced into third person, as taunts force it
/// (`m_nForceTauntCam`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ForcedTauntCam {
	/// The player chooses their own view.
	#[default]
	Off,

	/// Third person while the player is alive.
	WhileAlive,

	/// Third person, even while the player is dead.
	Always,
}

impl ForcedTauntCam {
	/// The value of `m_nForceTauntCam`.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Off => 0,
			Self::WhileAlive => 1,
			Self::Always => 2,
		}
	}
}

bitflags::bitflags! {
	/// The parts of the HUD a player's `m_iHideHUD` hides (`HIDEHUD_*`).
	///
	/// Clients receive only the low [`HIDEHUD_BITCOUNT`](raw::HIDEHUD_BITCOUNT)
	/// bits, which the named flags cover. Others read back as set, but no
	/// client sees them.
	#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
	pub struct HideHud: u32 {
		/// The whole HUD.
		#[doc(alias("HIDEHUD_ALL"))]
		const ALL = raw::HIDEHUD_ALL as u32;

		/// The bonus progress display of other Source games.
		#[doc(alias("HIDEHUD_BONUS_PROGRESS"))]
		const BONUS_PROGRESS = raw::HIDEHUD_BONUS_PROGRESS as u32;

		/// The Engineer's building status.
		#[doc(alias("HIDEHUD_BUILDING_STATUS"))]
		const BUILDING_STATUS = raw::HIDEHUD_BUILDING_STATUS as u32;

		/// Chat, and the other communication elements, such as the voice
		/// icons.
		#[doc(alias("HIDEHUD_CHAT"))]
		const CHAT = raw::HIDEHUD_CHAT as u32;

		/// The item effect meters, such as the Spy's cloak.
		#[doc(alias("HIDEHUD_CLOAK_AND_FEIGN"))]
		const CLOAK_AND_FEIGN = raw::HIDEHUD_CLOAK_AND_FEIGN as u32;

		/// The crosshair.
		#[doc(alias("HIDEHUD_CROSSHAIR"))]
		const CROSSHAIR = raw::HIDEHUD_CROSSHAIR as u32;

		/// The flashlight meter of other Source games.
		#[doc(alias("HIDEHUD_FLASHLIGHT"))]
		const FLASHLIGHT = raw::HIDEHUD_FLASHLIGHT as u32;

		/// The health display.
		#[doc(alias("HIDEHUD_HEALTH"))]
		const HEALTH = raw::HIDEHUD_HEALTH as u32;

		/// What a player in a vehicle should not see.
		#[doc(alias("HIDEHUD_INVEHICLE"))]
		const IN_VEHICLE = raw::HIDEHUD_INVEHICLE as u32;

		/// The match status, such as the round timer and the team status at
		/// the top of the screen.
		#[doc(alias("HIDEHUD_MATCH_STATUS"))]
		const MATCH_STATUS = raw::HIDEHUD_MATCH_STATUS as u32;

		/// The Engineer's metal display.
		#[doc(alias("HIDEHUD_METAL"))]
		const METAL = raw::HIDEHUD_METAL as u32;

		/// Miscellaneous status elements, such as the pickup history and the
		/// kill feed.
		#[doc(alias("HIDEHUD_MISCSTATUS"))]
		const MISC_STATUS = raw::HIDEHUD_MISCSTATUS as u32;

		/// What a player without Half-Life 2's HEV suit should not see.
		#[doc(alias("HIDEHUD_NEEDSUIT"))]
		const NEED_SUIT = raw::HIDEHUD_NEEDSUIT as u32;

		/// The Demoman's sticky bomb count and shield charge meter.
		#[doc(alias("HIDEHUD_PIPES_AND_CHARGE"))]
		const PIPES_AND_CHARGE = raw::HIDEHUD_PIPES_AND_CHARGE as u32;

		/// What a dead player should not see.
		#[doc(alias("HIDEHUD_PLAYERDEAD"))]
		const PLAYER_DEAD = raw::HIDEHUD_PLAYERDEAD as u32;

		/// The target ID: the name and health of the player under the
		/// crosshair.
		#[doc(alias("HIDEHUD_TARGET_ID"))]
		const TARGET_ID = raw::HIDEHUD_TARGET_ID as u32;

		/// The vehicle crosshair of other Source games.
		#[doc(alias("HIDEHUD_VEHICLE_CROSSHAIR"))]
		const VEHICLE_CROSSHAIR = raw::HIDEHUD_VEHICLE_CROSSHAIR as u32;

		/// The ammo count and the weapon selection.
		#[doc(alias("HIDEHUD_WEAPONSELECTION"))]
		const WEAPON_SELECTION = raw::HIDEHUD_WEAPONSELECTION as u32;
	}
}

/// Why a [`TfPlayer`] operation failed.
#[derive(Debug, thiserror::Error)]
pub enum PlayerError {
	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// The player is marked for deletion.
	#[error("the player is marked for deletion")]
	MarkedForDeletion,

	/// A networked variable could not be read or written.
	#[error(transparent)]
	NetProp(#[from] NetPropError),

	/// A vector, angle or time given has a component that is NaN or infinite.
	#[error("a value given is NaN or infinite")]
	NonFinite,

	/// The player is not alive.
	#[error("the player is not alive")]
	NotAlive,

	/// The player has no edict, so no client knows it.
	#[error("the player is not networked")]
	NotNetworked,

	/// The player is on neither RED nor BLU, which have the class menus.
	#[error("the player is not on a playing team")]
	NotPlaying,

	/// The entity is not a TF2 player, or the server does not run TF2.
	#[error("player operations require a TF2 player")]
	NotTfPlayer,

	/// The native binding of the member reported failure.
	#[error("the native method rejected its arguments")]
	Rejected,

	/// The player's `m_iClass` or `m_iDesiredPlayerClass` holds neither
	/// `TF_CLASS_UNDEFINED` nor a playable class, as scripts can make it.
	#[error("the player's class {0} is not one of TF2's playable classes")]
	UnknownClass(c_int),

	/// The player's `m_nPlayerState` is not one of TF2's player states.
	#[error("the player's state {0} is not one of TF2's")]
	UnknownState(c_int),

	/// The player's `m_iTeamNum` is not one of TF2's teams.
	#[error("the player's team {0} is not one of TF2's")]
	UnknownTeam(c_int),

	/// The player's script class descriptors lack the member, or its
	/// signature differs from the SDK's.
	#[error("the game does not expose the expected native method")]
	UnsupportedMethod,

	/// The game's menu could not be sent to the player.
	#[error(transparent)]
	UserMessage(#[from] UserMessageError),
}

impl From<BindingError> for PlayerError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// Where a TF2 player is in the game's handling of them (`m_nPlayerState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerState {
	/// Playing, alive or dead, until they start to observe
	/// (`TF_STATE_ACTIVE`).
	#[doc(alias("TF_STATE_ACTIVE"))]
	Active,

	/// Just joined the server, and has chosen no team yet
	/// (`TF_STATE_WELCOME`).
	#[doc(alias("TF_STATE_WELCOME"))]
	Welcome,

	/// Observing, as spectators, unassigned players, and dead players once
	/// the death cam ends, do (`TF_STATE_OBSERVER`).
	#[doc(alias("TF_STATE_OBSERVER"))]
	Observer,

	/// Dying: from death until the player starts to observe
	/// (`TF_STATE_DYING`).
	#[doc(alias("TF_STATE_DYING"))]
	Dying,
}

impl PlayerState {
	/// The state TF2 numbers `raw`, or `None` if it numbers none so.
	pub const fn from_raw(raw: c_int) -> Option<Self> {
		match raw {
			raw::TF_STATE_ACTIVE => Some(Self::Active),
			raw::TF_STATE_WELCOME => Some(Self::Welcome),
			raw::TF_STATE_OBSERVER => Some(Self::Observer),
			raw::TF_STATE_DYING => Some(Self::Dying),
			_ => None,
		}
	}

	/// TF2's number for the state.
	pub const fn to_raw(self) -> c_int {
		match self {
			Self::Active => raw::TF_STATE_ACTIVE,
			Self::Welcome => raw::TF_STATE_WELCOME,
			Self::Observer => raw::TF_STATE_OBSERVER,
			Self::Dying => raw::TF_STATE_DYING,
		}
	}
}

/// A TF2 player or bot, within one engine callback.
///
/// Keep the player's [`EntityHandle`] across callbacks, and wrap it again in
/// each.
#[derive(Debug, Clone, Copy)]
pub struct TfPlayer<'s> {
	server: Server<'s>,
	player: Entity<'s>,
}

impl<'s> TfPlayer<'s> {
	/// Wraps a player, or returns [`PlayerError::NotTfPlayer`] unless the
	/// server runs TF2 and `player`'s datamaps include `CTFPlayer`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, PlayerError> {
		if server.game() != Game::TeamFortress2 || !player.has_data_map_class(c"CTFPlayer") {
			return Err(PlayerError::NotTfPlayer);
		}

		Ok(Self { server, player })
	}

	/// Hides more parts of the player's HUD, keeping those hidden already
	/// (`AddHudHideFlags`).
	#[doc(alias("AddHudHideFlags", "m_iHideHUD"))]
	pub fn add_hud_hide_flags(self, flags: HideHud) -> Result<(), PlayerError> {
		self.call_with_flags(c"AddHudHideFlags", flags)
	}

	/// Pushes the player, adding `impulse` to their velocity as an explosion's
	/// knockback does (`ApplyAbsVelocityImpulse`), in units per second.
	///
	/// TF2 scales the push first: it doubles it for a player shrunk by
	/// `TF_COND_HALLOWEEN_TINY`, scales it by a scoped Sniper's
	/// `mult_aiming_knockback_resistance`, and its horizontal part by 1.5 for
	/// a player with a parachute open (or by 0 for one of Mann vs. Machine's
	/// robots). Fails with [`PlayerError::NonFinite`] for an impulse that is
	/// not finite, before the game is called.
	#[doc(alias("ApplyAbsVelocityImpulse"))]
	pub fn apply_impulse(self, impulse: Vector) -> Result<(), PlayerError> {
		if !impulse.is_finite() {
			return Err(PlayerError::NonFinite);
		}

		let impulse = sys::Vector::from(impulse);

		// SAFETY: The checked member adds the finite impulse, which it only
		// reads during the call, to the player's velocity.
		unsafe {
			self.call(
				c"ApplyAbsVelocityImpulse",
				&mut [vector(&impulse)],
				binding::VOID,
			)
		}?;

		Ok(())
	}

	/// Calls one of the native members `CTFPlayer` exposes to VScript, for a
	/// player not marked for deletion.
	///
	/// # Safety
	///
	/// As for [`binding::call`]: the member must accept this player and these
	/// arguments, which must stay valid for the call.
	unsafe fn call(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
		result_type: sys::ScriptDataType_t,
	) -> Result<sys::ScriptVariant_t, PlayerError> {
		self.check_live()?;

		// SAFETY: The caller vouches for the member and its arguments.
		Ok(unsafe { binding::call(self.player, SCRIPT_CLASS, name, arguments, result_type) }?)
	}

	/// Calls a member that takes one boolean and returns nothing.
	///
	/// # Safety
	///
	/// As for [`Self::call`].
	unsafe fn call_with_bool(self, name: &CStr, value: bool) -> Result<(), PlayerError> {
		// SAFETY: The caller vouches for the member.
		unsafe { self.call(name, &mut [boolean(value)], binding::VOID) }?;

		Ok(())
	}

	/// Calls one of the members that write `m_iHideHUD` with `flags`.
	fn call_with_flags(self, name: &CStr, flags: HideHud) -> Result<(), PlayerError> {
		// SAFETY: The HUD flag members only compute and store the player's
		// `m_iHideHUD`, of which clients read the bits they know.
		unsafe { self.call(name, &mut [int(flags.bits() as c_int)], binding::VOID) }?;

		Ok(())
	}

	/// Calls a member that takes a model's path and returns nothing.
	fn call_with_model(self, name: &CStr, model: &CStr) -> Result<(), PlayerError> {
		// SAFETY: The custom model members precache the model, copying its
		// path, which they read only during the call, into the string pool.
		// Updating the player's model frees nothing.
		unsafe { self.call(name, &mut [binding::string(model)], binding::VOID) }?;

		Ok(())
	}

	/// Calls a member that returns one float.
	///
	/// # Safety
	///
	/// As for [`Self::call`].
	unsafe fn call_for_float(self, name: &CStr) -> Result<f32, PlayerError> {
		// SAFETY: The caller vouches for the member.
		let result = unsafe { self.call(name, &mut [], binding::FLOAT) }?;

		// SAFETY: The checked return type selects the float member.
		Ok(unsafe { result.__bindgen_anon_1.m_float })
	}

	/// Fails with [`PlayerError::MarkedForDeletion`] for a player marked for
	/// deletion.
	fn check_live(self) -> Result<(), PlayerError> {
		if self.player.is_marked_for_deletion() {
			Err(PlayerError::MarkedForDeletion)
		} else {
			Ok(())
		}
	}

	/// The player's class (`m_iClass`), or `None` until they first spawn as
	/// one (`TF_CLASS_UNDEFINED`).
	///
	/// Fails with [`PlayerError::UnknownClass`] for any other value, such as
	/// the civilian a script can set.
	#[doc(alias("m_iClass", "GetPlayerClass"))]
	pub fn class(self) -> Result<Option<PlayerClass>, PlayerError> {
		self.read_class(c"m_iClass")
	}

	/// Clears the player's custom model, its offset and its rotation, so they
	/// show their class's model again (`SetCustomModel` with an empty path).
	#[doc(alias("SetCustomModel"))]
	pub fn clear_custom_model(self) -> Result<(), PlayerError> {
		self.call_with_model(c"SetCustomModel", c"")
	}

	/// Clears the rotation of the player's custom model
	/// (`ClearCustomModelRotation`).
	#[doc(alias("ClearCustomModelRotation"))]
	pub fn clear_custom_model_rotation(self) -> Result<(), PlayerError> {
		// SAFETY: The member resets the stored rotation, and invalidates the
		// player's cached angles.
		unsafe { self.call(c"ClearCustomModelRotation", &mut [], binding::VOID) }?;

		Ok(())
	}

	/// Kills the player as the `kill` and `explode` commands do, through the
	/// game's `CTFPlayer::CommitSuicide`, and returns whether they died.
	///
	/// With `explode`, the player bursts into gibs. TF2 refuses the suicide of
	/// a player who is not alive, who has no class or is not in
	/// [`PlayerState::Active`], who is a ghost or in a kart, while the match
	/// summary shows, and, without `force`, of a player on the losing team
	/// after a round is won.
	///
	/// The death runs the game's, other plugins', and this plugin's own death
	/// callbacks synchronously, before this returns, such as hooks of the
	/// player's `Event_Killed` and listeners of `player_death`. They must keep
	/// to the contract of [`Server::new`].
	#[doc(alias("CommitSuicide"))]
	pub fn commit_suicide(self, explode: bool, force: bool) -> Result<bool, PlayerError> {
		self.check_live()?;

		if !self.player.is_alive() {
			return Ok(false);
		}

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: `new` found `CTFPlayer` in the player's datamaps, so it is
		// one, whose entity base `sdk_raw::tf2` asserts is at offset zero, and
		// whose generated TF2 vtable has this entry under the target's ABI, at
		// the slot `sdk_raw::tf2::player` checks. The callback keeps the player
		// allocated, and the callbacks the death runs are bound by
		// `Server::new`'s contract, which lets them free entities only through
		// deferred deletion.
		unsafe {
			vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_CommitSuicide(explode, force))
		};

		Ok(!self.player.is_alive())
	}

	/// The class the player chose to play next (`m_iDesiredPlayerClass`),
	/// which they spawn as, or `None` if they have chosen none.
	///
	/// Fails with [`PlayerError::UnknownClass`] for a value that is neither
	/// `TF_CLASS_UNDEFINED` nor a playable class.
	#[doc(alias("m_iDesiredPlayerClass", "GetDesiredPlayerClassIndex"))]
	pub fn desired_class(self) -> Result<Option<PlayerClass>, PlayerError> {
		self.read_class(c"m_iDesiredPlayerClass")
	}

	/// The angles the player looks along (`EyeAngles`): their view angles, in
	/// the world's frame even while they move with a parent entity, whose frame
	/// the game keeps their view angles in then.
	#[doc(alias("EyeAngles"))]
	pub fn eye_angles(self) -> Result<QAngle, PlayerError> {
		self.check_live()?;

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: As for `commit_suicide`, at the slot `sdk_raw::tf2::player`
		// checks. The method only reads the player and its parent, and returns
		// a reference to angles the player holds, or to a static, copied before
		// anything can change them.
		let angles = unsafe {
			vcall!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_EyeAngles()).read()
		};

		Ok(angles.into())
	}

	/// Where the player sees from (`EyePosition`): their origin raised by their
	/// [view offset](Self::view_offset).
	#[doc(alias("EyePosition"))]
	pub fn eye_position(self) -> Result<Vector, PlayerError> {
		self.check_live()?;

		let player = self.player.as_ptr().cast::<sys::CTFPlayer>();

		// SAFETY: As for `commit_suicide`, at the slot `sdk_raw::tf2::player`
		// checks, whose generated entry returns a `Vector` by value in one of
		// the shapes `vcall_by_value!` calls. The method only reads the player.
		let position = unsafe {
			vcall_by_value!(player as sys::CTFPlayer__bindgen_vtable => CTFPlayer_EyePosition() -> sys::Vector)
		};

		Ok(position.into())
	}

	/// Moves the player to `team` as the game's own team changes do
	/// (`ForceChangeTeam`), whatever the team limits and balance allow.
	///
	/// TF2 removes the player's buildings and projectiles, and leaves a
	/// living player alive, on the new team, unless `full_team_switch` is
	/// false in highlander mode, which kills them and clears their class. A
	/// player moved to [`Team::Spectator`] loses their weapons, and one moved
	/// to [`Team::Spectator`] or [`Team::Unassigned`] starts to observe.
	/// Without `full_team_switch`, the player's dominations end too, as they
	/// do when a player changes teams themselves; the game passes it when it
	/// swaps whole teams.
	///
	/// TF2 changes nothing for a player already on the team, nor for one in a
	/// duel or coaching another, and outside community game modes its game
	/// rules can assign another team, as Mann vs. Machine does. The change
	/// runs the game's, other plugins', and this plugin's own callbacks
	/// synchronously, such as `metamod_source`'s `team_hooks` and listeners of
	/// `player_team`, and they must keep to the contract of [`Server::new`].
	#[doc(alias("ForceChangeTeam", "ChangeClientTeam"))]
	pub fn force_change_team(self, team: Team, full_team_switch: bool) -> Result<(), PlayerError> {
		// SAFETY: The checked member accepts every global team, and frees the
		// entities it removes, and those its callbacks free, only through
		// deferred deletion (`Server::new`'s contract).
		unsafe {
			self.call(
				c"ForceChangeTeam",
				&mut [int(team.to_raw()), boolean(full_team_switch)],
				binding::VOID,
			)
		}?;

		Ok(())
	}

	/// Hides both teams' class menus from the player, if shown.
	pub fn hide_class_menu(self) -> Result<(), PlayerError> {
		self.send_panel(raw::PANEL_CLASS_RED, false)?;
		self.send_panel(raw::PANEL_CLASS_BLUE, false)
	}

	/// Hides the team menu from the player, if shown.
	pub fn hide_team_menu(self) -> Result<(), PlayerError> {
		self.send_panel(raw::PANEL_TEAM, false)
	}

	/// The parts of the HUD hidden from the player (`GetHudHideFlags`).
	#[doc(alias("GetHudHideFlags", "m_iHideHUD"))]
	pub fn hud_hide_flags(self) -> Result<HideHud, PlayerError> {
		// SAFETY: The member returns the player's `m_iHideHUD`.
		let result = unsafe { self.call(c"GetHudHideFlags", &mut [], INT) }?;

		// SAFETY: The checked return type selects the integer member.
		let flags = unsafe { result.__bindgen_anon_1.m_int };

		Ok(HideHud::from_bits_retain(flags as u32))
	}

	/// Whether the player has called for a Medic in the last few seconds
	/// (`IsCallingForMedic`).
	#[doc(alias("IsCallingForMedic"))]
	pub fn is_calling_for_medic(self) -> Result<bool, PlayerError> {
		// SAFETY: The member compares the time of the player's last call with
		// the current time.
		let result = unsafe { self.call(c"IsCallingForMedic", &mut [], BOOL) }?;

		// SAFETY: The checked return type selects the boolean member.
		Ok(unsafe { result.__bindgen_anon_1.m_bool })
	}

	/// Resolves one of the player's networked variables.
	fn net_prop(self, name: &CStr) -> Result<NetProp<'s>, PlayerError> {
		Ok(self
			.server
			.server_game_dll()?
			.entity_net_prop(self.player, name)?)
	}

	/// The game time from which the player may change their class again
	/// (`GetNextChangeClassTime`), as the game's current time counts it.
	#[doc(alias("GetNextChangeClassTime", "m_flNextChangeClassTime"))]
	pub fn next_change_class_time(self) -> Result<f32, PlayerError> {
		// SAFETY: The member returns a field of the player.
		unsafe { self.call_for_float(c"GetNextChangeClassTime") }
	}

	/// The game time from which the player may change their team again
	/// (`GetNextChangeTeamTime`), as the game's current time counts it.
	#[doc(alias("GetNextChangeTeamTime", "m_flNextChangeTeamTime"))]
	pub fn next_change_team_time(self) -> Result<f32, PlayerError> {
		// SAFETY: The member returns a field of the player.
		unsafe { self.call_for_float(c"GetNextChangeTeamTime") }
	}

	/// The player.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// The player's latest `tf_ragdoll` (`m_hRagdoll`), from which clients
	/// make the ragdoll or gibs of their last death, or `None` if they have
	/// none, or it is marked for deletion.
	#[doc(alias("m_hRagdoll"))]
	pub fn ragdoll(self) -> Result<Option<TfRagdoll<'s>>, PlayerError> {
		let handle = self.net_prop(c"m_hRagdoll")?.get_handle(self.player)?;

		Ok(self
			.server
			.server_tools()?
			.entity_by_handle(handle)
			.filter(|ragdoll| !ragdoll.is_marked_for_deletion())
			.and_then(|ragdoll| TfRagdoll::new(self.server, ragdoll).ok()))
	}

	/// Reads a class variable, which holds `TF_CLASS_UNDEFINED` or a playable
	/// class.
	fn read_class(self, name: &CStr) -> Result<Option<PlayerClass>, PlayerError> {
		let raw = self.net_prop(name)?.get::<c_int>(self.player)?;

		match raw {
			0 => Ok(None),

			raw => PlayerClass::from_raw(raw)
				.map(Some)
				.ok_or(PlayerError::UnknownClass(raw)),
		}
	}

	/// Resupplies the living player as a resupply cabinet does
	/// (`Regenerate`).
	///
	/// TF2 sets the player up as their class afresh, which hands out their
	/// loadout's weapons and cosmetics again, keeps any overheal, and, with
	/// `refill_health_and_ammo`, refills their health and ammo and ends the
	/// conditions a resupply cabinet cures, such as burning and bleeding.
	/// Without it, their health and ammo stay as they are.
	///
	/// Handing out items creates entities, which runs entity-creation and
	/// spawn callbacks synchronously, before this returns: the game's, other
	/// plugins', and this plugin's own. They must keep to the contract of
	/// [`Server::new`]. Fails with [`PlayerError::NotAlive`] before the game is
	/// called for a player who is not alive.
	#[doc(alias("Regenerate"))]
	pub fn regenerate(self, refill_health_and_ammo: bool) -> Result<(), PlayerError> {
		self.check_live()?;

		if !self.player.is_alive() {
			return Err(PlayerError::NotAlive);
		}

		// SAFETY: The checked member resupplies this living player. It frees
		// the items it replaces, and its callbacks free entities, only through
		// deferred deletion (`Server::new`'s contract).
		unsafe { self.call_with_bool(c"Regenerate", refill_health_and_ammo) }
	}

	/// Respawns the player afresh, dead or alive, as their desired class, with
	/// their health, ammo and items as a resupply leaves them
	/// (`ForceRegenerateAndRespawn`).
	///
	/// This goes through the player's `ForceRespawn`, as
	/// [`force_respawn`](crate::tf2::respawn::force_respawn) does, and does
	/// nothing in the same cases.
	#[doc(alias("ForceRegenerateAndRespawn"))]
	pub fn regenerate_and_respawn(self) -> Result<(), PlayerError> {
		// SAFETY: The checked member respawns the player as `force_respawn`
		// does, whose spawn callbacks are bound by `Server::new`'s contract.
		unsafe { self.call(c"ForceRegenerateAndRespawn", &mut [], binding::VOID) }?;

		Ok(())
	}

	/// Removes the player's buildings, as changing class does
	/// (`RemoveAllObjects`), or, with `explode`, destroys them, as the
	/// Engineer's destruction PDA does.
	///
	/// TF2 fires `object_removed` for each, then, with `explode`, also
	/// `object_destroyed`, and the listeners of these run before this returns.
	#[doc(alias("RemoveAllObjects"))]
	pub fn remove_all_objects(self, explode: bool) -> Result<(), PlayerError> {
		// SAFETY: The checked member removes the player's buildings through
		// deferred deletion, and its callbacks free entities only so too.
		unsafe { self.call_with_bool(c"RemoveAllObjects", explode) }
	}

	/// Shows parts of the player's HUD again (`RemoveHudHideFlags`).
	#[doc(alias("RemoveHudHideFlags", "m_iHideHUD"))]
	pub fn remove_hud_hide_flags(self, flags: HideHud) -> Result<(), PlayerError> {
		self.call_with_flags(c"RemoveHudHideFlags", flags)
	}

	/// Removes the player's [ragdoll](Self::ragdoll), so clients make none of
	/// their last death, or none from now on if they made it already, and
	/// returns whether there was one to remove.
	///
	/// The `tf_ragdoll` is removed as the game removes it, with deferred
	/// deletion, and `m_hRagdoll` cleared, which the game checks before each
	/// use. Remove it in the frame of the death to keep clients from making
	/// its ragdoll at all.
	#[doc(alias("m_hRagdoll"))]
	pub fn remove_ragdoll(self) -> Result<bool, PlayerError> {
		self.check_live()?;

		let prop = self.net_prop(c"m_hRagdoll")?;
		let engine = self.server.valve_engine()?;
		let tools = self.server.server_tools()?;
		let Some(ragdoll) = self.ragdoll()? else {
			return Ok(false);
		};

		if tools.remove(ragdoll.entity()).is_err() {
			return Ok(false);
		}

		// SAFETY: A player has no ragdoll until their first death, and the
		// game checks it for one before each use.
		unsafe { prop.set_handle(engine, self.player, EntityHandle::INVALID) }?;

		Ok(true)
	}

	/// Sends the player the game's message showing or hiding a panel.
	fn send_panel(self, name: &CStr, show: bool) -> Result<(), PlayerError> {
		self.check_live()?;

		let edict = self.player.edict().ok_or(PlayerError::NotNetworked)?;
		let menu = VguiMenu {
			name,
			show,
			keys: &[],
		};

		user_messages::send(self.server, &Recipients::player(edict).reliable(), &menu)?;

		Ok(())
	}

	/// Changes the player's class at once, as scripts do (`SetPlayerClass`),
	/// without respawning them.
	///
	/// TF2 sets the player up as the class: their speed, maximum health and
	/// abilities follow it, and their weapons are told of the change. Their
	/// model, weapons and health stay as they are until they next spawn or
	/// resupply, as do their [desired class](Self::desired_class), which they
	/// spawn as, and their custom model, which this clears.
	#[doc(alias("SetPlayerClass", "m_iClass"))]
	pub fn set_class(self, class: PlayerClass) -> Result<(), PlayerError> {
		// SAFETY: The checked member sets the player up as a playable class,
		// as the game does itself, and frees nothing.
		unsafe { self.call(c"SetPlayerClass", &mut [int(class.to_raw())], binding::VOID) }?;

		Ok(())
	}

	/// Marks the class menu as open on the player's client, or closed, as
	/// the client reports it, by running the client's `menuopen` or
	/// `menuclosed` command for them.
	///
	/// While the game counts a dead player's class menu as open, they miss
	/// respawn waves, and once it closes, they spawn at once if the game
	/// would have spawned them meanwhile. A client reports its own menu
	/// whenever it shows or hides it, overriding this.
	///
	/// The command goes through the engine's client command handling, as a
	/// client's own commands do, so command hooks and listeners, the game's
	/// and other plugins' included, run before this returns, and it counts
	/// toward `sv_quota_stringcmdspersecond`.
	#[doc(alias("menuopen", "menuclosed", "m_bIsClassMenuOpen"))]
	pub fn set_class_menu_open(self, open: bool) -> Result<(), PlayerError> {
		self.check_live()?;

		let edict = self.player.edict().ok_or(PlayerError::NotNetworked)?;
		let command = if open { c"menuopen" } else { c"menuclosed" };

		self.server.plugin_helpers()?.client_command(edict, command);

		Ok(())
	}

	/// Shows the player as `model`, a model's path such as
	/// `models/bots/scout/bot_scout.mdl`, with the model's own animations
	/// (`SetCustomModel`).
	///
	/// TF2 precaches the model if needed, which takes an entry of the level's
	/// model table until the level ends, and fails the level if the table is
	/// full. So precache models when the level starts, and do not take paths
	/// from untrusted input. The model stays until it is cleared or the player
	/// changes class, through respawns.
	#[doc(alias("SetCustomModel"))]
	pub fn set_custom_model(self, model: &CStr) -> Result<(), PlayerError> {
		self.call_with_model(c"SetCustomModel", model)
	}

	/// Offsets the player's custom model from where the player stands
	/// (`SetCustomModelOffset`). Fails with [`PlayerError::NonFinite`] for an
	/// offset that is not finite.
	#[doc(alias("SetCustomModelOffset"))]
	pub fn set_custom_model_offset(self, offset: Vector) -> Result<(), PlayerError> {
		if !offset.is_finite() {
			return Err(PlayerError::NonFinite);
		}

		let offset = sys::Vector::from(offset);

		// SAFETY: The checked member stores the finite offset, which it only
		// reads during the call, and invalidates the player's cached position.
		unsafe {
			self.call(
				c"SetCustomModelOffset",
				&mut [vector(&offset)],
				binding::VOID,
			)
		}?;

		Ok(())
	}

	/// Whether the player's custom model turns with the player, as it does by
	/// default (`SetCustomModelRotates`).
	#[doc(alias("SetCustomModelRotates"))]
	pub fn set_custom_model_rotates(self, rotates: bool) -> Result<(), PlayerError> {
		// SAFETY: The checked member stores the flag, and invalidates the
		// player's cached angles.
		unsafe { self.call_with_bool(c"SetCustomModelRotates", rotates) }
	}

	/// Rotates the player's custom model (`SetCustomModelRotation`). Fails
	/// with [`PlayerError::NonFinite`] for angles that are not finite.
	#[doc(alias("SetCustomModelRotation"))]
	pub fn set_custom_model_rotation(self, rotation: QAngle) -> Result<(), PlayerError> {
		if ![rotation.pitch, rotation.yaw, rotation.roll]
			.iter()
			.all(|angle| angle.is_finite())
		{
			return Err(PlayerError::NonFinite);
		}

		let rotation = sys::QAngle::from(rotation);

		// SAFETY: The checked member stores the finite angles, which it only
		// reads during the call, and invalidates the player's cached angles.
		unsafe {
			self.call(
				c"SetCustomModelRotation",
				&mut [qangle(&rotation)],
				binding::VOID,
			)
		}?;

		Ok(())
	}

	/// Whether the player sees their custom model in third person, as they do
	/// by default (`SetCustomModelVisibleToSelf`).
	#[doc(alias("SetCustomModelVisibleToSelf"))]
	pub fn set_custom_model_visible_to_self(self, visible: bool) -> Result<(), PlayerError> {
		// SAFETY: The checked member stores the flag.
		unsafe { self.call_with_bool(c"SetCustomModelVisibleToSelf", visible) }
	}

	/// As [`Self::set_custom_model`], but animates the model with the
	/// player's class's animations, as Mann vs. Machine's robots are
	/// (`SetCustomModelWithClassAnimations`).
	#[doc(alias("SetCustomModelWithClassAnimations"))]
	pub fn set_custom_model_with_class_animations(self, model: &CStr) -> Result<(), PlayerError> {
		self.call_with_model(c"SetCustomModelWithClassAnimations", model)
	}

	/// Sets the class the player spawns as next (`m_iDesiredPlayerClass`),
	/// as choosing it from the class menu would, without its checks or
	/// respawn, or clears it with `None`, which keeps the player from
	/// spawning until they choose one.
	#[doc(alias("m_iDesiredPlayerClass", "SetDesiredPlayerClassIndex"))]
	pub fn set_desired_class(self, class: Option<PlayerClass>) -> Result<(), PlayerError> {
		self.check_live()?;

		let engine = self.server.valve_engine()?;
		let raw = class.map_or(0, PlayerClass::to_raw);

		// SAFETY: The game assigns `TF_CLASS_UNDEFINED` and the playable
		// classes itself.
		unsafe {
			self.net_prop(c"m_iDesiredPlayerClass")?
				.set(engine, self.player, raw)
		}?;

		Ok(())
	}

	/// Forces the player's view into third person, as taunts do, or lets
	/// them choose it again (`SetForcedTauntCam`). The game clears it as the
	/// player spawns.
	#[doc(alias("SetForcedTauntCam", "m_nForceTauntCam"))]
	pub fn set_forced_taunt_cam(self, cam: ForcedTauntCam) -> Result<(), PlayerError> {
		// SAFETY: The checked member stores one of the values the game uses.
		unsafe {
			self.call(
				c"SetForcedTauntCam",
				&mut [int(cam.to_raw())],
				binding::VOID,
			)
		}?;

		Ok(())
	}

	/// Hides exactly `flags`'s parts of the player's HUD, showing the others
	/// (`SetHudHideFlags`).
	#[doc(alias("SetHudHideFlags", "m_iHideHUD"))]
	pub fn set_hud_hide_flags(self, flags: HideHud) -> Result<(), PlayerError> {
		self.call_with_flags(c"SetHudHideFlags", flags)
	}

	/// Sets the game time from which the player may change their class again
	/// (`SetNextChangeClassTime`). Fails with [`PlayerError::NonFinite`] for a
	/// time that is not finite.
	#[doc(alias("SetNextChangeClassTime", "m_flNextChangeClassTime"))]
	pub fn set_next_change_class_time(self, time: f32) -> Result<(), PlayerError> {
		self.set_time(c"SetNextChangeClassTime", time)
	}

	/// Sets the game time from which the player may change their team again
	/// (`SetNextChangeTeamTime`). Fails with [`PlayerError::NonFinite`] for a
	/// time that is not finite.
	#[doc(alias("SetNextChangeTeamTime", "m_flNextChangeTeamTime"))]
	pub fn set_next_change_team_time(self, time: f32) -> Result<(), PlayerError> {
		self.set_time(c"SetNextChangeTeamTime", time)
	}

	/// Calls a member that stores a finite game time.
	fn set_time(self, name: &CStr, time: f32) -> Result<(), PlayerError> {
		if !time.is_finite() {
			return Err(PlayerError::NonFinite);
		}

		// SAFETY: The time members store a field of the player.
		unsafe { self.call(name, &mut [binding::float(time)], binding::VOID) }?;

		Ok(())
	}

	/// Shows the player their team's class menu, as the game does as they join
	/// a team. Fails with [`PlayerError::NotPlaying`] for a player on neither
	/// RED nor BLU.
	pub fn show_class_menu(self) -> Result<(), PlayerError> {
		let panel = match self.team()? {
			Team::Red => raw::PANEL_CLASS_RED,
			Team::Blue => raw::PANEL_CLASS_BLUE,
			Team::Unassigned | Team::Spectator => return Err(PlayerError::NotPlaying),
		};

		self.send_panel(panel, true)
	}

	/// Shows the player the team menu, as the game does as they join the
	/// server.
	pub fn show_team_menu(self) -> Result<(), PlayerError> {
		self.send_panel(raw::PANEL_TEAM, true)
	}

	/// A bit the game flips each time the player spawns (`m_iSpawnCounter`),
	/// from which clients tell that they respawned. A change between two
	/// reads means they spawned an odd number of times in between.
	#[doc(alias("m_iSpawnCounter"))]
	pub fn spawn_parity(self) -> Result<bool, PlayerError> {
		Ok(self
			.net_prop(c"m_iSpawnCounter")?
			.get::<c_int>(self.player)?
			!= 0)
	}

	/// Where the player is in the game's handling of them
	/// (`m_nPlayerState`).
	#[doc(alias("m_nPlayerState"))]
	pub fn state(self) -> Result<PlayerState, PlayerError> {
		let raw = self
			.net_prop(c"m_nPlayerState")?
			.get::<c_int>(self.player)?;

		PlayerState::from_raw(raw).ok_or(PlayerError::UnknownState(raw))
	}

	/// The player's team (`m_iTeamNum`).
	#[doc(alias("m_iTeamNum", "GetTeamNumber"))]
	pub fn team(self) -> Result<Team, PlayerError> {
		let raw = self.net_prop(c"m_iTeamNum")?.get::<c_int>(self.player)?;

		Team::from_raw(raw).ok_or(PlayerError::UnknownTeam(raw))
	}

	/// The player's view offset (`m_vecViewOffset`): how far above their
	/// origin they see from, which depends on their class and whether they
	/// duck.
	#[doc(alias("m_vecViewOffset", "GetViewOffset"))]
	pub fn view_offset(self) -> Result<Vector, PlayerError> {
		let [x, y, z] = VIEW_OFFSET.map(|name| {
			self.net_prop(name)
				.and_then(|prop| Ok(prop.get::<f32>(self.player)?))
		});

		Ok(Vector::new(x?, y?, z?))
	}
}
