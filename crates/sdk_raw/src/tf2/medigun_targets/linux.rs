//! Resolution in retail TF2's 64-bit Linux `server_srv.so`, through the
//! mangled symbol of an unstripped build.

#[cfg(test)]
#[path = "../../tests/tf2/medigun_targets/linux.rs"]
mod tests;

use crate::util::elf::LoadedElf;

/// `CWeaponMedigun::AllowedToHealTarget(CBaseEntity*)`.
const ALLOWED_TO_HEAL_TARGET: &[u8] = b"_ZN14CWeaponMedigun19AllowedToHealTargetEP11CBaseEntity";

/// Resolves `AllowedToHealTarget` from the symbols of the module containing
/// `address`.
///
/// # Safety
///
/// The module containing `address` stays loaded, with its image mappings
/// unchanged, throughout this call.
pub(super) unsafe fn resolve(address: usize) -> Option<usize> {
	// SAFETY: The caller guarantees the module remains loaded and its image
	// mappings remain valid throughout this snapshot.
	let elf = unsafe { LoadedElf::at(address) }.ok()?;
	let (function, _) = elf.resolve(ALLOWED_TO_HEAL_TARGET)?;

	Some(function)
}
