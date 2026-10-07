//! Mann vs. Machine: players' money, mini-bosses and upgrades, the wave the
//! population manager is on, the map's populator inputs, and the mission
//! the server plays.
//!
//! [`MvmPlayer`] calls the native methods `CTFPlayer`'s script class
//! declares for these (`game/server/tf/tf_player.cpp:555-712`), through
//! their typed binding descriptors, without a script VM. Every player has
//! them in every mode, but only Mann vs. Machine, and modes with upgrades,
//! spend money and buy upgrades.
//!
//! [`MvmWave`] reads what the objective resource (`tf_objective_resource`)
//! networks of the population manager's state, for the HUD: the wave, its
//! robots, and the money lying uncollected. The population manager writes
//! it, so it is read-only here. [`Populator`] sends the inputs maps send the
//! population manager through a `point_populator_interface`, and
//! [`load_popfile`] queues `tf_mvm_popfile`, which plays another mission.
//!
//! The robots are [bots](crate::tf2::bots), and the tank is a
//! [`BaseBoss`](crate::tf2::bosses::BaseBoss).
//!
//! # Unverified
//!
//! The wrappers follow Valve's Source SDK 2013 and have not been tested on a
//! live server.

#[cfg(test)]
#[path = "../tests/tf2/mvm.rs"]
mod tests;

use crate::datatables::{NetPropError, NetValue};
use crate::entities::Entity;
use crate::inputs::{InputError, InputValue};
use crate::tf2::script_binding::{self as binding, BindingError, VOID};
use crate::{Game, InterfaceError, Server};
use sdk_raw::tf2::bots::TF_PLAYER_CLASS;
use sdk_raw::tf2::script_binding::{BOOL, INT, boolean, int};
use std::ffi::{CStr, CString, c_int};

/// The most money [`MvmPlayer::add_currency`] leaves a player with, as
/// `CTFPlayer::AddCurrency` caps it.
pub const MAX_ADDED_CURRENCY: c_int = 30_000;

/// The data map class of the objective resource.
const OBJECTIVE_RESOURCE_CLASS: &CStr = c"CTFObjectiveResource";

/// The class name of the objective resource.
const OBJECTIVE_RESOURCE: &CStr = c"tf_objective_resource";

/// The class name of the populator interface.
const POPULATOR: &CStr = c"point_populator_interface";

/// The data map class of the populator interface.
const POPULATOR_CLASS: &CStr = c"CPointPopulatorInterface";

/// Why a Mann vs. Machine wrapper failed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MvmError {
	/// The server is not running Team Fortress 2.
	#[error("Mann vs. Machine requires Team Fortress 2")]
	UnsupportedGame,

	/// The entity is not of the class the wrapper takes.
	#[error("the entity is not a {class:?}")]
	WrongClass {
		/// The data map class the wrapper takes.
		class: &'static CStr,
	},

	/// The entity is marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The player's money would overflow an `int`, which the game adds and
	/// subtracts unchecked.
	#[error("{amount} would overflow the player's money of {currency}")]
	CurrencyOverflow {
		/// The player's money.
		currency: c_int,

		/// The amount added or removed.
		amount: c_int,
	},

	/// A mission name is empty, or holds a `"` or a control character, which
	/// would break the command line.
	#[error("the mission name cannot be passed to tf_mvm_popfile")]
	InvalidPopfileName,

	/// The game created no `point_populator_interface`.
	#[error("the game could not create a point_populator_interface")]
	NotCreated,

	/// The new `point_populator_interface` marked itself for deletion as it
	/// spawned.
	#[error("the populator interface removed itself as it spawned")]
	SpawnFailed,

	/// A networked variable is not of the kind the SDK reads it as.
	#[error("the networked variable {name:?} is not of the expected kind")]
	UnsupportedLayout {
		/// The variable.
		name: &'static CStr,
	},

	/// The script class descriptors lack a native method, or its signature
	/// differs from the SDK's.
	#[error("the game does not expose the expected native method")]
	UnsupportedMethod,

	/// A native method's binding adapter reported failure.
	#[error("the native method rejected the call")]
	Rejected,

	/// An input was not sent, or the entity rejected it.
	#[error(transparent)]
	Input(#[from] InputError),

	/// A required engine interface is unavailable.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// A networked variable could not be read.
	#[error(transparent)]
	NetProp(#[from] NetPropError),
}

impl From<BindingError> for MvmError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// A TF2 player or bot within the current engine callback, for what Mann vs.
/// Machine gives players: money, the mini-boss flag, the boss health bar,
/// and upgrades.
///
/// A robot's money is what it drops when killed, which the population
/// manager sets.
#[derive(Debug, Clone, Copy)]
pub struct MvmPlayer<'s> {
	entity: Entity<'s>,
}

