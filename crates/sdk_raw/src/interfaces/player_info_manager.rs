//! Hand-written ABI of `IPlayerInfoManager` and `IPlayerInfo` that the
//! generated bindings do not describe: the version string it is requested
//! by, from `public/game/server/iplayerinfo.h`, and calls of the `IPlayerInfo`
//! methods that return vectors by value, whose generated signatures differ by
//! ABI.

use crate::vcall_by_value;
use std::ffi::CStr;

/// The version string `IPlayerInfoManager` is exported and requested under.
///
/// This is `INTERFACEVERSION_PLAYERINFOMANAGER` from
/// `public/game/server/iplayerinfo.h`.
#[doc(alias("INTERFACEVERSION_PLAYERINFOMANAGER"))]
pub const VERSION: &CStr = c"PlayerInfoManager002";

/// Declares a function calling an `IPlayerInfo` method that returns a class
/// by value, through [`vcall_by_value!`]: through a hidden result pointer
/// after `this` under the MSVC ABI, since the class has constructors, and in
/// registers under the Itanium ABI, since it is trivially copyable, as the
/// generated signatures give.
macro_rules! by_value {
	($(#[$meta:meta])* $name:ident => $method:ident() -> $Type:ty) => {
		$(#[$meta])*
		///
		/// # Safety
		///
		/// `info` must point to a live `IPlayerInfo` of the loaded game DLL,
		/// whose player is live, and the call must be made on the server's main
		/// thread.
		pub unsafe fn $name(info: *mut sys::IPlayerInfo) -> $Type {
			// SAFETY: The caller upholds the contract, and the method has the
			// shapes `vcall_by_value!` calls.
			unsafe { vcall_by_value!(info => $method() -> $Type) }
		}
	};
}

by_value! {
	/// Calls `IPlayerInfo::GetAbsAngles`, which returns the angles of the
	/// player's entity.
	#[doc(alias("GetAbsAngles"))]
	abs_angles => IPlayerInfo_GetAbsAngles() -> sys::QAngle
}

by_value! {
	/// Calls `IPlayerInfo::GetAbsOrigin`, which returns the origin of the
	/// player's entity.
	#[doc(alias("GetAbsOrigin"))]
	abs_origin => IPlayerInfo_GetAbsOrigin() -> sys::Vector
}

by_value! {
	/// Calls `IPlayerInfo::GetPlayerMaxs`, which returns the largest corner of
	/// the player's collision bounds, relative to its origin.
	#[doc(alias("GetPlayerMaxs"))]
	player_maxs => IPlayerInfo_GetPlayerMaxs() -> sys::Vector
}

by_value! {
	/// Calls `IPlayerInfo::GetPlayerMins`, which returns the smallest corner of
	/// the player's collision bounds, relative to its origin.
	#[doc(alias("GetPlayerMins"))]
	player_mins => IPlayerInfo_GetPlayerMins() -> sys::Vector
}
