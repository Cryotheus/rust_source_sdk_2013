//! Health, maximum health, life state and damage mode, which `CBaseEntity`
//! declares for every entity: players, `func_breakable`s, props, TF2's
//! buildings and bosses alike.
//!
//! The members are found through `CBaseEntity`'s data description map, where
//! every game declares them, as the `health` and `max_health` key values, and
//! the private `m_lifeState` and `m_takedamage`. Each setter records the
//! change for networking, as the game's network variables do on assignment,
//! so the classes that send a member to clients, such as players for their
//! health, send the new value.
//!
//! # Setting health
//!
//! There are two ways to change an entity's health, which differ in how much
//! of the game runs:
//!
//! - [`Entity::set_health`] assigns the member, as `CBaseEntity::SetHealth`
//!   and VScript's `SetHealth` do, and runs no game logic: a player set to 0
//!   stays alive, and a `func_breakable` set to 0 does not break, until they
//!   are next damaged. TF2's buildings keep their health as a float too,
//!   which it sets as well.
//! - [`ServerTools::send_health_input`] sends one of the [`HealthInput`]s,
//!   which run the game logic of the classes that declare them, listed
//!   below, as a map's outputs do. Entities of other classes refuse them, as
//!   [`InputError::UnknownInput`] reports.
//!
//! | Class | Entities | Inputs | What the inputs do |
//! | --- | --- | --- | --- |
//! | `CBasePlayer` | `player` | `SetHealth` | Heals through `TakeHealth` or damages through `TakeDamage`, so armor is ignored, damage can kill, and TF2's healing caps at the player's maximum health. |
//! | `CAI_BaseNPC` | NPCs | `SetHealth` | As for players. |
//! | `CBreakable` | `func_breakable`, `func_breakable_surf`, `func_physbox`, `func_physbox_multiplayer`, `func_pushable` | `SetHealth`, `AddHealth`, `RemoveHealth` | When the health changes: fires `OnHealthChanged`, breaks the entity at 0 or less, unless it is unbreakable glass, and otherwise makes it take damage, or not if it only breaks when triggered (`CBreakable::UpdateHealth`). |
//! | `CBreakableProp` | `prop_physics`, `prop_dynamic`, and the props deriving from them | `SetHealth`, `AddHealth`, `RemoveHealth` | When the health changes: fires `OnHealthChanged`, and breaks the prop at 0 or less, leaving its damage mode alone (`CBreakableProp::UpdateHealth`). |
//! | TF2's `CBaseObject` | `obj_sentrygun`, `obj_dispenser`, `obj_teleporter`, `obj_attachment_sapper`, `mapobj_cart_dispenser` | `SetHealth`, `AddHealth`, `RemoveHealth` | `SetHealth` sets the maximum health too, `AddHealth` caps at the maximum, and `RemoveHealth` destroys the building at 0 or less, each through `CBaseObject::SetHealth`, which fires `OnObjectHealthChanged` when the health changes, and limits a redeploying building to the health it had when picked up. |
//! | TF2's `CTFBaseBoss` | `base_boss`, `tank_boss` | `SetHealth`, `SetMaxHealth`, `AddHealth`, `RemoveHealth` | `AddHealth` caps at the maximum, and `RemoveHealth` kills the boss at 0 or less. |
//!
//! # Maximum health
//!
//! Classes may compute their maximum health instead of storing it, as TF2's
//! players do from their class and attributes: [`Entity::max_health`] calls
//! the game's `GetMaxHealth` to get it, and [`Entity::stored_max_health`]
//! reads the `m_iMaxHealth` that `CBaseEntity::GetMaxHealth` returns. Setting
//! the stored maximum of a TF2 player changes nothing: change its
//! attributes, such as `max health additive bonus`, instead.
//!
//! [`ServerTools::send_health_input`]: crate::interfaces::ServerTools::send_health_input
//! [`InputError::UnknownInput`]: crate::inputs::InputError::UnknownInput

#[cfg(test)]
#[path = "../tests/entities/health.rs"]
mod tests;

use crate::entities::Entity;
use crate::interfaces::ValveEngine;
use crate::server::{Game, InterfaceError, Server};
use sdk_raw::entities::health as raw;
use sdk_raw::players::{LIFE_ALIVE, LIFE_DEAD, LIFE_DISCARDBODY, LIFE_DYING, LIFE_RESPAWNABLE};
use std::ffi::{CStr, c_int};
use std::sync::OnceLock;