impl<'s> MvmPlayer<'s> {
	/// Wraps `entity`. Fails with [`MvmError::UnsupportedGame`] outside TF2,
	/// and with [`MvmError::WrongClass`] unless its data maps include
	/// `CTFPlayer`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, MvmError> {
		check_class(server, entity, TF_PLAYER_CLASS)?;

		Ok(Self { entity })
	}

	/// Adds `amount` to the player's money, or removes it if negative, keeping
	/// the result within 0 to [`MAX_ADDED_CURRENCY`] (`AddCurrency`). Unlike
	/// the money the game awards, this counts in no statistics.
	///
	/// Fails with [`MvmError::CurrencyOverflow`] if the sum overflows, as it
	/// can for money [set](Self::set_currency) beyond the cap.
	#[doc(alias("AddCurrency"))]
	pub fn add_currency(self, amount: c_int) -> Result<(), MvmError> {
		let currency = self.currency()?;

		if currency.checked_add(amount).is_none() {
			return Err(MvmError::CurrencyOverflow { currency, amount });
		}

		// SAFETY: The method adds the amount to the player's money and clamps
		// it, and the sum fits in an `int`.
		unsafe { self.call(c"AddCurrency", &mut [int(amount)], VOID) }?;

		Ok(())
	}

	/// Calls one of `CTFPlayer`'s native methods, unless the player is marked
	/// for deletion.
	///
	/// # Safety
	///
	/// As for [`binding::call`]: the method must accept the player and these
	/// arguments.
	unsafe fn call(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
		result: sys::ScriptDataType_t,
	) -> Result<sys::ScriptVariant_t, MvmError> {
		if self.entity.is_marked_for_deletion() {
			return Err(MvmError::MarkedForDeletion);
		}

		// SAFETY: As the caller promises.
		Ok(unsafe { binding::call(self.entity, TF_PLAYER_CLASS, name, arguments, result) }?)
	}

	/// The player's money (`GetCurrency`, `m_nCurrency`).
	#[doc(alias("GetCurrency", "m_nCurrency"))]
	pub fn currency(self) -> Result<c_int, MvmError> {
		// SAFETY: The method reads the player's money.
		let result = unsafe { self.call(c"GetCurrency", &mut [], INT) }?;

		// SAFETY: `call` checked that the adapter returned an integer variant.
		Ok(unsafe { result.__bindgen_anon_1.m_int })
	}

	/// The player's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Whether the player is a mini-boss, as the giant robots are
	/// (`IsMiniBoss`).
	#[doc(alias("IsMiniBoss", "m_bIsMiniBoss"))]
	pub fn is_mini_boss(self) -> Result<bool, MvmError> {
		// SAFETY: The method reads a flag of the player.
		let result = unsafe { self.call(c"IsMiniBoss", &mut [], BOOL) }?;

		// SAFETY: `call` checked that the adapter returned a boolean variant.
		Ok(unsafe { result.__bindgen_anon_1.m_bool })
	}

	/// Removes `amount` from the player's money, down to 0 at the least
	/// (`RemoveCurrency`). In Mann vs. Machine, the population manager counts
	/// the amount as spent, in the mission's statistics.
	///
	/// Fails with [`MvmError::CurrencyOverflow`] if the difference overflows.
	#[doc(alias("RemoveCurrency"))]
	pub fn remove_currency(self, amount: c_int) -> Result<(), MvmError> {
		let currency = self.currency()?;

		if currency.checked_sub(amount).is_none() {
			return Err(MvmError::CurrencyOverflow { currency, amount });
		}

		// SAFETY: The method subtracts the amount from the player's money, which
		// the difference fits, and tells the population manager, which the game
		// rules create in Mann vs. Machine.
		unsafe { self.call(c"RemoveCurrency", &mut [int(amount)], VOID) }?;

		Ok(())
	}

	/// Removes every upgrade the player bought, from the player and their
	/// items, and from the upgrades the population manager restores at a
	/// checkpoint, refunding their money if `refund` is set
	/// (`GrantOrRemoveAllUpgrades`).
	#[doc(alias("GrantOrRemoveAllUpgrades"))]
	pub fn remove_upgrades(self, refund: bool) -> Result<(), MvmError> {
		// SAFETY: The method undoes the player's purchases through the upgrade
		// station's logic, if the level has the station, as a respec does with
		// a refund.
		unsafe {
			self.call(
				c"GrantOrRemoveAllUpgrades",
				&mut [boolean(true), boolean(refund)],
				VOID,
			)
		}?;

		Ok(())
	}

	/// Sets the player's money, without the cap of
	/// [`add_currency`](Self::add_currency) (`SetCurrency`).
	#[doc(alias("SetCurrency", "m_nCurrency"))]
	pub fn set_currency(self, currency: c_int) -> Result<(), MvmError> {
		// SAFETY: The method stores the player's money.
		unsafe { self.call(c"SetCurrency", &mut [int(currency)], VOID) }?;

		Ok(())
	}

	/// Sets whether the player is a mini-boss, the flag the game checks for
	/// the giant robots (`SetIsMiniBoss`). It changes neither the player's
	/// size nor their health. A bot carrying the bomb as a mini-boss does not
	/// upgrade it.
	#[doc(alias("SetIsMiniBoss", "m_bIsMiniBoss"))]
	pub fn set_mini_boss(self, mini_boss: bool) -> Result<(), MvmError> {
		// SAFETY: The method stores the flag.
		unsafe { self.call(c"SetIsMiniBoss", &mut [boolean(mini_boss)], VOID) }?;

		Ok(())
	}

	/// Sets whether clients show the player's health in a boss health bar, as
	/// the population manager does for robots with the `UseBossHealthBar`
	/// attribute (`SetUseBossHealthBar`).
	#[doc(alias("SetUseBossHealthBar", "m_bUseBossHealthBar"))]
	pub fn set_use_boss_health_bar(self, use_bar: bool) -> Result<(), MvmError> {
		// SAFETY: The method stores the flag.
		unsafe { self.call(c"SetUseBossHealthBar", &mut [boolean(use_bar)], VOID) }?;

		Ok(())
	}
}

