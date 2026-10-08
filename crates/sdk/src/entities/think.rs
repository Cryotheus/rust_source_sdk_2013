//! Think contexts: the functions besides its main think that an entity
//! schedules under a name (`SetContextThink`), such as the `DieContext` in
//! which TF2's dropped ammo packs remove themselves 30 seconds after they
//! spawn.
//!
//! Each context is scheduled for a tick, which
//! [`GlobalVars::time_to_ticks`](crate::interfaces::player_info_manager::GlobalVars::time_to_ticks)
//! computes from a game time as the game does. A context that is not
//! scheduled keeps its function and name, and the game schedules it again
//! when it sets its next think.
//!
//! The contexts are found at the offset `CBaseEntity`'s datamap gives for
//! `m_aThinkFunctions`, which is only trusted where TF2's layout has it, as
//! [`sdk_raw::entities::think`] describes.

#[cfg(test)]
#[path = "../tests/entities/think.rs"]
mod tests;

use crate::entities::Entity;
use sdk_raw::entities::think as raw;
use std::ffi::{CStr, c_int};
use std::sync::OnceLock;

/// The offset of `m_aThinkFunctions`, once found.
static THINK_FUNCTIONS_OFFSET: OnceLock<usize> = OnceLock::new();

/// An entity's think contexts could not be accessed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
pub enum ThinkError {
	/// `CBaseEntity`'s datamap does not declare `m_aThinkFunctions` as a
	/// custom field where the SDK's layout of `CBaseEntity` has it, so the
	/// game DLL does not match the SDK.
	#[error(
		"the game's CBaseEntity datamap does not declare m_aThinkFunctions where the SDK expects it"
	)]
	UnsupportedLayout,
}

impl<'s> Entity<'s> {
	/// Unschedules the entity's think context named `context`, as the game's
	/// `SetNextThink(context, TICK_NEVER_THINK)` does, and returns whether it
	/// was scheduled.
	///
	/// The context keeps its function, and runs again only if the game
	/// schedules it again. The game keeps entities with no scheduled think out
	/// of its list of thinking entities, which it updates the next time it
	/// schedules or runs any think of the entity, rather than here: until
	/// then, the entity stays in the list, and the unscheduled context does not
	/// run.
	///
	/// Names are compared in their first 32 bytes, as the game compares them.
	/// Returns `Ok(false)` if the entity has no such context, or it is not
	/// scheduled.
	#[doc(alias("SetNextThink", "TICK_NEVER_THINK"))]
	pub fn cancel_think_context(self, context: &CStr) -> Result<bool, ThinkError> {
		let offset = self.think_functions_offset()?;

		// SAFETY: The entity is live during `'s`, on the main thread, and its
		// `m_aThinkFunctions` lies at the offset found and checked in the
		// datamaps of the game DLL's `CBaseEntity`, which every entity shares.
		// The game names contexts with pooled strings, and changes them only on
		// the main thread.
		let Some(index) = (unsafe { raw::think_context_index(self.as_ptr(), offset, context) })
		else {
			return Ok(false);
		};

		// SAFETY: As above.
		let is_scheduled = unsafe { raw::think_context_tick(self.as_ptr(), offset, index) }
			.is_some_and(|tick| tick > 0);

		if is_scheduled {
			// SAFETY: As above. The game writes the tick on the main thread too.
			unsafe {
				raw::set_think_context_tick(self.as_ptr(), offset, index, raw::TICK_NEVER_THINK)
			};
		}

		Ok(is_scheduled)
	}

	/// The tick at which the entity's think context named `context` next
	/// runs, or `None` if the entity has no such context, or it is not
	/// scheduled, at a tick of 0 or less.
	///
	/// Names are compared in their first 32 bytes, as the game compares them.
	#[doc(alias("GetNextThinkTick"))]
	pub fn next_think_tick(self, context: &CStr) -> Result<Option<c_int>, ThinkError> {
		let offset = self.think_functions_offset()?;

		// SAFETY: As for `cancel_think_context`.
		let tick = unsafe {
			raw::think_context_index(self.as_ptr(), offset, context)
				.and_then(|index| raw::think_context_tick(self.as_ptr(), offset, index))
		};

		Ok(tick.filter(|&tick| tick > 0))
	}

	/// The offset of `m_aThinkFunctions`, found through the entity's datamaps.
	///
	/// Only a found offset is kept, though every entity's datamaps include the
	/// `CBaseEntity` map it is found in.
	fn think_functions_offset(self) -> Result<usize, ThinkError> {
		if let Some(&offset) = THINK_FUNCTIONS_OFFSET.get() {
			return Ok(offset);
		}

		let offset =
			raw::find_think_functions(self.data_maps()).ok_or(ThinkError::UnsupportedLayout)?;

		Ok(*THINK_FUNCTIONS_OFFSET.get_or_init(|| offset))
	}
}