/// The exclusive bound on a TF2 building's float health, `m_flHealth`: 2^31,
/// from which TF2's conversion back to an `int` is undefined in C++.
const BUILDING_HEALTH_LIMIT: f32 = 2_147_483_648.0;

/// The most health an entity may have for [`Entity::heal`], and the `tf2`
/// feature's `take_health`, to heal it: 2^30, so that healing of up to
/// [`MAX_HEALING`], scaled by TF2's healing multipliers by less than 64, stays
/// within an `int`.
const MAX_HEALED_HEALTH: c_int = 1 << 30;

/// The most healing [`Entity::heal`], and the `tf2` feature's `take_health`,
/// pass to the game, 2^24: an `f32` holds every integer up to it, and no
/// entity's health approaches it.
pub const MAX_HEALING: f32 = 16_777_216.0;

/// `m_iHealth`, the `health` key value.
static HEALTH: BaseEntityMember = BaseEntityMember::new(
	c"m_iHealth",
	sys::_fieldtypes_FIELD_INTEGER,
	size_of::<c_int>(),
);

/// `m_lifeState`.
static LIFE_STATE: BaseEntityMember = BaseEntityMember::new(
	c"m_lifeState",
	sys::_fieldtypes_FIELD_CHARACTER,
	size_of::<u8>(),
);

/// `m_iMaxHealth`, the `max_health` key value.
static MAX_HEALTH: BaseEntityMember = BaseEntityMember::new(
	c"m_iMaxHealth",
	sys::_fieldtypes_FIELD_INTEGER,
	size_of::<c_int>(),
);

/// `m_takedamage`.
static TAKE_DAMAGE: BaseEntityMember = BaseEntityMember::new(
	c"m_takedamage",
	sys::_fieldtypes_FIELD_CHARACTER,
	size_of::<u8>(),
);

/// A member `CBaseEntity`'s own datamap declares, whose offset is found once
/// and kept: every entity shares it through its `CBaseEntity` base.
struct BaseEntityMember {
	name: &'static CStr,
	field_type: sys::fieldtype_t,
	size: usize,
	offset: OnceLock<usize>,
}

impl BaseEntityMember {
	const fn new(name: &'static CStr, field_type: sys::fieldtype_t, size: usize) -> Self {
		Self {
			name,
			field_type,
			size,
			offset: OnceLock::new(),
		}
	}

	/// The member's offset, found through `entity`'s datamaps.
	///
	/// Only a found offset is kept, so an entity of a class whose maps lack
	/// `CBaseEntity`'s, which no game has, does not hide it from others.
	fn offset(&self, entity: Entity<'_>) -> Result<usize, HealthError> {
		if let Some(&offset) = self.offset.get() {
			return Ok(offset);
		}

		let offset = entity
			.find_base_entity_field(self.name, self.field_type, self.size)
			.ok_or(HealthError::UnsupportedLayout { member: self.name })?;

		Ok(*self.offset.get_or_init(|| offset))
	}

	/// Reads the member of `entity`.
	fn read<T: Copy>(&self, entity: Entity<'_>) -> Result<T, HealthError> {
		debug_assert_eq!(size_of::<T>(), self.size);

		let offset = self.offset(entity)?;

		// SAFETY: The offset was validated against `CBaseEntity`'s datamap,
		// which every entity shares through its base, for a member of `T`'s
		// size and type, aligned for it. The member is read without forming a
		// reference, as the game writes it too.
		Ok(unsafe { entity.as_ptr().byte_add(offset).cast::<T>().read() })
	}

	/// Writes the member of `entity`, and records the change for networking.
	fn write<T: Copy>(
		&self,
		engine: ValveEngine<'_>,
		entity: Entity<'_>,
		value: T,
	) -> Result<(), HealthError> {
		debug_assert_eq!(size_of::<T>(), self.size);

		let offset = self.offset(entity)?;

		// SAFETY: As for `read`. The game writes the member the same way,
		// through its own pointers, on the main thread.
		unsafe { entity.as_ptr().byte_add(offset).cast::<T>().write(value) };

		entity.network_state_changed(engine, offset);

		Ok(())
	}
}