/// The population manager's state, as the objective resource
/// (`tf_objective_resource`, `CTFObjectiveResource`) networks it for the HUD,
/// within the current engine callback.
///
/// Outside Mann vs. Machine, the values stay at 0.
#[derive(Debug, Clone, Copy)]
pub struct MvmWave<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
}

impl<'s> MvmWave<'s> {
	/// Wraps `entity`. Fails with [`MvmError::UnsupportedGame`] outside TF2,
	/// and with [`MvmError::WrongClass`] unless its data maps include
	/// `CTFObjectiveResource`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, MvmError> {
		check_class(server, entity, OBJECTIVE_RESOURCE_CLASS)?;

		Ok(Self { server, entity })
	}

	/// Finds the objective resource, or `None` if there is none, as before
	/// the level's entities spawn.
	pub fn find(server: Server<'s>) -> Result<Option<Self>, MvmError> {
		find(server, OBJECTIVE_RESOURCE, Self::new)
	}

	/// How far the bomb carrier has upgraded, from 0 to 3, or 4 for a
	/// mini-boss carrying it, which does not upgrade
	/// (`m_nFlagCarrierUpgradeLevel`).
	#[doc(alias("m_nFlagCarrierUpgradeLevel"))]
	pub fn bomb_carrier_level(self) -> Result<c_int, MvmError> {
		self.get(c"m_nFlagCarrierUpgradeLevel")
	}

	/// How many waves the mission has (`m_nMannVsMachineMaxWaveCount`).
	#[doc(alias("m_nMannVsMachineMaxWaveCount"))]
	pub fn count(self) -> Result<c_int, MvmError> {
		self.get(c"m_nMannVsMachineMaxWaveCount")
	}

	/// How many robots the current wave sends, as its HUD counts them
	/// (`m_nMannVsMachineWaveEnemyCount`).
	#[doc(alias("m_nMannVsMachineWaveEnemyCount"))]
	pub fn enemy_count(self) -> Result<c_int, MvmError> {
		self.get(c"m_nMannVsMachineWaveEnemyCount")
	}

	/// The objective resource's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Reads a networked variable.
	fn get<T: crate::datatables::NetVar>(self, name: &CStr) -> Result<T, MvmError> {
		if self.entity.is_marked_for_deletion() {
			return Err(MvmError::MarkedForDeletion);
		}

		let prop = self
			.server
			.server_game_dll()?
			.entity_net_prop(self.entity, name)?;

		Ok(prop.get(self.entity)?)
	}

	/// Whether the current wave sends a tank
	/// (`m_nMannVsMachineWaveHasTanks`).
	#[doc(alias("m_nMannVsMachineWaveHasTanks"))]
	pub fn has_tanks(self) -> Result<bool, MvmError> {
		self.get(c"m_nMannVsMachineWaveHasTanks")
	}

	/// Whether the mission is between waves, while the defenders upgrade and
	/// ready up (`m_bMannVsMachineBetweenWaves`).
	#[doc(alias("m_bMannVsMachineBetweenWaves"))]
	pub fn is_between_waves(self) -> Result<bool, MvmError> {
		self.get(c"m_bMannVsMachineBetweenWaves")
	}

	/// The game time at which the next wave starts on its own, or `None`
	/// while none is set to (`m_flMannVsMachineNextWaveTime`).
	#[doc(alias("m_flMannVsMachineNextWaveTime"))]
	pub fn next_wave_time(self) -> Result<Option<f32>, MvmError> {
		let time = self.get::<f32>(c"m_flMannVsMachineNextWaveTime")?;

		Ok((time > 0.0).then_some(time))
	}

	/// The current wave's number, from 1 (`m_nMannVsMachineWaveCount`).
	#[doc(alias("m_nMannVsMachineWaveCount"))]
	pub fn number(self) -> Result<c_int, MvmError> {
		self.get(c"m_nMannVsMachineWaveCount")
	}

	/// The path of the mission's population file, such as
	/// `scripts/population/mvm_decoy.pop` (`m_iszMvMPopfileName`).
	#[doc(alias("m_iszMvMPopfileName"))]
	pub fn popfile(self) -> Result<CString, MvmError> {
		const NAME: &CStr = c"m_iszMvMPopfileName";

		if self.entity.is_marked_for_deletion() {
			return Err(MvmError::MarkedForDeletion);
		}

		let prop = self
			.server
			.server_game_dll()?
			.entity_net_prop(self.entity, NAME)?;

		match prop.value(self.entity)? {
			NetValue::String(name) => Ok(name),
			_ => Err(MvmError::UnsupportedLayout { name: NAME }),
		}
	}

	/// The money robots dropped that no defender has collected yet
	/// (`m_nMvMWorldMoney`).
	#[doc(alias("m_nMvMWorldMoney"))]
	pub fn world_money(self) -> Result<c_int, MvmError> {
		self.get(c"m_nMvMWorldMoney")
	}
}

