//! Resolution in retail TF2's 64-bit Windows `server.dll`, through the start
//! of `CBaseAnimatingOverlay::RestartGesture`.
//!
//! The function starts by looking through the entity's layers for the active
//! one, not dying, that plays the activity, and calls `AddGesture` when there
//! is none and `addifmissing` is set: the signature covers that, up to the
//! call, with the short branch displacements and the call's left open. The
//! offsets of the layers' vector (`m_AnimOverlay`, 0x5E8, and its count,
//! 0x5F8) and of a layer's activity (0x34) stay exact.

#[cfg(test)]
#[path = "../../tests/tf2/animation_layers/windows.rs"]
mod tests;

use crate::sig;
use crate::util::{Image, SignaturePattern, detoured};

/// How many bytes a detour's 5-byte jump overwrites at the start of
/// [`RESTART_GESTURE`]: its first instruction, which saves `rbx`, ends
/// exactly there.
const DETOUR_PATCH: usize = 5;

/// The alignment of function entries in the module.
const FUNCTION_ALIGNMENT: usize = 16;

/// The start of `RestartGesture`, up to its call of `AddGesture`: it saves
/// registers, allocates its stack frame, and walks the layers, a stride of
/// 0x50 bytes, for an active one whose activity is the one asked for.
const RESTART_GESTURE: &[SignaturePattern] = &sig![0x48 0x89 0x5c 0x24 0x10 0x48 0x89 0x6c 0x24 0x18 0x48 0x89 0x74 0x24 0x20 0x57 0x48 0x83 0xec 0x20 0x44 0x8b 0x99 0xf8 0x05 0 0 0x33 0xed 0x48 0x8b 0xf1 0x44 0x8b 0xd5 0x45 0x85 0xdb 0x7e ? 0x48 0x8b 0xb9 0xe8 0x05 0 0 0x90 0x41 0x8b 0xc2 0x48 0x8d 0x1c 0x80 0x48 0x03 0xdb 0x8b 0x04 0xdf 0x24 0x05 0x3c 0x01 0x75 ? 0x8b 0x44 0xdf 0x34 0x83 0xf8 0xff 0x74 ? 0x3b 0xc2 0x74 ? 0x41 0xff 0xc2 0x45 0x3b 0xd3 0x7c ? 0x45 0x84 0xc0 0x74 ? 0x45 0x0f 0xb6 0xc1 0xe8];

/// Resolves `RestartGesture` in the module containing `address`.
///
/// # Safety
///
/// The module containing `address` stays loaded throughout this call.
pub(super) unsafe fn resolve(address: usize) -> Option<usize> {
	// SAFETY: The caller keeps the module loaded throughout this call.
	let image = unsafe { Image::load(address) }.ok()?;

	resolve_image(&image)
}

/// Resolves `RestartGesture` in a snapshot of the module: the one function the
/// signature matches, or, failing that, the one it matches as a detour left it
/// (see [`detoured`]).
fn resolve_image(image: &Image) -> Option<usize> {
	image
		.unique(RESTART_GESTURE, FUNCTION_ALIGNMENT)
		.or_else(|| image.unique(&detoured(RESTART_GESTURE, DETOUR_PATCH), FUNCTION_ALIGNMENT))
}
