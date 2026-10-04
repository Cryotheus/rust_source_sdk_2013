//! `IPlayerInfoManager`, which exposes player state and the engine's globals.

use crate::NotThreadSafe;
use crate::edicts::Edict;
use crate::math::{QAngle, Vector};
use crate::players::UserId;
use sdk_raw::interfaces::player_info_manager as raw;
use sdk_raw::util::cstr::copy_cstr;
use sdk_raw::vcall;
use std::ffi::{CString, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

interface! {
	/// Exposes player state and the engine's globals (`IPlayerInfoManager`).
	#[doc(alias("IPlayerInfoManager", "CPlayerInfoManager"))]
	pub struct PlayerInfoManager(sys::IPlayerInfoManager) = GameServer sdk_raw::interfaces::player_info_manager::VERSION;
}

/// The engine's globals (`CGlobalVars`), which it updates every frame.
///
/// Every accessor reads the current value.
#[doc(alias("CGlobalVars"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlobalVars<'s> {
	raw: NonNull<sys::CGlobalVars>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> PlayerInfoManager<'s> {
	/// The engine's globals, which the game calls `gpGlobals`.
	#[doc(alias("GetGlobalVars", "gpGlobals"))]
	pub fn global_vars(self) -> Option<GlobalVars<'s>> {
		// SAFETY: `Server::new` guarantees the interface is live.
		let globals =
			NonNull::new(unsafe { vcall!(self.as_ptr() => IPlayerInfoManager_GetGlobalVars()) })?;

		Some(GlobalVars {
			raw: globals,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		})
	}

	/// The state of the player occupying an edict, or `None` if the edict
	/// holds no player.
	///
	/// Only the player slots, from 1 up to [`GlobalVars::max_clients`], hold
	/// players, so `None` is returned for any other edict.
	#[doc(alias("GetPlayerInfo"))]
	pub fn player_info(self, edict: Edict<'_>) -> Option<PlayerInfo<'s>> {
		let max_clients = self.global_vars().map_or(0, GlobalVars::max_clients);

		// The game casts the edict's entity to `CBasePlayer` unchecked, so only
		// occupied player slots are passed, as the game's `UTIL_PlayerByIndex`
		// checks before the same cast.
		if edict.is_free() || !(1..=max_clients).contains(&edict.index()) {
			return None;
		}

		// SAFETY: As for `global_vars`, and the edict is an occupied player
		// slot, whose entity, if any, is a player.
		let info = NonNull::new(unsafe {
			vcall!(self.as_ptr() => IPlayerInfoManager_GetPlayerInfo(edict.as_ptr()))
		})?;

		Some(PlayerInfo {
			raw: info,
			_scope: PhantomData,
			_not_thread_safe: PhantomData,
		})
	}
}