/// How an entity reacts to damage (`m_takedamage`).
#[doc(alias("m_takedamage"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DamageMode {
	/// The entity loses no health to damage: the `OnTakeDamage` of
	/// `CBaseEntity` and `CBaseCombatCharacter` return before applying any,
	/// although a class may react to the damage first, as TF2's players do.
	/// `TakeHealth` heals nothing either, except TF2's players healed with
	/// `DMG_IGNORE_MAXHEALTH`.
	#[doc(alias("DAMAGE_NO"))]
	Immune,

	/// The entity runs its damage functions, such as its outputs and effects,
	/// without losing health, in the classes whose `OnTakeDamage` checks it:
	/// TF2's buildings (`CBaseObject::OnTakeDamage`) do not, and still lose
	/// health. `TakeHealth` heals nothing, as for [`Immune`](Self::Immune).
	#[doc(alias("DAMAGE_EVENTS_ONLY"))]
	EventsOnly,

	/// The entity takes damage.
	#[doc(alias("DAMAGE_YES"))]
	Vulnerable,

	/// The entity takes damage, and aim assistance may target it.
	#[doc(alias("DAMAGE_AIM"))]
	Aimable,
}

impl DamageMode {
	/// Converts a `DAMAGE_*` value, or returns `None` for another.
	pub const fn from_raw(value: u8) -> Option<Self> {
		match value {
			raw::DAMAGE_NO => Some(Self::Immune),
			raw::DAMAGE_EVENTS_ONLY => Some(Self::EventsOnly),
			raw::DAMAGE_YES => Some(Self::Vulnerable),
			raw::DAMAGE_AIM => Some(Self::Aimable),
			_ => None,
		}
	}

	/// The mode's `DAMAGE_*` value.
	pub const fn to_raw(self) -> u8 {
		match self {
			Self::Immune => raw::DAMAGE_NO,
			Self::EventsOnly => raw::DAMAGE_EVENTS_ONLY,
			Self::Vulnerable => raw::DAMAGE_YES,
			Self::Aimable => raw::DAMAGE_AIM,
		}
	}
}

/// An entity's health, or how it reacts to damage, could not be accessed.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HealthError {
	/// `CBaseEntity`'s datamap does not declare the member with the type and
	/// size the SDK gives it, at a plausible offset, so the game DLL does not
	/// match the SDK.
	#[error(
		"the game's CBaseEntity datamap does not declare `{}` as the SDK does",
		.member.to_string_lossy()
	)]
	UnsupportedLayout {
		/// The member's name, such as `m_iHealth`.
		member: &'static CStr,
	},

	/// The game server does not export an interface the setter needs, such as
	/// `IVEngineServer`, through which changes are recorded for networking.
	#[error(transparent)]
	Interface(#[from] InterfaceError),

	/// Only TF2's slot of the game's `GetMaxHealth` is known. Other games'
	/// slots depend on how they were built.
	#[error("the slot of GetMaxHealth is only known for TF2")]
	UnsupportedGame,

	/// A maximum health of 0 or less, which the game divides integers by,
	/// which crashes for 0: TF2's Horseless Headless Horsemann does while it
	/// moves, on the server, and so does the client of a TF2 building's
	/// builder, in its building-status HUD.
	#[error("a maximum health must be positive, not {0}")]
	NonPositiveMaxHealth(c_int),

	/// Healing that is negative, NaN, or more than [`MAX_HEALING`]. The game
	/// converts it to an `int`.
	#[error("healing must lie between 0 and 2^24, not {0}")]
	InvalidAmount(f32),

	/// The entity's health is above 2^30, to which healing, as TF2 may scale
	/// it, could add more than an `int` holds.
	#[error("an entity with {0} health is too healthy to heal")]
	HealingOverflow(c_int),

	/// A health or maximum health for a TF2 building that rounds to 2^31 or
	/// more as a float, which TF2 keeps a building's health as, and converts
	/// back to an `int` in a way C++ leaves undefined for it.
	#[error("a building's health must round below 2^31 as a float, which {0} does not")]
	UnrepresentableHealth(c_int),

	/// The entity's `m_lifeState` holds a value that is no `LIFE_*` value.
	#[error("the entity's life state is {0}, which is no LIFE_* value")]
	UnknownLifeState(u8),

	/// The entity's `m_takedamage` holds a value that is no `DAMAGE_*` value.
	#[error("the entity's damage mode is {0}, which is no DAMAGE_* value")]
	UnknownDamageMode(u8),
}

