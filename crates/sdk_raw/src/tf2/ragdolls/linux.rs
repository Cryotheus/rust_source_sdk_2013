//! Resolution in retail TF2's 64-bit Linux `server_srv.so`, through the
//! mangled symbol of an unstripped build.

#[cfg(test)]
#[path = "../../tests/tf2/ragdolls/linux.rs"]
mod tests;

use crate::util::elf::LoadedElf;

/// `CreateServerRagdoll(CBaseAnimating *, int, const CTakeDamageInfo &, int,
/// bool)`.
const CREATE_SERVER_RAGDOLL: &[u8] =
	b"_Z19CreateServerRagdollP14CBaseAnimatingiRK15CTakeDamageInfoib";

/// Resolves `CreateServerRagdoll` from the symbols of the module containing
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
	let (function, _) = elf.resolve(CREATE_SERVER_RAGDOLL)?;

	Some(function)
}
