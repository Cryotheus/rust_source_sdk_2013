//! TF2 player conditions, including the object-backed conditions in
//! `tf_condition.cpp` and the remaining `CTFPlayerShared` conditions.
//!
//! Calls the game's native methods through their typed binding descriptors.
//! No script VM is needed, and no condition bits are written directly: the
//! game runs its normal add/remove notifications, durations and cleanup.

use crate::entities::Entity;
use crate::{Game, Server};
use sdk_raw::tf2::conditions as raw;
use sdk_raw::tf2::script_binding::BindingError;
use std::ptr::NonNull;

/// A valid `ETFCond` identifier from TF2's `tf_shareddefs.h`.
/// All identifiers, including less common conditions, are available through
/// [`crate::sys`] as `ETFCond_TF_COND_*` constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[doc(alias("ETFCond"))]
pub struct Condition(i32);

impl Condition {
	/// A Sniper aiming or a Heavy using the minigun.
	#[doc(alias("TF_COND_AIMING"))]
	pub const AIMING: Self = Self(sys::ETFCond_TF_COND_AIMING);

	/// Bleeding.
	#[doc(alias("TF_COND_BLEEDING"))]
	pub const BLEEDING: Self = Self(sys::ETFCond_TF_COND_BLEEDING);

	/// On fire.
	#[doc(alias("TF_COND_BURNING"))]
	pub const BURNING: Self = Self(sys::ETFCond_TF_COND_BURNING);

	/// The critical boost reserved for the Kritzkrieg and revenge crits.
	#[doc(alias("TF_COND_CRITBOOSTED"))]
	pub const CRITBOOSTED: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED);

	/// Wearing a Spy's disguise.
	#[doc(alias("TF_COND_DISGUISED"))]
	pub const DISGUISED: Self = Self(sys::ETFCond_TF_COND_DISGUISED);

	/// Invulnerability, as from a Medic's ÜberCharge.
	#[doc(alias("TF_COND_INVULNERABLE"))]
	pub const INVULNERABLE: Self = Self(sys::ETFCond_TF_COND_INVULNERABLE);

	/// Marked for death: TF2 promotes non-critical damage to the player to a
	/// mini critical hit.
	#[doc(alias("TF_COND_MARKEDFORDEATH"))]
	pub const MARKED_FOR_DEATH: Self = Self(sys::ETFCond_TF_COND_MARKEDFORDEATH);

	/// A movement speed boost.
	#[doc(alias("TF_COND_SPEED_BOOST"))]
	pub const SPEED_BOOST: Self = Self(sys::ETFCond_TF_COND_SPEED_BOOST);

	/// A cloaked Spy.
	#[doc(alias("TF_COND_STEALTHED"))]
	pub const STEALTHED: Self = Self(sys::ETFCond_TF_COND_STEALTHED);

	/// Zoomed in through a scope.
	#[doc(alias("TF_COND_ZOOMED"))]
	pub const ZOOMED: Self = Self(sys::ETFCond_TF_COND_ZOOMED);

	/// Validates a raw identifier. Returns `None` for negative values and
	/// values at or above the `TF_COND_LAST` sentinel.
	pub const fn from_raw(raw: sys::ETFCond) -> Option<Self> {
		if raw >= 0 && raw < sys::ETFCond_TF_COND_LAST {
			Some(Self(raw))
		} else {
			None
		}
	}

	/// The raw `ETFCond` value.
	pub const fn to_raw(self) -> sys::ETFCond {
		self.0
	}
}

/// Condition lifetime. The game may keep a longer existing duration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConditionDuration(f32);

impl ConditionDuration {
	/// No expiry: TF2's `PERMANENT_CONDITION`, passed as -1 seconds.
	#[doc(alias("PERMANENT_CONDITION"))]
	pub const PERMANENT: Self = Self(raw::PERMANENT_CONDITION);

	/// A finite, nonnegative number of seconds. Zero expires on a subsequent
	/// game update; it does not mean permanent.
	pub fn seconds(seconds: f32) -> Option<Self> {
		(seconds.is_finite() && seconds >= 0.0).then_some(Self(seconds))
	}

	/// The duration passed to TF2: seconds, or -1 for [`Self::PERMANENT`].
	pub const fn as_raw(self) -> f32 {
		self.0
	}
}

/// A condition operation is unavailable for this game, entity, or binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConditionError {
	/// The server is not running TF2, or the entity's class name is not
	/// `player`.
	#[error("conditions require a TF2 player")]
	NotTfPlayer,

	/// The player's script class descriptors lack the native method, or its
	/// signature differs from the SDK's.
	#[error("the game does not expose the expected native condition method")]
	UnsupportedMethod,

	/// The native method's binding adapter reported failure.
	#[error("the native condition method rejected its arguments")]
	Rejected,
}