/// One of the inputs through which maps change an entity's health, which run
/// the game logic of the classes that declare them, as the
/// [module documentation](self#setting-health) lists.
///
/// Send one with
/// [`ServerTools::send_health_input`](crate::interfaces::ServerTools::send_health_input).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HealthInput {
	/// `SetHealth`: sets the entity's health, or for TF2's buildings, its
	/// health and maximum health.
	#[doc(alias("SetHealth", "InputSetHealth"))]
	Set(c_int),

	/// `AddHealth`: adds to the entity's health.
	#[doc(alias("AddHealth", "InputAddHealth"))]
	Add(c_int),

	/// `RemoveHealth`: removes from the entity's health.
	#[doc(alias("RemoveHealth", "InputRemoveHealth"))]
	Remove(c_int),

	/// `SetMaxHealth`: sets the maximum health of TF2's bosses.
	#[doc(alias("SetMaxHealth", "InputSetMaxHealth"))]
	SetMax(c_int),
}

impl HealthInput {
	/// The amount of health the input carries.
	pub const fn amount(self) -> c_int {
		match self {
			Self::Set(amount) | Self::Add(amount) | Self::Remove(amount) | Self::SetMax(amount) => {
				amount
			}
		}
	}

	/// The input's name, as the classes handling it declare it.
	pub const fn name(self) -> &'static CStr {
		match self {
			Self::Set(_) => c"SetHealth",
			Self::Add(_) => c"AddHealth",
			Self::Remove(_) => c"RemoveHealth",
			Self::SetMax(_) => c"SetMaxHealth",
		}
	}
}

/// Whether an entity is alive, dying or dead (`m_lifeState`).
#[doc(alias("m_lifeState"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifeState {
	/// Alive.
	#[doc(alias("LIFE_ALIVE"))]
	Alive,

	/// Playing its death animation, or falling until it hits the ground.
	#[doc(alias("LIFE_DYING"))]
	Dying,

	/// Dead, lying still. TF2's buildings take this state when destroyed.
	#[doc(alias("LIFE_DEAD"))]
	Dead,

	/// A dead player waiting to respawn, which TF2's players become once
	/// their death animation and freeze cam are over.
	#[doc(alias("LIFE_RESPAWNABLE"))]
	Respawnable,

	/// Dead, with a body to be discarded, which no game code assigns.
	#[doc(alias("LIFE_DISCARDBODY"))]
	DiscardBody,
}

impl LifeState {
	/// Converts a `LIFE_*` value, or returns `None` for another.
	pub const fn from_raw(value: u8) -> Option<Self> {
		match value {
			LIFE_ALIVE => Some(Self::Alive),
			LIFE_DYING => Some(Self::Dying),
			LIFE_DEAD => Some(Self::Dead),
			LIFE_RESPAWNABLE => Some(Self::Respawnable),
			LIFE_DISCARDBODY => Some(Self::DiscardBody),
			_ => None,
		}
	}

	/// The state's `LIFE_*` value.
	pub const fn to_raw(self) -> u8 {
		match self {
			Self::Alive => LIFE_ALIVE,
			Self::Dying => LIFE_DYING,
			Self::Dead => LIFE_DEAD,
			Self::Respawnable => LIFE_RESPAWNABLE,
			Self::DiscardBody => LIFE_DISCARDBODY,
		}
	}
}

impl<'s> Entity<'s> {
	/// How the entity reacts to damage (`m_takedamage`).
	///
	/// Fails with [`HealthError::UnknownDamageMode`] for a value that is no
	/// `DAMAGE_*` value, which the game never assigns.
	#[doc(alias("m_takedamage"))]
	pub fn damage_mode(self) -> Result<DamageMode, HealthError> {
		let value = TAKE_DAMAGE.read::<u8>(self)?;

		DamageMode::from_raw(value).ok_or(HealthError::UnknownDamageMode(value))
	}

