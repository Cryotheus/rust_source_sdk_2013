//! Hand-written ABI of TF2's wearables: values from its headers that the
//! generated bindings omit, and layout facts they rely on.
//!
//! A `CBaseEntity *` to a TF2 wearable is also a `CEconWearable *`, the type
//! `EquipWearable` and `RemoveWearable` take: every class from `CTFWearable`
//! down to `CBaseEntity` starts with its primary base, as this module asserts
//! against the generated layouts. The Itanium bindings describe
//! `CEconWearable` and `CBaseAnimating` as opaque blobs, whose single,
//! polymorphic primary bases that ABI also places first.

use std::mem::offset_of;

const _: () = {
	assert!(offset_of!(sys::CTFWearable, _base) == 0 && offset_of!(sys::CEconEntity, _base) == 0);

	#[cfg(target_os = "windows")]
	assert!(
		offset_of!(sys::CEconWearable, _base) == 0 && offset_of!(sys::CBaseAnimating, _base) == 0
	);
};

/// The most entries of a player's wearable list (`m_hMyWearables`) that the
/// game networks.
///
/// This is `MAX_WEARABLES_SENT_FROM_SERVER` from
/// `game/shared/econ/econ_wearable.h`, which TF2 sets to
/// `LOADOUT_MAX_WEARABLES_COUNT` from `game/shared/tf/tf_item_constants.h`.
#[doc(alias = "LOADOUT_MAX_WEARABLES_COUNT")]
pub const MAX_WEARABLES_SENT_FROM_SERVER: usize = 8;