impl From<BindingError> for ConditionError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// One player's conditions within the current engine callback.
///
/// Methods fail with [`ConditionError::UnsupportedMethod`] when the game
/// lacks the expected native method, and [`ConditionError::Rejected`] when
/// the method's binding reports failure.
#[derive(Debug, Clone, Copy)]
pub struct PlayerConditions<'s> {
	player: Entity<'s>,
}

impl<'s> PlayerConditions<'s> {
	/// Wraps `player` for condition calls. Fails with
	/// [`ConditionError::NotTfPlayer`] unless the server runs TF2 and
	/// `player`'s class name is `player`.
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, ConditionError> {
		if server.game() != Game::TeamFortress2 || player.class_name() != c"player" {
			return Err(ConditionError::NotTfPlayer);
		}
		Ok(Self { player })
	}

	/// Adds a condition without a provider, using TF2's normal duration rules.
	/// Returns whether it is active afterwards. TF2 can refuse additions, for
	/// example on dead players or outside the competitive match summary.
	#[doc(alias("AddCond", "AddCondEx"))]
	pub fn add(
		self,
		condition: Condition,
		duration: ConditionDuration,
	) -> Result<bool, ConditionError> {
		// SAFETY: A TF2 `player` is a CTFPlayer, live on the main thread for
		// this callback, whose game module stays loaded. The checked native
		// method only adds a validated condition; null means no provider. Its
		// condition effects respect the callback's deferred entity-deletion
		// contract.
		unsafe {
			raw::add_cond_ex(
				self.raw_player(),
				condition.0,
				duration.0,
				std::ptr::null_mut(),
			)
		}?;
		self.in_cond(condition)
	}

	/// Checks both the object-backed condition list and all extended bitfields.
	#[doc(alias("InCond"))]
	pub fn in_cond(self, condition: Condition) -> Result<bool, ConditionError> {
		// SAFETY: As for `add`. This is the native read-only query on a
		// validated player and condition.
		Ok(unsafe { raw::in_cond(self.raw_player(), condition.0) }?)
	}

	/// The player whose conditions these are.
	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// The player's pointer, for the native condition methods.
	fn raw_player(self) -> NonNull<sys::CBaseEntity> {
		// SAFETY: An entity's pointer is never null.
		unsafe { NonNull::new_unchecked(self.player.as_ptr()) }
	}

	/// Removes a condition. `ignore_duration` bypasses conditions' minimum
	/// duration (notably crit boost). Returns whether it is absent afterwards.
	#[doc(alias("RemoveCond", "RemoveCondEx"))]
	pub fn remove(
		self,
		condition: Condition,
		ignore_duration: bool,
	) -> Result<bool, ConditionError> {
		// SAFETY: As for `add`. This checked native method runs TF2's ordinary
		// removal path with a valid condition identifier.
		unsafe { raw::remove_cond_ex(self.raw_player(), condition.0, ignore_duration) }?;
		Ok(!self.in_cond(condition)?)
	}

	/// Runs TF2's full `RemoveAllCond` cleanup, including its object list.
	#[doc(alias("RemoveAllCond"))]
	pub fn remove_all(self) -> Result<(), ConditionError> {
		// SAFETY: As for `add`. The native method performs normal condition
		// cleanup without immediately deleting entities.
		unsafe { raw::remove_all_cond(self.raw_player()) }?;
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn duration_cannot_pass_invalid_floats_to_the_engine() {
		for invalid in [-1.0, f32::NEG_INFINITY, f32::INFINITY, f32::NAN] {
			assert!(ConditionDuration::seconds(invalid).is_none());
		}
		assert_eq!(ConditionDuration::PERMANENT.as_raw(), -1.0);
		assert_eq!(ConditionDuration::seconds(0.0).unwrap().as_raw(), 0.0);
		assert_eq!(ConditionDuration::seconds(3.5).unwrap().as_raw(), 3.5);
	}

	#[test]
	fn identifiers_exclude_sentinels_and_include_extended_conditions() {
		assert!(Condition::from_raw(-1).is_none());
		assert!(Condition::from_raw(sys::ETFCond_TF_COND_LAST).is_none());
		assert!(Condition::from_raw(i32::MAX).is_none());
		assert_eq!(Condition::from_raw(0), Some(Condition::AIMING));
		assert_eq!(Condition::from_raw(130).unwrap().to_raw(), 130);
	}
}