	/// Heals the entity by `amount`, up to its [maximum health](Self::max_health),
	/// through the game's `TakeHealth`, and returns the health it gained.
	///
	/// The game heals only networked entities that [take damage](Self::damage_mode),
	/// and nothing at or above the maximum. TF2's players heal less while
	/// their active weapon reduces healing, and not at all under the
	/// `TF_COND_NOHEALINGDAMAGEBUFF` condition. For healing beyond the
	/// maximum, as TF2's overheal does, the `tf2` feature's `take_health`
	/// takes `DamageType::IGNORE_MAX_HEALTH`.
	///
	/// `TakeHealth` only adds to `m_iHealth`, so on TF2, a building's float
	/// health is then set to it, as [`set_health`](Self::set_health) sets it,
	/// for TF2 to keep the healing.
	///
	/// Fails, without healing, with [`HealthError::InvalidAmount`] for a
	/// negative amount, NaN, or more than [`MAX_HEALING`], and with
	/// [`HealthError::HealingOverflow`] for an entity whose health is above
	/// 2^30.
	#[doc(alias("TakeHealth"))]
	pub fn heal(self, server: Server<'_>, amount: f32) -> Result<c_int, HealthError> {
		self.take_health_raw(server, amount, raw::DMG_GENERIC)
	}

	/// The entity's health (`m_iHealth`), as `CBaseEntity::GetHealth` returns
	/// it.
	///
	/// TF2's buildings keep their health as a float, and this is the integer
	/// TF2 rounds it up to, which it networks.
	#[doc(alias("GetHealth", "m_iHealth"))]
	pub fn health(self) -> Result<c_int, HealthError> {
		HEALTH.read(self)
	}

	/// Whether the game considers the entity alive, through its `IsAlive`.
	///
	/// That is whether its [life state](Self::life_state) is
	/// [`LifeState::Alive`], unless its class decides otherwise, as props,
	/// which are never alive, do (`CBaseProp`).
	#[doc(alias("IsAlive"))]
	pub fn is_alive(self) -> bool {
		// SAFETY: The entity is live during `'s`, on the main thread, and its
		// class belongs to the loaded game DLL, whose vtables have `IsAlive`
		// where the generated one does. The method only reads the entity.
		unsafe { raw::is_alive(self.as_ptr()) }
	}

	/// Whether the entity is alive, dying or dead (`m_lifeState`).
	///
	/// Fails with [`HealthError::UnknownLifeState`] for a value that is no
	/// `LIFE_*` value, which the game never assigns.
	#[doc(alias("m_lifeState"))]
	pub fn life_state(self) -> Result<LifeState, HealthError> {
		let value = LIFE_STATE.read::<u8>(self)?;

		LifeState::from_raw(value).ok_or(HealthError::UnknownLifeState(value))
	}

	/// The entity's maximum health, as the game's `GetMaxHealth` computes it.
	///
	/// That is the [stored maximum](Self::stored_max_health), unless the
	/// entity's class computes its own, as TF2's players do from their class
	/// and attributes, including what their active weapon adds.
	///
	/// Fails with [`HealthError::UnsupportedGame`] on other games than TF2,
	/// where [`stored_max_health`](Self::stored_max_health), and for players
	/// [`PlayerInfo::max_health`](crate::interfaces::player_info_manager::PlayerInfo::max_health),
	/// remain.
	#[doc(alias("GetMaxHealth"))]
	pub fn max_health(self, server: Server<'_>) -> Result<c_int, HealthError> {
		match server.game() {
			// SAFETY: The entity is live during `'s`, on the main thread, and
			// `Server::new` condition 2 guarantees the game DLL is TF2's, whose
			// vtables have `GetMaxHealth` where the generated one does. TF2's
			// own methods only read the entity and its attributes.
			Game::TeamFortress2 => Ok(unsafe { raw::get_max_health(self.as_ptr()) }),

			Game::SourceSdk2013 | Game::SourceSdk2013NextBot => Err(HealthError::UnsupportedGame),
		}
	}

	/// Records that the member at `offset` changed, so the engine sends it to
	/// clients if the entity's class networks it, as
	/// `CBaseEntity::NetworkStateChanged` does. Server-only entities have
	/// nothing to record.
	fn network_state_changed(self, engine: ValveEngine<'_>, offset: usize) {
		let Some(edict) = self.edict() else {
			return;
		};

		match u16::try_from(offset) {
			Ok(offset) => edict.state_changed(engine, offset),
			Err(_) => edict.full_state_changed(engine),
		}
	}

