//! Resolution in retail TF2's 64-bit Windows `server.dll`, through the start
//! of `CWeaponMedigun::AllowedToHealTarget`.
//!
//! The function starts by finding the Medi Gun's owner through
//! `m_hOwnerEntity`, checks that the owner and the target are players and the
//! target alive (`IsPlayer` and `IsAlive`, through the vtable), and asks the
//! target's active weapon for `weapon_blocks_healing`: the signature covers
//! that, up to the comparison of the attribute's value, with the branch
//! displacements, the entity list's address, the calls and the attribute name's
//! address left open. The member offset is the generated layout's, which the
//! assertion below ties to the signature.

#[cfg(test)]
#[path = "../../tests/tf2/medigun_targets/windows.rs"]
mod tests;

use crate::sig;
use crate::util::{Image, SignaturePattern, detoured, exact_u32};
use std::mem::offset_of;

// The signature's member offset is the one the layout gives.
const _: () = assert!(
	matches!(exact_u32(ALLOWED_TO_HEAL_TARGET, OWNER_ENTITY), Some(offset) if offset as usize == offset_of!(sys::CBaseEntity, m_hOwnerEntity))
);

/// The start of `AllowedToHealTarget`, up to the comparison of the target's
/// `weapon_blocks_healing`: it saves registers, allocates its stack frame,
/// reads `m_hOwnerEntity` and resolves it in the entity list, and calls
/// `IsPlayer` and `IsAlive` through the vtable.
const ALLOWED_TO_HEAL_TARGET: &[SignaturePattern] = &sig![0x48 0x89 0x5c 0x24 0x10 0x48 0x89 0x6c 0x24 0x18 0x48 0x89 0x74 0x24 0x20 0x57 0x48 0x83 0xec 0x30 0x48 0x8b 0xda 0x48 0x8b 0xf1 0x8b 0x91 0xe0 0x02 0 0 0x85 0xd2 0x0f 0x84 ? ? ? ? 0xb8 0xff 0x1f 0 0 0x83 0xfa 0xff 0x74 0x03 0x0f 0xb7 0xc2 0x48 0x8b 0x3d ? ? ? ? 0x8b 0xc8 0x48 0x83 0xc7 0x08 0x48 0xc1 0xe1 0x05 0x48 0x03 0xf9 0x0f 0x84 ? ? ? ? 0xc1 0xea 0x10 0x39 0x57 0x08 0x0f 0x85 ? ? ? ? 0x48 0x8b 0x3f 0x48 0x85 0xff 0x0f 0x84 ? ? ? ? 0x48 0x8b 0x07 0x48 0x8b 0xcf 0xff 0x90 0x98 0x02 0 0 0x84 0xc0 0x0f 0x84 ? ? ? ? 0x48 0x85 0xdb 0x0f 0x84 ? ? ? ? 0x48 0x8b 0x03 0x48 0x8b 0xcb 0xff 0x90 0x98 0x02 0 0 0x48 0x8b 0xcb 0x84 0xc0 0x0f 0x84 ? ? ? ? 0x48 0x8b 0x03 0xff 0x90 0x98 0x02 0 0 0x84 0xc0 0x74 ? 0x48 0x8b 0x03 0x48 0x8b 0xcb 0xff 0x90 0x18 0x02 0 0 0x84 0xc0 0x74 ? 0x48 0x8b 0xcb 0xe8 ? ? ? ? 0x45 0x33 0xc9 0xc6 0x44 0x24 0x20 0x01 0x4c 0x8b 0xc0 0x48 0x8d 0x15 ? ? ? ? 0x33 0xc9 0xe8 ? ? ? ? 0x83 0xf8 0x01];

/// How many bytes a detour's 5-byte jump overwrites at the start of
/// [`ALLOWED_TO_HEAL_TARGET`]: its first instruction, which saves `rbx`, ends
/// exactly there.
const DETOUR_PATCH: usize = 5;

/// The alignment of function entries in the module.
const FUNCTION_ALIGNMENT: usize = 16;

/// Where the displacement of `m_hOwnerEntity` starts in
/// [`ALLOWED_TO_HEAL_TARGET`].
const OWNER_ENTITY: usize = 28;

/// Resolves `AllowedToHealTarget` in the module containing `address`.
///
/// # Safety
///
/// The module containing `address` stays loaded throughout this call.
pub(super) unsafe fn resolve(address: usize) -> Option<usize> {
	// SAFETY: The caller keeps the module loaded throughout this call.
	let image = unsafe { Image::load(address) }.ok()?;

	resolve_image(&image)
}

/// Resolves `AllowedToHealTarget` in a snapshot of the module: the one function
/// the signature matches, or, failing that, the one it matches as a detour
/// left it (see [`detoured`]), as KHook leaves it after an earlier load of a
/// plugin hooked it.
fn resolve_image(image: &Image) -> Option<usize> {
	image
		.unique(ALLOWED_TO_HEAL_TARGET, FUNCTION_ALIGNMENT)
		.or_else(|| {
			image.unique(
				&detoured(ALLOWED_TO_HEAL_TARGET, DETOUR_PATCH),
				FUNCTION_ALIGNMENT,
			)
		})
}
