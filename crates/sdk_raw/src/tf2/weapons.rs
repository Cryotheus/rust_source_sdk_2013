//! Hand-written ABI of TF2's weapons, and of the players holding them.
//!
//! A `CBaseEntity *` to a TF2 player is also a `CTFPlayer *`, and one to a
//! TF2 weapon is also a `CTFWeaponBase *` and a `CBaseCombatWeapon *`, the
//! types the generated vtables of those classes take: every class from them
//! down to `CBaseEntity` starts with its primary base, as this module asserts
//! against the generated layouts. The Itanium bindings describe `CBasePlayer`,
//! `CBaseCombatWeapon` and `CBaseAnimating` as opaque blobs, whose single,
//! polymorphic primary bases that ABI also places first.

use std::mem::offset_of;

const _: () = {
	assert!(
		offset_of!(sys::CTFPlayer, _base) == 0
			&& offset_of!(sys::CBaseMultiplayerPlayer, _base) == 0
			&& offset_of!(sys::CAI_ExpresserHost<sys::CBasePlayer>, _base) == 0
			&& offset_of!(sys::CBaseCombatCharacter, _base) == 0
			&& offset_of!(sys::CBaseFlex, _base) == 0
			&& offset_of!(sys::CBaseAnimatingOverlay, _base) == 0
	);

	assert!(offset_of!(sys::CTFWeaponBase, _base) == 0 && offset_of!(sys::CEconEntity, _base) == 0);

	#[cfg(target_os = "windows")]
	assert!(
		offset_of!(sys::CBasePlayer, _base) == 0
			&& offset_of!(sys::CBaseCombatWeapon, _base) == 0
			&& offset_of!(sys::CBaseAnimating, _base) == 0
	);
};

/// The item definition index that names no item,
/// `(item_definition_index_t)-1`.
///
/// This is `INVALID_ITEM_DEF_INDEX` from
/// `game/shared/econ/econ_item_constants.h`.
pub const INVALID_ITEM_DEF_INDEX: sys::item_definition_index_t = sys::item_definition_index_t::MAX;