	/// Sets how the entity reacts to damage (`m_takedamage`), and records
	/// the change for networking.
	///
	/// [`DamageMode::Immune`] makes an entity invulnerable, but entities set
	/// it themselves, such as a `func_breakable` once broken, or one that
	/// only breaks when triggered, and change it again later, such as a
	/// breakable whose health an input raises. Removing damage does not
	/// change [`health`](Self::health).
	#[doc(alias("m_takedamage"))]
	pub fn set_damage_mode(self, server: Server<'_>, mode: DamageMode) -> Result<(), HealthError> {
		TAKE_DAMAGE.write(server.valve_engine()?, self, mode.to_raw())
	}

	/// Sets the entity's health (`m_iHealth`), as `CBaseEntity::SetHealth`
	/// and VScript's `SetHealth` do, and records the change for networking.
	///
	/// This runs no game logic, as the
	/// [module documentation](self#setting-health) compares with sending the
	/// `SetHealth` input. Health may exceed the maximum, as TF2's overheal
	/// does, which TF2's players then lose over time, and may be 0 or less
	/// without killing anything until the entity is next damaged.
	///
	/// On TF2, a building's (`CBaseObject`) health is also set as its float
	/// health, `m_flHealth`, which its damage, repairs and upgrades work
	/// from, as `CBaseObject::SetHealth` does, without its
	/// `OnObjectHealthChanged` output. As there, the integer health is the
	/// float rounded up, so health beyond 16,777,216 is rounded, and fails
	/// with [`HealthError::UnrepresentableHealth`] if it rounds to 2^31.
	/// Unlike there, a carried building's health is not limited to what it
	/// had when picked up while it redeploys: TF2 lowers it to that at its
	/// next construction step.
	#[doc(alias("SetHealth", "m_iHealth", "health"))]
	pub fn set_health(self, server: Server<'_>, health: c_int) -> Result<(), HealthError> {
		let engine = server.valve_engine()?;

		let Some(building) = self.tf2_building(server) else {
			return HEALTH.write(engine, self, health);
		};

		let float = health as f32;

		if float >= BUILDING_HEALTH_LIMIT {
			return Err(HealthError::UnrepresentableHealth(health));
		}

		HEALTH.offset(self)?;

		// SAFETY: `tf2_building` found a TF2 `CBaseObject`, whose primary bases
		// start their classes, as `sdk_raw::entities::health` asserts, and the
		// member is written without forming a reference, as the game writes it
		// too. TF2 does not network it.
		unsafe { (&raw mut (*building).m_flHealth).write(float) };

		// The float lies within `int`'s range, so its ceiling does too.
		HEALTH.write(engine, self, float.ceil() as c_int)
	}

	/// Sets whether the entity is alive, dying or dead (`m_lifeState`), and
	/// records the change for networking.
	///
	/// This only changes the state, which kills or revives nothing: game code
	/// asking whether the entity is alive, such as its `IsAlive`, sees the new
	/// state until the game assigns its own, when it next kills or spawns the
	/// entity.
	#[doc(alias("m_lifeState"))]
	pub fn set_life_state(self, server: Server<'_>, state: LifeState) -> Result<(), HealthError> {
		LIFE_STATE.write(server.valve_engine()?, self, state.to_raw())
	}

	/// Sets the entity's stored maximum health (`m_iMaxHealth`), as
	/// `CBaseEntity::SetMaxHealth` and VScript's `SetMaxHealth` do, and records
	/// the change for networking. Health above the new maximum is kept.
	///
	/// Classes that compute their maximum, such as TF2's players, ignore it,
	/// as the [module documentation](self#maximum-health) describes.
	///
	/// Fails, without setting it, with [`HealthError::NonPositiveMaxHealth`]
	/// for 0 or less, which game code divides by, and, on TF2, with
	/// [`HealthError::UnrepresentableHealth`] for a building's maximum health
	/// that rounds to 2^31 as a float: TF2 heals a building up to its maximum
	/// as a float.
	#[doc(alias("SetMaxHealth", "m_iMaxHealth", "max_health"))]
	pub fn set_max_health(self, server: Server<'_>, max_health: c_int) -> Result<(), HealthError> {
		if max_health <= 0 {
			return Err(HealthError::NonPositiveMaxHealth(max_health));
		}

		if max_health as f32 >= BUILDING_HEALTH_LIMIT && self.tf2_building(server).is_some() {
			return Err(HealthError::UnrepresentableHealth(max_health));
		}

		MAX_HEALTH.write(server.valve_engine()?, self, max_health)
	}

