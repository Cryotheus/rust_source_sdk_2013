//! `IPlayerInfoManager`, which exposes player state and the engine's globals.

use crate::edicts::Edict;
use crate::ffi::{NotThreadSafe, copy_cstr, vcall};
use crate::players::UserId;
use std::ffi::{CString, c_int};
use std::marker::PhantomData;
use std::ptr::NonNull;

interface! {
	/// Exposes player state and the engine's globals (`IPlayerInfoManager`).
	#[doc(alias = "IPlayerInfoManager")]
	pub struct PlayerInfoManager(sys::IPlayerInfoManager) = GameServer c"PlayerInfoManager002";
}

impl<'s> PlayerInfoManager<'s> {
	/// The engine's globals, which the game calls `gpGlobals`.
	#[doc(alias = "GetGlobalVars")]
	#[doc(alias = "gpGlobals")]
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
	#[doc(alias = "GetPlayerInfo")]
	pub fn player_info(self, edict: Edict<'_>) -> Option<PlayerInfo<'s>> {
		if edict.is_free() {
			return None;
		}

		// SAFETY: As for `global_vars`, and the edict is occupied.
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

/// The engine's globals (`CGlobalVars`), which it updates every frame.
///
/// Every accessor reads the current value.
#[doc(alias = "CGlobalVars")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlobalVars<'s> {
	raw: NonNull<sys::CGlobalVars>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

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

impl<'s> GlobalVars<'s> {
	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::CGlobalVars {
		self.raw.as_ptr()
	}

	global_var! {
		/// Game time in seconds, which only advances while the game simulates.
		#[doc(alias = "curtime")]
		current_time: f32 = _base.curtime
	}

	global_var! {
		/// Game time elapsed in the current frame.
		frame_time: f32 = _base.frametime
	}

	global_var! {
		/// The number of player slots.
		max_clients: c_int = _base.maxClients
	}

	global_var! {
		/// Simulation ticks since the level started.
		tick_count: c_int = _base.tickcount
	}

	global_var! {
		/// Seconds per simulation tick.
		interval_per_tick: f32 = _base.interval_per_tick
	}

	/// The name of the current map, such as `ctf_2fort`.
	#[doc(alias = "mapname")]
	pub fn map_name(self) -> Option<CString> {
		// SAFETY: As for `global_var!`. The name is copied immediately.
		unsafe { copy_cstr((&raw const (*self.raw.as_ptr()).mapname.pszValue).read()) }
	}
}

/// The state of a connected player (`IPlayerInfo`).
///
/// The game embeds this in the player entity, so it lives as long as the player.
#[doc(alias = "IPlayerInfo")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlayerInfo<'s> {
	raw: NonNull<sys::IPlayerInfo>,
	_scope: PhantomData<&'s ()>,
	_not_thread_safe: NotThreadSafe,
}

impl<'s> PlayerInfo<'s> {
	/// Returns the native pointer for low-level interop.
	pub const fn as_ptr(self) -> *mut sys::IPlayerInfo {
		self.raw.as_ptr()
	}

	/// The player's name.
	#[doc(alias = "GetName")]
	pub fn name(self) -> Option<CString> {
		// SAFETY: The player is live for `'s`. The name lives in a buffer that
		// renaming overwrites, so it is copied immediately.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IPlayerInfo_GetName())) }
	}

	#[doc(alias = "GetUserID")]
	pub fn user_id(self) -> Option<UserId> {
		// SAFETY: As for `name`.
		UserId::from_raw(unsafe { vcall!(self.as_ptr() => IPlayerInfo_GetUserID()) }).ok()
	}

	/// The player's network ID, such as a rendered Steam ID or `BOT`.
	#[doc(alias = "GetNetworkIDString")]
	pub fn network_id(self) -> Option<CString> {
		// SAFETY: As for `name`.
		unsafe { copy_cstr(vcall!(self.as_ptr() => IPlayerInfo_GetNetworkIDString())) }
	}

	/// The index of the player's team.
	#[doc(alias = "GetTeamIndex")]
	pub fn team(self) -> c_int {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_GetTeamIndex()) }
	}

	#[doc(alias = "IsDead")]
	pub fn is_dead(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_IsDead()) }
	}

	/// Whether the player is a bot.
	#[doc(alias = "IsFakeClient")]
	pub fn is_fake_client(self) -> bool {
		// SAFETY: As for `name`.
		unsafe { vcall!(self.as_ptr() => IPlayerInfo_IsFakeClient()) }
	}
}
