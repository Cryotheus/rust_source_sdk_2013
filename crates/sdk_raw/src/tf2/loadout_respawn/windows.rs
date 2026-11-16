//! Resolution in retail TF2's 64-bit Windows `server.dll`, through the
//! prologue of `CTFPlayer::CheckInstantLoadoutRespawn`.
//!
//! The function starts by asking whether the player is alive (`IsAlive`), and
//! then whether their `WorldSpaceCenter` is in a respawn room of their team
//! (`PointInRespawnRoom`, a direct call): the signature covers both virtual
//! calls up to that call, with the branch's target and the call's address left
//! open. Found in the server.dll of 2026-11-15 (IDA, through the job
//! `CGC_RespawnPostLoadoutChange`, whose run calls it once).

#[cfg(test)]
#[path = "../../tests/tf2/loadout_respawn/windows.rs"]
mod tests;

use crate::sig;
use crate::util::{Image, SignaturePattern, detoured};

/// The prologue of `CheckInstantLoadoutRespawn`, up to the call of
/// `PointInRespawnRoom`: `IsAlive` (vtable offset `0x218`), the early return,
/// `WorldSpaceCenter` (vtable offset `0x4e0`), and the call's arguments.
const CHECK_INSTANT_LOADOUT_RESPAWN: &[SignaturePattern] = &sig![0x40 0x56 0x48 0x83 0xec 0x30 0x48 0x8b 0x01 0x48 0x8b 0xf1 0xff 0x90 0x18 0x02 0 0 0x84 0xc0 0x0f 0x84 ? ? ? ? 0x48 0x8b 0x06 0x48 0x8b 0xce 0xff 0x90 0xe0 0x04 0 0 0x41 0xb0 0x01 0x48 0x8b 0xce 0x48 0x8b 0xd0 0xe8];

/// How many bytes a detour's 5-byte jump overwrites at the start of
/// [`CHECK_INSTANT_LOADOUT_RESPAWN`]: its first two instructions, `push rsi`
/// and `sub rsp, 30h`, end at 6.
const DETOUR_PATCH: usize = 6;

/// The alignment of function entries in the module.
const FUNCTION_ALIGNMENT: usize = 16;

/// Resolves `CheckInstantLoadoutRespawn` in the module containing `address`.
///
/// # Safety
///
/// The module containing `address` stays loaded throughout this call.
pub(super) unsafe fn resolve(address: usize) -> Option<usize> {
	// SAFETY: The caller keeps the module loaded throughout this call.
	let image = unsafe { Image::load(address) }.ok()?;

	resolve_image(&image)
}

/// Resolves `CheckInstantLoadoutRespawn` in a snapshot of the module: the one
/// function the signature matches, or, failing that, the one it matches as a
/// detour left it (see [`detoured`]).
fn resolve_image(image: &Image) -> Option<usize> {
	image
		.unique(CHECK_INSTANT_LOADOUT_RESPAWN, FUNCTION_ALIGNMENT)
		.or_else(|| {
			image.unique(
				&detoured(CHECK_INSTANT_LOADOUT_RESPAWN, DETOUR_PATCH),
				FUNCTION_ALIGNMENT,
			)
		})
}