	/// The entity's stored maximum health (`m_iMaxHealth`), which
	/// `CBaseEntity::GetMaxHealth` returns, unless the entity's class
	/// computes its own, as the [module documentation](self#maximum-health)
	/// describes.
	#[doc(alias("m_iMaxHealth", "max_health"))]
	pub fn stored_max_health(self) -> Result<c_int, HealthError> {
		MAX_HEALTH.read(self)
	}

	/// Heals the entity by `amount` through the game's `TakeHealth`, as
	/// [`heal`](Self::heal) does, with `damage_type` as the kind of damage
	/// healed, and returns the health it gained. `CBasePlayer` stops
	/// suffering the healed kinds of damage that do not hurt over time.
	///
	/// On TF2, [`DamageType::IGNORE_MAX_HEALTH`] makes a player's healing
	/// exceed its maximum health, as overheal does, without TF2's limit on
	/// overheal. TF2 then adds the healing itself, without the checks of
	/// [`heal`](Self::heal) for networked entities that take damage, and
	/// without the `take_health` event, and clears the healed kinds of damage
	/// itself. The bounds [`heal`](Self::heal) checks keep the result within
	/// an `int` while TF2's healing multipliers, which scale the amount first,
	/// stay below 64.
	///
	/// [`DamageType::IGNORE_MAX_HEALTH`]: crate::tf2::damage::DamageType::IGNORE_MAX_HEALTH
	#[cfg(feature = "tf2")]
	#[cfg_attr(docsrs, doc(cfg(feature = "tf2")))]
	#[doc(alias("TakeHealth"))]
	pub fn take_health(
		self,
		server: Server<'_>,
		amount: f32,
		damage_type: crate::tf2::damage::DamageType,
	) -> Result<c_int, HealthError> {
		self.take_health_raw(server, amount, damage_type.bits().cast_signed())
	}

	/// Calls the game's `TakeHealth` with a `DMG_*` mask, after checking the
	/// amount and the entity's health, and keeps a TF2 building's float
	/// health in step.
	fn take_health_raw(
		self,
		server: Server<'_>,
		amount: f32,
		damage_type: c_int,
	) -> Result<c_int, HealthError> {
		if !(0.0..=MAX_HEALING).contains(&amount) {
			return Err(HealthError::InvalidAmount(amount));
		}

		let health = HEALTH.read::<c_int>(self)?;

		if health > MAX_HEALED_HEALTH {
			return Err(HealthError::HealingOverflow(health));
		}

		// SAFETY: The entity is live during `'s`, on the main thread, and its
		// class belongs to the loaded game DLL. The amount is at most 2^24 and
		// the health at most 2^30, so the amount, scaled by TF2's healing
		// multipliers, which their gameplay domains keep below 64, converts to
		// an `int`, and adding it to the health does not overflow. Attribute
		// values outside those domains can only be set with unsafe calls
		// whose callers rule them out. The healing, including TF2's
		// attribute hooks and the `take_health` event, frees no entity
		// (`Server::new` condition 4).
		let gained = unsafe { raw::take_health(self.as_ptr(), amount, damage_type) };

		if let Some(building) = self.tf2_building(server) {
			let healed = HEALTH.read::<c_int>(self)?;

			if healed != health {
				// SAFETY: As for `set_health`. The health is at most
				// 2^30 + 2^24, which rounds below 2^31 as a float.
				unsafe { (&raw mut (*building).m_flHealth).write(healed as f32) };
			}
		}

		Ok(gained)
	}

	/// The entity as a TF2 building, if the server runs TF2 and a datamap of
	/// the entity's class or its bases is `CBaseObject`'s.
	fn tf2_building(self, server: Server<'_>) -> Option<*mut sys::CBaseObject> {
		(server.game() == Game::TeamFortress2 && self.has_data_map_class(c"CBaseObject"))
			.then(|| self.as_ptr().cast())
	}
}
