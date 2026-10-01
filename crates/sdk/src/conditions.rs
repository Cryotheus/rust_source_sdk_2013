//! TF2 player conditions, including the object-backed conditions in
//! `tf_condition.cpp` and the remaining `CTFPlayerShared` conditions.
//!
//! Calls the game's native methods through their typed binding descriptors.
//! No script VM is needed, and no condition bits are written directly: the
//! game runs its normal add/remove notifications, durations and cleanup.

use crate::entities::Entity;
use crate::script_binding::{self as binding, BindingError};
use crate::{Game, Server};

/// A valid `ETFCond` identifier from TF2's `tf_shareddefs.h`.
/// All identifiers, including less common conditions, are available through
/// [`crate::sys`] as `ETFCond_TF_COND_*` constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct Condition(i32);

impl Condition {
	pub const AIMING: Self = Self(sys::ETFCond_TF_COND_AIMING);
	pub const ZOOMED: Self = Self(sys::ETFCond_TF_COND_ZOOMED);
	pub const DISGUISED: Self = Self(sys::ETFCond_TF_COND_DISGUISED);
	pub const STEALTHED: Self = Self(sys::ETFCond_TF_COND_STEALTHED);
	pub const INVULNERABLE: Self = Self(sys::ETFCond_TF_COND_INVULNERABLE);
	pub const CRITBOOSTED: Self = Self(sys::ETFCond_TF_COND_CRITBOOSTED);
	pub const BURNING: Self = Self(sys::ETFCond_TF_COND_BURNING);
	pub const BLEEDING: Self = Self(sys::ETFCond_TF_COND_BLEEDING);
	pub const SPEED_BOOST: Self = Self(sys::ETFCond_TF_COND_SPEED_BOOST);
	pub const MARKED_FOR_DEATH: Self = Self(sys::ETFCond_TF_COND_MARKEDFORDEATH);

	/// Rejects negative values and the `TF_COND_LAST` sentinel.
	pub const fn from_raw(raw: sys::ETFCond) -> Option<Self> {
		if raw >= 0 && raw < sys::ETFCond_TF_COND_LAST {
			Some(Self(raw))
		} else {
			None
		}
	}

	pub const fn to_raw(self) -> sys::ETFCond {
		self.0
	}
}

/// Condition lifetime. The game may keep a longer existing duration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConditionDuration(f32);

impl ConditionDuration {
	pub const PERMANENT: Self = Self(-1.0);

	/// A finite, nonnegative number of seconds. Zero expires on a subsequent
	/// game update; it does not mean permanent.
	pub fn seconds(seconds: f32) -> Option<Self> {
		(seconds.is_finite() && seconds >= 0.0).then_some(Self(seconds))
	}

	pub const fn as_raw(self) -> f32 {
		self.0
	}
}

/// A condition operation is unavailable for this game, entity, or binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConditionError {
	#[error("conditions require a TF2 player")]
	NotTfPlayer,
	#[error("the game does not expose the expected native condition method")]
	UnsupportedMethod,
	#[error("the native condition method rejected its arguments")]
	Rejected,
}

impl From<BindingError> for ConditionError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Rejected => Self::Rejected,
			_ => Self::UnsupportedMethod,
		}
	}
}

/// One player's conditions within the current engine callback.
#[derive(Debug, Clone, Copy)]
pub struct PlayerConditions<'s> {
	player: Entity<'s>,
}

impl<'s> PlayerConditions<'s> {
	pub fn new(server: Server<'s>, player: Entity<'s>) -> Result<Self, ConditionError> {
		if server.game() != Game::TeamFortress2 || player.class_name() != c"player" {
			return Err(ConditionError::NotTfPlayer);
		}
		Ok(Self { player })
	}

	pub const fn player(self) -> Entity<'s> {
		self.player
	}

	/// Adds a condition without a provider, using TF2's normal duration rules.
	/// Returns whether it is active afterwards. TF2 can refuse additions, for
	/// example on dead players or outside the competitive match summary.
	#[doc(alias = "AddCond")]
	pub fn add(
		self,
		condition: Condition,
		duration: ConditionDuration,
	) -> Result<bool, ConditionError> {
		// SAFETY: The checked native CTFPlayer method only adds a validated
		// condition on this live player; null means no provider. Its condition
		// effects respect the callback's deferred entity-deletion contract.
		unsafe {
			binding::call(
				self.player,
				c"CTFPlayer",
				c"AddCondEx",
				&mut [
					binding::int(condition.0),
					binding::float(duration.0),
					binding::handle(std::ptr::null_mut()),
				],
				binding::VOID,
			)?;
		}
		self.in_cond(condition)
	}

	/// Removes a condition. `ignore_duration` bypasses conditions' minimum
	/// duration (notably crit boost). Returns whether it is absent afterwards.
	#[doc(alias = "RemoveCond")]
	pub fn remove(
		self,
		condition: Condition,
		ignore_duration: bool,
	) -> Result<bool, ConditionError> {
		// SAFETY: This checked native method runs TF2's ordinary removal path
		// with a valid condition identifier on a live player.
		unsafe {
			binding::call(
				self.player,
				c"CTFPlayer",
				c"RemoveCondEx",
				&mut [binding::int(condition.0), binding::boolean(ignore_duration)],
				binding::VOID,
			)?;
		}
		Ok(!self.in_cond(condition)?)
	}

	/// Checks both the object-backed condition list and all extended bitfields.
	#[doc(alias = "InCond")]
	pub fn in_cond(self, condition: Condition) -> Result<bool, ConditionError> {
		// SAFETY: This is the native read-only query on a validated player and
		// condition. The binding verifies FIELD_BOOLEAN before returning.
		let result = unsafe {
			binding::call(
				self.player,
				c"CTFPlayer",
				c"InCond",
				&mut [binding::int(condition.0)],
				binding::BOOL,
			)?
		};
		// SAFETY: The checked return type selects the bool union member.
		Ok(unsafe { result.__bindgen_anon_1.m_bool })
	}

	/// Runs TF2's full `RemoveAllCond` cleanup, including its object list.
	#[doc(alias = "RemoveAllCond")]
	pub fn remove_all(self) -> Result<(), ConditionError> {
		// SAFETY: The native method performs normal condition cleanup on a
		// live player without immediately deleting entities.
		unsafe {
			binding::call(
				self.player,
				c"CTFPlayer",
				c"RemoveAllCond",
				&mut [],
				binding::VOID,
			)?;
		}
		Ok(())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn identifiers_exclude_sentinels_and_include_extended_conditions() {
		assert!(Condition::from_raw(-1).is_none());
		assert!(Condition::from_raw(sys::ETFCond_TF_COND_LAST).is_none());
		assert!(Condition::from_raw(i32::MAX).is_none());
		assert_eq!(Condition::from_raw(0), Some(Condition::AIMING));
		assert_eq!(Condition::from_raw(130).unwrap().to_raw(), 130);
	}

	#[test]
	fn duration_cannot_pass_invalid_floats_to_the_engine() {
		for invalid in [-1.0, f32::NEG_INFINITY, f32::INFINITY, f32::NAN] {
			assert!(ConditionDuration::seconds(invalid).is_none());
		}
		assert_eq!(ConditionDuration::PERMANENT.as_raw(), -1.0);
		assert_eq!(ConditionDuration::seconds(0.0).unwrap().as_raw(), 0.0);
		assert_eq!(ConditionDuration::seconds(3.5).unwrap().as_raw(), 3.5);
	}
}
