//! Hand-written ABI of TF2's economy item attributes: values from its headers
//! that the generated bindings omit.

/// The duration `CTFPlayer::AddCustomAttribute` defaults to. The game expires
/// only attributes added with a positive duration, so one added with this
/// lasts until removed.
///
/// This is the default of its `flDuration` in `game/server/tf/tf_player.h`.
#[doc(alias = "AddCustomAttribute")]
pub const DEFAULT_CUSTOM_ATTRIBUTE_DURATION: f32 = -1.0;

/// The attribute definition index that names no attribute,
/// `(attrib_definition_index_t)-1`.
///
/// This is `INVALID_ATTRIB_DEF_INDEX` from
/// `game/shared/econ/econ_item_constants.h`.
pub const INVALID_ATTRIB_DEF_INDEX: sys::attrib_definition_index_t =
	sys::attrib_definition_index_t::MAX;

/// The most runtime attributes the game networks per item.
///
/// This is `MAX_ATTRIBUTES_PER_ITEM` from
/// `game/shared/econ/econ_item_constants.h`.
pub const MAX_ATTRIBUTES_PER_ITEM: usize = 20;
