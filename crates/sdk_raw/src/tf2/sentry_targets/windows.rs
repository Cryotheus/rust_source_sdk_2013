//! Resolution in retail TF2's 64-bit Windows `server.dll`, through the
//! prologue of `CObjectSentrygun::FindTarget`.
//!
//! The function starts by clearing `m_bPlayerControlled` and setting
//! `m_flShieldFadeTime` three seconds ahead, if the sentry was aimed by its
//! builder: the signature covers that, up to the call that follows it, with the
//! stack frame's size and the addresses of the globals it reads left open. The
//! offsets of the two members are the generated layout's, which the
//! assertions below tie to the signature.

#[cfg(test)]
#[path = "../../tests/tf2/sentry_targets/windows.rs"]
mod tests;

use crate::sig;
use crate::util::{Image, SignaturePattern, exact_u32};
use std::mem::offset_of;

// The signature's two member offsets are those the layout gives.
const _: () = {
	assert!(
		matches!(exact_u32(FIND_TARGET, PLAYER_CONTROLLED), Some(offset) if offset as usize == offset_of!(sys::CObjectSentrygun, m_bPlayerControlled))
	);
	assert!(
		matches!(exact_u32(FIND_TARGET, SHIELD_FADE_TIME), Some(offset) if offset as usize == offset_of!(sys::CObjectSentrygun, m_flShieldFadeTime))
	);
};

/// The alignment of function entries in the module.
const FUNCTION_ALIGNMENT: usize = 16;

/// The prologue of `FindTarget`, up to the call after it stores
/// `m_flShieldFadeTime` and clears `m_bPlayerControlled`: it saves registers,
/// allocates its stack frame, and tests `m_bPlayerControlled` (`r15` holds the
/// address of the member).
const FIND_TARGET: &[SignaturePattern] = &sig![0x40 0x55 0x56 0x41 0x57 0x48 0x8d 0xac 0x24 ? ? ? ? 0x48 0x81 0xec ? ? ? ? 0x4c 0x8d 0xb9 0xa0 0x0d 0 0 0x48 0x8b 0xf1 0x41 0x80 0x3f 0 0x74 ? 0x48 0x8b 0x05 ? ? ? ? 0x49 0x8b 0xd7 0xf3 0x0f 0x10 0x40 0x0c 0xf3 0x0f 0x58 0x05 ? ? ? ? 0xf3 0x0f 0x11 0x81 0xa8 0x0d 0 0 0xe8 ? ? ? ? 0x41 0xc6 0x07 0];

/// Where the displacement of `m_bPlayerControlled` starts in [`FIND_TARGET`].
const PLAYER_CONTROLLED: usize = 23;

/// Where the displacement of `m_flShieldFadeTime` starts in [`FIND_TARGET`].
const SHIELD_FADE_TIME: usize = 63;

/// Resolves `FindTarget` in the module containing `address`.
///
/// # Safety
///
/// The module containing `address` stays loaded throughout this call.
pub(super) unsafe fn resolve(address: usize) -> Option<usize> {
	// SAFETY: The caller keeps the module loaded throughout this call.
	let image = unsafe { Image::load(address) }.ok()?;

	resolve_image(&image)
}

/// Resolves `FindTarget` in a snapshot of the module: the one function the
/// signature matches.
fn resolve_image(image: &Image) -> Option<usize> {
	image.unique(FIND_TARGET, FUNCTION_ALIGNMENT)
}
