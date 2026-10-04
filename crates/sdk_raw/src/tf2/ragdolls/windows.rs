//! Resolution in retail TF2's 64-bit Windows `server.dll`, through the
//! prologue of `CreateServerRagdoll` and the calls to it from two virtual
//! methods of `CBaseCombatCharacter`, which must agree.
//!
//! `BecomeRagdoll` and `BecomeRagdollBoogie` each call `CreateServerRagdoll`
//! directly, with their argument setup interleaved with other instructions.
//! Their functions are found in `CBaseCombatCharacter`'s primary vtable,
//! through its run-time type information, and each must hold a `call rel32`
//! to the function the prologue matched within its first [`CALLER_WINDOW`]
//! bytes, before the other's function starts. The table of
//! `CBaseCombatCharacter` itself is used rather than a player's, since no
//! entity has that exact class, so no plugin hooks its slots.

#[cfg(test)]
#[path = "../../tests/tf2/ragdolls/windows.rs"]
mod tests;

use crate::abi::VTABLE_SLOT_SIZE;
use crate::tf2::damage::{DMG_CRUSH, DMG_VEHICLE};
use crate::util::{Image, SignaturePattern, is_exact, relative, word_at};
use crate::{sig, vtable_slot};
use std::mem::offset_of;

// The prologue tests `info.m_bitsDamageType & (DMG_CRUSH | DMG_VEHICLE)`, as
// `CreateServerRagdoll` first does, with `info` in `r8`.
const _: () = {
	assert!(is_exact(
		CREATE_SERVER_RAGDOLL,
		DAMAGE_TYPE_DISPLACEMENT,
		offset_of!(sys::CTakeDamageInfo, m_bitsDamageType) as u8
	));
	assert!(offset_of!(sys::CTakeDamageInfo, m_bitsDamageType) < 0x80);
	assert!(is_exact(
		CREATE_SERVER_RAGDOLL,
		DAMAGE_TYPE_DISPLACEMENT + 1,
		(DMG_CRUSH | DMG_VEHICLE) as u8
	));
};

// The slots match the retail `server.dll`, in whose `CBaseCombatCharacter`,
// `CTFPlayer` and `CTFBot` vtables they hold the same two functions.
const _: () = assert!(BECOME_RAGDOLL_SLOT == 302 && BECOME_RAGDOLL_BOOGIE_SLOT == 304);

/// `CBaseCombatCharacter::BecomeRagdollBoogie` in the primary vtable.
const BECOME_RAGDOLL_BOOGIE_SLOT: usize = vtable_slot!(
	sys::CBaseCombatCharacter__bindgen_vtable,
	CBaseCombatCharacter_BecomeRagdollBoogie
);

/// `CBaseCombatCharacter::BecomeRagdoll` in the primary vtable.
const BECOME_RAGDOLL_SLOT: usize = vtable_slot!(
	sys::CBaseCombatCharacter__bindgen_vtable,
	CBaseCombatCharacter_BecomeRagdoll
);

/// The opcode of `call rel32`.
const CALL: u8 = 0xe8;

/// How far into each caller its call to `CreateServerRagdoll` may start. The
/// retail calls start 0x1a3 bytes into `BecomeRagdoll` and 0x94 bytes into
/// `BecomeRagdollBoogie`.
const CALLER_WINDOW: usize = 0x200;

/// The class whose primary vtable holds the callers.
const CLASS: &str = "CBaseCombatCharacter";

/// The prologue of `CreateServerRagdoll`, up to its test of the damage type:
/// it saves registers, allocates its stack frame through `__chkstk`, and
/// tests `info.m_bitsDamageType`.
const CREATE_SERVER_RAGDOLL: &[SignaturePattern] = &sig![0x48 0x89 0x5c 0x24 0x20 0x89 0x54 0x24 0x10 0x55 0x56 0x57 0x41 0x54 0x41 0x55 0x41 0x56 0x41 0x57 0x48 0x8d 0xac 0x24 ? ? ? ? 0xb8 ? ? ? ? 0xe8 ? ? ? ? 0x48 0x2b 0xe0 0x41 0xf6 0x40 0x3c 0x11];

/// Where the displacement of [`CREATE_SERVER_RAGDOLL`]'s test of
/// `info.m_bitsDamageType` starts, followed by its mask.
const DAMAGE_TYPE_DISPLACEMENT: usize = 0x2c;

/// The alignment of function entries in the module.
const FUNCTION_ALIGNMENT: usize = 16;

/// Whether the function at `caller` calls `target` within its first
/// [`CALLER_WINDOW`] bytes, before `other` if it starts after `caller`.
fn calls(image: &Image, caller: usize, other: usize, target: usize) -> bool {
	let mut end = caller.saturating_add(CALLER_WINDOW);

	if other > caller {
		end = end.min(other);
	}

	let Some(bytes) = image.read(caller, end - caller) else {
		return false;
	};

	bytes
		.iter()
		.enumerate()
		.any(|(offset, &byte)| byte == CALL && relative(caller, bytes, offset + 1) == Some(target))
}

/// Resolves `CreateServerRagdoll` in the module containing `address`.
///
/// # Safety
///
/// The module containing `address` stays loaded throughout this call.
pub(super) unsafe fn resolve(address: usize) -> Option<usize> {
	// SAFETY: The caller keeps the module loaded throughout this call.
	let image = unsafe { Image::load(address) }.ok()?;

	resolve_image(&image)
}

/// Resolves `CreateServerRagdoll` in a snapshot of the module.
fn resolve_image(image: &Image) -> Option<usize> {
	let table = image.primary_vtable(CLASS, BECOME_RAGDOLL_BOOGIE_SLOT)?;

	resolve_with_table(image, table)
}

/// Resolves `CreateServerRagdoll` in a snapshot of the module, with the
/// callers in the vtable at `table`.
fn resolve_with_table(image: &Image, table: usize) -> Option<usize> {
	let target = image.unique(CREATE_SERVER_RAGDOLL, FUNCTION_ALIGNMENT)?;
	let become_ragdoll = slot(image, table, BECOME_RAGDOLL_SLOT)?;
	let boogie = slot(image, table, BECOME_RAGDOLL_BOOGIE_SLOT)?;

	// Independent native callers must agree on the target.
	(become_ragdoll != boogie
		&& calls(image, become_ragdoll, boogie, target)
		&& calls(image, boogie, become_ragdoll, target))
	.then_some(target)
}

/// The function in `slot` of the vtable at `table`, if it lies in the image's
/// code. A hook's trampoline outside the image fails resolution.
fn slot(image: &Image, table: usize, slot: usize) -> Option<usize> {
	let entry = table.checked_add(slot.checked_mul(VTABLE_SLOT_SIZE)?)?;
	let function = word_at(image.read(entry, VTABLE_SLOT_SIZE)?, 0)?;

	image.executable(function).then_some(function)
}