/// A `point_populator_interface` (`CPointPopulatorInterface`) within the
/// current engine callback: the entity through which maps pause the
/// population manager's spawning and change its robots' attributes.
///
/// Its inputs do nothing outside Mann vs. Machine.
#[doc(alias("point_populator_interface", "CPointPopulatorInterface"))]
#[derive(Debug, Clone, Copy)]
pub struct Populator<'s> {
	server: Server<'s>,
	entity: Entity<'s>,
}

impl<'s> Populator<'s> {
	/// Wraps `entity`. Fails with [`MvmError::UnsupportedGame`] outside TF2,
	/// and with [`MvmError::WrongClass`] unless its data maps include
	/// `CPointPopulatorInterface`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, MvmError> {
		check_class(server, entity, POPULATOR_CLASS)?;

		Ok(Self { server, entity })
	}

	/// Finds a `point_populator_interface`, or spawns one if the level has
	/// none.
	pub fn find_or_spawn(server: Server<'s>) -> Result<Self, MvmError> {
		if let Some(populator) = find(server, POPULATOR, Self::new)? {
			return Ok(populator);
		}

		let tools = server.server_tools()?;

		// SAFETY: `CPointPopulatorInterface` declares no constructor, and
		// `CPointEntity`'s sets nothing that frees entities.
		let entity =
			unsafe { tools.create_entity_by_name(POPULATOR) }.ok_or(MvmError::NotCreated)?;

		// SAFETY: `CPointEntity::Spawn` only makes the entity non-solid.
		unsafe { tools.dispatch_spawn(entity) };

		if entity.is_marked_for_deletion() {
			return Err(MvmError::SpawnFailed);
		}

		Self::new(server, entity)
	}

	/// Applies the attributes the mission's population file names `event` to
	/// the living robots that have them (`ChangeBotAttributes`).
	///
	/// With `tf_populator_debug` set, an event the file lacks is reported and
	/// skipped.
	#[doc(alias("ChangeBotAttributes"))]
	pub fn change_bot_attributes(self, event: &CStr) -> Result<(), MvmError> {
		self.input(c"ChangeBotAttributes", InputValue::String(event))
	}

	/// Makes the robots spawned from now on take the attributes the
	/// mission's population file names `event` (`ChangeDefaultEventAttributes`).
	#[doc(alias("ChangeDefaultEventAttributes"))]
	pub fn change_default_event_attributes(self, event: &CStr) -> Result<(), MvmError> {
		self.input(c"ChangeDefaultEventAttributes", InputValue::String(event))
	}

	/// The populator interface's entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// Sends the populator interface an input, with itself as the activator
	/// and caller.
	fn input(self, name: &CStr, value: InputValue<'_>) -> Result<(), MvmError> {
		let tools = self.server.server_tools()?;

		Ok(tools.accept_input(self.entity, name, value, self.entity, self.entity)?)
	}

	/// Pauses the population manager's spawning of robots
	/// (`PauseBotSpawning`).
	#[doc(alias("PauseBotSpawning"))]
	pub fn pause_spawning(self) -> Result<(), MvmError> {
		self.input(c"PauseBotSpawning", InputValue::Void)
	}

	/// Resumes the population manager's spawning of robots
	/// (`UnpauseBotSpawning`).
	#[doc(alias("UnpauseBotSpawning"))]
	pub fn unpause_spawning(self) -> Result<(), MvmError> {
		self.input(c"UnpauseBotSpawning", InputValue::Void)
	}
}

