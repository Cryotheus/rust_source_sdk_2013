//! TF2's facts about the ammo packs players drop (`CTFAmmoPack`, from
//! `game/server/tf/tf_ammo_pack.h`): the kinds of packs (`AmmoPackType_t`),
//! and where a pack keeps its kind, which it does not network.

use std::ffi::c_int;

/// `AP_CHRISTMAS`: a Christmas gift, which also counts toward a gift
/// achievement.
pub const AP_CHRISTMAS: c_int = 2;

/// `AP_HALLOWEEN`: a Halloween map's pumpkin, which also grants critical hits
/// for a few seconds to the first player to pick it up.
pub const AP_HALLOWEEN: c_int = 1;

/// `AP_NORMAL`: a pack that gives ammo, or health from a Heavy's lunchbox.
pub const AP_NORMAL: c_int = 0;

/// The bytes from a pack's kind (`m_PackType`), an `AmmoPackType_t`, to its
/// networked initial velocity (`m_vecInitialVelocity`), which `CTFAmmoPack`
/// declares right after the kind. The generated bindings leave `CTFAmmoPack`
/// opaque, so this comes from the class's declaration; both ABIs lay out its
/// 4-byte enum and the vector of floats after it without padding.
pub const PACK_TYPE_BEFORE_INITIAL_VELOCITY: usize = 4;