/// Declares an accessor that reads one field of the engine's globals.
macro_rules! global_var {
	($(#[$meta:meta])* $name:ident: $Type:ty = $($field:ident).+) => {
		$(#[$meta])*
		pub fn $name(self) -> $Type {
			// SAFETY: The engine's globals are a static of the engine, only
			// written on the main thread. Fields are read without forming
			// references, since the engine writes to them through its own.
			unsafe { (&raw const (*self.raw.as_ptr()).$($field).+).read() }
		}
	};
}

/// The state of a connected player (`IPlayerInfo`).
///
/// The game embeds this in the player entity, so it lives as long as the player.
#[doc(alias("IPlayerInfo", "CPlayerInfo"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerInfo<'s> {
	raw: NonNull<sys::IPlayerInfo>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> PlayerInfo<'s> {
	/// The angles of the player's entity.
	#[doc(alias("GetAbsAngles"))]
	pub fn abs_angles(self) -> QAngle {
		// SAFETY: As for `name`.
		unsafe { raw::abs_angles(self.as_ptr()) }.into()
	}

	/// The origin of the player's entity.
	#[doc(alias("GetAbsOrigin"))]
	pub fn abs_origin(self) -> Vector {
		// SAFETY: As for `name`.
		unsafe { raw::abs_origin(self.as_ptr()) }.into()
	}

	/// The player's armor, which TF2's players do not have.
	#[doc(alias("GetArmorValue", "ArmorValue", "m_ArmorValue"))]
	pub fn armor(self) -> c_int {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_GetArmorValue()) }
	}

	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::IPlayerInfo {
		self.raw.as_ptr()
	}

	/// The player's death count (`m_iDeaths`).
	#[doc(alias("GetDeathCount", "DeathCount", "m_iDeaths"))]
	pub fn death_count(self) -> c_int {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_GetDeathCount()) }
	}

	/// The player's frag count (`m_iFrags`).
	#[doc(alias("GetFragCount", "FragCount", "m_iFrags"))]
	pub fn frag_count(self) -> c_int {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_GetFragCount()) }
	}

	/// The player's health, as
	/// [`Entity::health`](crate::entities::Entity::health) reads it.
	#[doc(alias("GetHealth"))]
	pub fn health(self) -> c_int {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_GetHealth()) }
	}

	/// Whether the player's client is connected.
	#[doc(alias("IsConnected"))]
	pub fn is_connected(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_IsConnected()) }
	}

	/// Whether the player is dead, which the game decides by its life state
	/// being `LIFE_DEAD`, so a dying player is not dead yet.
	#[doc(alias("IsDead"))]
	pub fn is_dead(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_IsDead()) }
	}

	/// Whether the player is a bot.
	#[doc(alias("IsFakeClient"))]
	pub fn is_fake_client(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_IsFakeClient()) }
	}

	/// Whether the player is SourceTV's.
	#[doc(alias("IsHLTV"))]
	pub fn is_hltv(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_IsHLTV()) }
	}

	/// Whether the player is in a vehicle, which TF2 has none of.
	#[doc(alias("IsInAVehicle"))]
	pub fn is_in_vehicle(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_IsInAVehicle()) }
	}

	/// Whether the player is observing, as spectators and dead players do.
	#[doc(alias("IsObserver"))]
	pub fn is_observer(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_IsObserver()) }
	}

	/// Whether the player's entity reports itself as a player.
	#[doc(alias("IsPlayer"))]
	pub fn is_player(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_IsPlayer()) }
	}

	/// Whether the player is the replay system's. Only TF2 reports one;
	/// other games' game DLLs always answer `false`.
	#[doc(alias("IsReplay"))]
	pub fn is_replay(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_IsReplay()) }
	}

	/// The player's maximum health, as the game's `GetMaxHealth` computes
	/// it, which TF2's players compute from their class and attributes.
	///
	/// Unlike [`Entity::max_health`](crate::entities::Entity::max_health),
	/// this calls through an interface every game shares.
	#[doc(alias("GetMaxHealth"))]
	pub fn max_health(self) -> c_int {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_GetMaxHealth()) }
	}

	/// The path of the player's model, such as
	/// `models/player/scout.mdl`.
	#[doc(alias("GetModelName"))]
	pub fn model_name(self) -> Option<CString> {
		// SAFETY: As for `name`. Model names are pooled strings, copied
		// immediately.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IPlayerInfo_GetModelName())) }
	}

	/// The player's name.
	#[doc(alias("GetName"))]
	pub fn name(self) -> Option<CString> {
		// SAFETY: The player is live for `'s`. The name lives in a buffer that
		// renaming overwrites, so it is copied immediately.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IPlayerInfo_GetName())) }
	}

	/// The player's network ID, such as a rendered Steam ID or `BOT`.
	#[doc(alias("GetNetworkIDString"))]
	pub fn network_id(self) -> Option<CString> {
		// SAFETY: As for `name`.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IPlayerInfo_GetNetworkIDString())) }
	}

	/// The largest corner of the player's collision bounds, relative to its
	/// origin, which depends on whether it is ducking or observing.
	#[doc(alias("GetPlayerMaxs"))]
	pub fn player_maxs(self) -> Vector {
		// SAFETY: As for `name`.
		unsafe { raw::player_maxs(self.as_ptr()) }.into()
	}

	/// The smallest corner of the player's collision bounds, relative to its
	/// origin, which depends on whether it is ducking or observing.
	#[doc(alias("GetPlayerMins"))]
	pub fn player_mins(self) -> Vector {
		// SAFETY: As for `name`.
		unsafe { raw::player_mins(self.as_ptr()) }.into()
	}

	/// The index of the player's team.
	#[doc(alias("GetTeamIndex"))]
	pub fn team(self) -> c_int {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_GetTeamIndex()) }
	}

	/// The user ID of the player's client, or `None` if no connected client
	/// owns the player, as for [`ValveEngine::user_id_of_edict`].
	///
	/// [`ValveEngine::user_id_of_edict`]: crate::interfaces::ValveEngine::user_id_of_edict
	#[doc(alias("GetUserID"))]
	pub fn user_id(self) -> Option<UserId> {
		// SAFETY: As for `name`.
		UserId::from_raw(unsafe { vcall!(self.as_ptr() => IPlayerInfo_GetUserID()) }).ok()
	}

	/// The class name of the player's active weapon, such as
	/// `tf_weapon_scattergun`, or `None` if it has none.
	#[doc(alias("GetWeaponName"))]
	pub fn weapon_name(self) -> Option<CString> {
		// SAFETY: As for `name`. The name is copied immediately.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IPlayerInfo_GetWeaponName())) }
	}
}

impl<'s> GlobalVars<'s> {
	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::CGlobalVars {
		self.raw.as_ptr()
	}

	global_var! {
		/// Game time in seconds, which only advances while the game simulates.
		#[doc(alias("curtime"))]
		current_time: f32 = _base.curtime
	}

	global_var! {
		/// Game time elapsed in the current frame, in seconds.
		#[doc(alias("frametime"))]
		frame_time: f32 = _base.frametime
	}

	global_var! {
		/// The number of player slots.
		#[doc(alias("maxClients"))]
		max_clients: c_int = _base.maxClients
	}

	global_var! {
		/// Simulation ticks since the level started.
		#[doc(alias("tickcount"))]
		tick_count: c_int = _base.tickcount
	}

	global_var! {
		/// Seconds per simulation tick.
		interval_per_tick: f32 = _base.interval_per_tick
	}

	/// The name of the current map, such as `ctf_2fort`.
	#[doc(alias("mapname"))]
	pub fn map_name(self) -> Option<CString> {
		// SAFETY: As for `global_var!`. The name is copied immediately.
		unsafe { copy_cstr((&raw const (*self.raw.as_ptr()).mapname.pszValue).read()) }
	}
}