/// Queues `tf_mvm_popfile` with `name`, which the server runs from its
/// command buffer, normally on the next frame. The game then finds the
/// population file by its short name, such as `mvm_decoy_advanced`, and, if
/// it is valid, plays its mission from the start, resetting the map.
/// Outside Mann vs. Machine, or for a file it does not find, the game only
/// prints a message.
///
/// Fails with [`MvmError::InvalidPopfileName`] for an empty name, or one
/// holding a `"` or a control character.
#[doc(alias("tf_mvm_popfile"))]
pub fn load_popfile(server: Server<'_>, name: &CStr) -> Result<(), MvmError> {
	if server.game() != Game::TeamFortress2 {
		return Err(MvmError::UnsupportedGame);
	}

	let bytes = name.to_bytes();

	if bytes.is_empty()
		|| bytes
			.iter()
			.any(|&byte| byte < 0x20 || byte == 0x7f || byte == b'"')
	{
		return Err(MvmError::InvalidPopfileName);
	}

	let mut command = b"tf_mvm_popfile \"".to_vec();

	command.extend_from_slice(bytes);
	command.extend_from_slice(b"\"\n");

	let command = CString::new(command).expect("the name holds no NUL");

	server.valve_engine()?.server_command(&command);
	Ok(())
}

/// Fails with [`MvmError::UnsupportedGame`] outside TF2, and with
/// [`MvmError::WrongClass`] unless `entity`'s data maps include `class`'s.
fn check_class(
	server: Server<'_>,
	entity: Entity<'_>,
	class: &'static CStr,
) -> Result<(), MvmError> {
	if server.game() != Game::TeamFortress2 {
		return Err(MvmError::UnsupportedGame);
	}

	if entity.has_data_map_class(class) {
		Ok(())
	} else {
		Err(MvmError::WrongClass { class })
	}
}

/// The first entity of `class_name`, not marked for deletion, that `wrap`
/// accepts, or `None`.
fn find<'s, T>(
	server: Server<'s>,
	class_name: &CStr,
	wrap: impl Fn(Server<'s>, Entity<'s>) -> Result<T, MvmError>,
) -> Result<Option<T>, MvmError> {
	if server.game() != Game::TeamFortress2 {
		return Err(MvmError::UnsupportedGame);
	}

	let tools = server.server_tools()?;
	let mut found = tools.find_by_class_name(None, class_name);

	while let Some(entity) = found {
		if !entity.is_marked_for_deletion()
			&& let Ok(wrapped) = wrap(server, entity)
		{
			return Ok(Some(wrapped));
		}

		found = tools.find_by_class_name(Some(entity), class_name);
	}

	Ok(None)
}
