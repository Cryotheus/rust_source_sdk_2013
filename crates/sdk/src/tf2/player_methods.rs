//! Typed calls of `CTFPlayer`'s native script methods, for the condition,
//! effect, meter and taunt wrappers.
//!
//! Each call goes through the game's typed VScript adapter, as
//! [`script_binding::call`] describes, and reads the result's union member
//! for the type the call checked.

use crate::entities::Entity;
use crate::tf2::script_binding::{self, BindingError};
use crate::{Game, Server};
use sdk_raw::tf2::script_binding::{BOOL, FLOAT, INT, VOID};
use std::ffi::CStr;

/// The class whose script bindings declare the player's methods.
const CLASS: &CStr = c"CTFPlayer";

/// Calls a native `CTFPlayer` method that returns a `bool`.
///
/// # Safety
///
/// As for [`script_binding::call`], with `player` a `CTFPlayer`.
pub(crate) unsafe fn call_bool(
	player: Entity<'_>,
	method: &CStr,
	arguments: &mut [sys::ScriptVariant_t],
) -> Result<bool, BindingError> {
	// SAFETY: The caller upholds `call`'s contract.
	let result = unsafe { script_binding::call(player, CLASS, method, arguments, BOOL) }?;

	// SAFETY: `call` checked that the adapter returned a `FIELD_BOOLEAN`
	// variant, which it assigns through the bool member.
	Ok(unsafe { result.__bindgen_anon_1.m_bool })
}

/// Calls a native `CTFPlayer` method that returns an `f32`.
///
/// # Safety
///
/// As for [`call_bool`].
pub(crate) unsafe fn call_float(
	player: Entity<'_>,
	method: &CStr,
	arguments: &mut [sys::ScriptVariant_t],
) -> Result<f32, BindingError> {
	// SAFETY: The caller upholds `call`'s contract.
	let result = unsafe { script_binding::call(player, CLASS, method, arguments, FLOAT) }?;

	// SAFETY: `call` checked that the adapter returned a `FIELD_FLOAT`
	// variant, which it assigns through the float member.
	Ok(unsafe { result.__bindgen_anon_1.m_float })
}

/// Calls a native `CTFPlayer` method that returns an `int`.
///
/// # Safety
///
/// As for [`call_bool`].
pub(crate) unsafe fn call_int(
	player: Entity<'_>,
	method: &CStr,
	arguments: &mut [sys::ScriptVariant_t],
) -> Result<i32, BindingError> {
	// SAFETY: The caller upholds `call`'s contract.
	let result = unsafe { script_binding::call(player, CLASS, method, arguments, INT) }?;

	// SAFETY: `call` checked that the adapter returned a `FIELD_INTEGER`
	// variant, which it assigns through the int member.
	Ok(unsafe { result.__bindgen_anon_1.m_int })
}

/// Calls a native `CTFPlayer` method that returns nothing.
///
/// # Safety
///
/// As for [`call_bool`].
pub(crate) unsafe fn call_void(
	player: Entity<'_>,
	method: &CStr,
	arguments: &mut [sys::ScriptVariant_t],
) -> Result<(), BindingError> {
	// SAFETY: The caller upholds `call`'s contract.
	unsafe { script_binding::call(player, CLASS, method, arguments, VOID) }.map(drop)
}

/// Whether `player` is a TF2 player: the server runs TF2, and the entity's
/// class name is `player`, which only `CTFPlayer` registers.
pub(crate) fn is_tf_player(server: Server<'_>, player: Entity<'_>) -> bool {
	server.game() == Game::TeamFortress2 && player.class_name() == c"player"
}
