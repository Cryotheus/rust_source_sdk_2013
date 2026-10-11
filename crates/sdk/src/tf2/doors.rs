//! Checked brush/prop-door identity and a reversible brush-door Editor opening policy.
//!
//! `func_door` and `func_door_rotating` share `CBaseDoor`. Endpoint inputs
//! run the game's movement, collision and area-portal behavior; raw origins
//! or toggle-state writes would omit those effects. Holding a door also
//! requires an input hook to refuse Close, Toggle, Lock, SetToggleState and
//! AddOutput for that selected entity. This module does not install hooks.
//!
//! The policy follows Valve SDK commit
//! `b8cfb12c0e083a2ef5b2f9f9b50f3902fa034474`, `doors.cpp`/`doors.h`.

use crate::tf2::objectives::{Objective, ObjectiveError};
use crate::{Server, entities::Entity, entities::fields::FieldError, inputs::InputValue};
use std::ffi::{CStr, c_int};

/// A TF2 brush door with checked `CBaseDoor` datamaps.
#[derive(Debug, Clone, Copy)]
pub struct Door<'s>(Objective<'s>);

impl<'s> Door<'s> {
	/// As [`Self::hold_open`], with ownership checks before mutation and after
	/// each synchronous native input. Map outputs can replace phase/config or
	/// level; keep the restoration record if this returns any error.
	pub fn hold_open_while(
		self,
		settings: DoorSettings,
		mut current: impl FnMut() -> bool,
	) -> Result<(), DoorError> {
		settings.validate_editor()?;
		if !current() {
			return Err(DoorError::Interrupted);
		}
		self.0.check_live()?;
		let engine = self
			.0
			.server()
			.valve_engine()
			.map_err(ObjectiveError::from)?;
		self.entity()
			.set_data_field(engine, c"m_flWait", -1.0_f32)?;
		self.entity()
			.set_data_field(engine, c"m_bForceClosed", false)?;
		// NO_AUTO_RETURN (32) permits direct Use/Touch closing; USE_CLOSES
		// (8192) also permits manual closing outside AcceptInput.
		self.entity()
			.set_spawn_flags(engine, settings.spawn_flags & !(32 | 8192))?;
		checked_inputs(&mut current, &[c"Unlock", c"Open"], |name| {
			self.checked_input(name)
		})
	}

	/// As [`Self::restore`], with ownership checks before mutation and after
	/// each synchronous input. Retain the snapshot on interruption or failure
	/// so a later bounded attempt can complete restoration.
	pub fn restore_while(
		self,
		settings: DoorSettings,
		mut current: impl FnMut() -> bool,
	) -> Result<(), DoorError> {
		settings.validate_editor()?;
		if !current() {
			return Err(DoorError::Interrupted);
		}
		self.0.check_live()?;
		let engine = self
			.0
			.server()
			.valve_engine()
			.map_err(ObjectiveError::from)?;
		self.entity()
			.set_data_field(engine, c"m_flWait", settings.wait)?;
		self.entity()
			.set_data_field(engine, c"m_bForceClosed", settings.force_closed)?;
		self.entity()
			.set_spawn_flags(engine, settings.spawn_flags)?;
		checked_inputs(
			&mut current,
			&[
				c"Unlock",
				if settings.position == DoorPosition::Closed {
					c"Close"
				} else {
					c"Open"
				},
				if settings.locked { c"Lock" } else { c"Unlock" },
			],
			|name| self.checked_input(name),
		)
	}
}

impl<'s> Door<'s> {
	/// Checks the map class and native datamap lineage. Other barriers,
	/// including prop doors and moving brushes, are not accepted.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, DoorError> {
		if !matches!(
			entity.class_name().to_bytes(),
			b"func_door" | b"func_door_rotating"
		) {
			return Err(ObjectiveError::WrongClass {
				expected: "func_door or func_door_rotating",
			}
			.into());
		}
		let door = Self(Objective::new(
			server,
			entity,
			c"CBaseDoor",
			"func_door or func_door_rotating",
		)?);
		door.0.check_live()?;
		Ok(door)
	}

	/// The underlying checked brush door.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}

	/// Starts opening with automatic and manual closing disabled. The caller
	/// must record settings and install the selected-entity input filter first.
	/// An error can follow partial mutation; restore the recorded state.
	pub fn hold_open(self, settings: DoorSettings) -> Result<(), DoorError> {
		self.hold_open_while(settings, || true)
	}

	/// Restores recorded flags, delay, lock and endpoint. Remove the handle
	/// from the input filter first. Endpoint movement is asynchronous and
	/// fires normal outputs; interrupted trajectories are not fabricated.
	pub fn restore(self, settings: DoorSettings) -> Result<(), DoorError> {
		self.restore_while(settings, || true)
	}

	/// Reads every member before any Editor mutation. Native fields are
	/// validated by the owning class's datamap, including their types.
	pub fn settings(self) -> Result<DoorSettings, DoorError> {
		let position = match self.0.int_field(c"CBaseToggle", c"m_toggle_state")? {
			0 => DoorPosition::Open,
			1 => DoorPosition::Closed,
			2 => DoorPosition::Opening,
			3 => DoorPosition::Closing,

			value => {
				return Err(ObjectiveError::UnknownValue {
					name: c"m_toggle_state",
					value,
				}
				.into());
			}
		};
		Ok(DoorSettings {
			position,
			locked: self.0.bool_field(c"CBaseDoor", c"m_bLocked")?,
			force_closed: self.0.bool_field(c"CBaseDoor", c"m_bForceClosed")?,
			wait: self.0.float_field(c"CBaseToggle", c"m_flWait")?,
			spawn_flags: self.entity().spawn_flags()?,
			move_done_time: self.0.float_field(c"CBaseEntity", c"m_flMoveDoneTime")?,
			speed: self.0.float_field(c"CBaseEntity", c"m_flSpeed")?,
		})
	}
}

impl<'s> Door<'s> {
	fn checked_input(self, name: &CStr) -> Result<(), DoorError> {
		self.0.check_live()?;
		// An Unlock output can change speed before the following endpoint
		// input. Validate the current value, not only the captured snapshot.
		let speed = self.0.float_field(c"CBaseEntity", c"m_flSpeed")?;
		if !speed.is_finite() || speed <= 0.0 {
			return Err(DoorError::InvalidTiming);
		}
		self.0
			.input(name, InputValue::Void)
			.map_err(DoorError::from)
	}
}

/// Why a door could not be wrapped, read, held or restored.
#[derive(Debug, thiserror::Error)]
pub enum DoorError {
	/// A synchronous callback ended or replaced the caller's operation.
	#[error("the caller's door operation was interrupted")]
	Interrupted,
	/// A class, datamap, interface or input check failed.
	#[error(transparent)]
	Objective(#[from] ObjectiveError),
	/// A typed field could not be read or written.
	#[error(transparent)]
	Field(#[from] FieldError),
	/// The snapshot would lose an interrupted trajectory.
	#[error("the door is moving; an endpoint snapshot cannot restore its trajectory")]
	Moving,
	/// Legacy inversion does not have the supported endpoint semantics.
	#[error("the door uses the obsolete inverted start-open flag")]
	Inverted,
	/// Timing is nonfinite or movement speed is not positive.
	#[error("the door has unsupported timing or movement speed")]
	InvalidTiming,
	/// An open door has an automatic return that cannot be reconstructed.
	#[error("the open door has a pending automatic close")]
	PendingClose,
}

/// `TS_AT_TOP`, `TS_AT_BOTTOM`, and the two moving states in `CBaseToggle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoorPosition {
	/// At the open endpoint (`TS_AT_TOP`).
	Open,
	/// At the closed endpoint (`TS_AT_BOTTOM`).
	Closed,
	/// Moving toward the open endpoint (`TS_GOING_UP`).
	Opening,
	/// Moving toward the closed endpoint (`TS_GOING_DOWN`).
	Closing,
}

/// A brush door's state before an Editor hold. Store it with an entity
/// handle and map generation, not with a borrowed entity across callbacks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DoorSettings {
	/// The endpoint or motion represented by `m_toggle_state`.
	pub position: DoorPosition,
	/// Whether native Open/Toggle/Use requests are locked.
	pub locked: bool,
	/// Whether the door forces closing when obstructed.
	pub force_closed: bool,
	/// Automatic return delay in seconds; -1 keeps the door open.
	pub wait: f32,
	/// Original door flags, including manual return/use policy.
	pub spawn_flags: c_int,
	/// Native movement callback time, used to reject a pending return.
	pub move_done_time: f32,
	/// Movement speed in units or degrees per second.
	pub speed: f32,
}

impl DoorSettings {
	/// Refuses moving or inverted doors and an open door with a pending
	/// automatic close. Their interrupted trajectories/timers cannot be
	/// reconstructed from this endpoint-only snapshot.
	pub fn validate_editor(self) -> Result<(), DoorError> {
		if !matches!(self.position, DoorPosition::Open | DoorPosition::Closed) {
			return Err(DoorError::Moving);
		}
		if self.spawn_flags & 1 != 0 {
			return Err(DoorError::Inverted);
		}
		if !self.wait.is_finite()
			|| !self.move_done_time.is_finite()
			|| !self.speed.is_finite()
			|| self.speed <= 0.0
		{
			return Err(DoorError::InvalidTiming);
		}
		if self.position == DoorPosition::Open && self.move_done_time > 0.0 {
			return Err(DoorError::PendingClose);
		}
		Ok(())
	}
}

fn checked_inputs(
	current: &mut impl FnMut() -> bool,
	names: &[&CStr],
	mut send: impl FnMut(&CStr) -> Result<(), DoorError>,
) -> Result<(), DoorError> {
	for name in names {
		if !current() {
			return Err(DoorError::Interrupted);
		}
		send(name)?;
		if !current() {
			return Err(DoorError::Interrupted);
		}
	}
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn editor_refuses_states_it_cannot_restore() {
		let original = settings();
		assert!(original.validate_editor().is_ok());
		for position in [DoorPosition::Opening, DoorPosition::Closing] {
			assert!(matches!(
				DoorSettings {
					position,
					..original
				}
				.validate_editor(),
				Err(DoorError::Moving)
			));
		}
		assert!(matches!(
			DoorSettings {
				spawn_flags: 1,
				..original
			}
			.validate_editor(),
			Err(DoorError::Inverted)
		));
		assert!(matches!(
			DoorSettings {
				position: DoorPosition::Open,
				move_done_time: 5.0,
				..original
			}
			.validate_editor(),
			Err(DoorError::PendingClose)
		));
		for wait in [f32::NAN, f32::INFINITY] {
			assert!(matches!(
				DoorSettings { wait, ..original }.validate_editor(),
				Err(DoorError::InvalidTiming)
			));
		}
		for speed in [0.0, -1.0, f32::NAN] {
			assert!(matches!(
				DoorSettings { speed, ..original }.validate_editor(),
				Err(DoorError::InvalidTiming)
			));
		}
		assert!(
			DoorSettings {
				position: DoorPosition::Open,
				move_done_time: -1.0,
				wait: -1.0,
				..original
			}
			.validate_editor()
			.is_ok()
		);
	}

	fn settings() -> DoorSettings {
		DoorSettings {
			position: DoorPosition::Closed,
			locked: true,
			force_closed: true,
			wait: 3.0,
			spawn_flags: 32 | 8192,
			move_done_time: 0.0,
			speed: 100.0,
		}
	}

	#[test]
	fn synchronous_outputs_stop_remaining_native_inputs() {
		let current = std::cell::Cell::new(true);
		let mut sent = Vec::new();
		let result = checked_inputs(&mut || current.get(), &[c"Unlock", c"Open"], |name| {
			sent.push(name.to_owned());
			current.set(false);
			Ok(())
		});
		assert!(matches!(result, Err(DoorError::Interrupted)));
		assert_eq!(sent, [c"Unlock".to_owned()]);
		current.set(true);
		sent.clear();
		let result = checked_inputs(
			&mut || current.get(),
			&[c"Unlock", c"Close", c"Lock"],
			|name| {
				sent.push(name.to_owned());
				if name == c"Close" {
					current.set(false);
				}
				Ok(())
			},
		);
		assert!(matches!(result, Err(DoorError::Interrupted)));
		assert_eq!(sent, [c"Unlock".to_owned(), c"Close".to_owned()]);
		sent.clear();
		let result = checked_inputs(&mut || false, &[c"Open"], |name| {
			sent.push(name.to_owned());
			Ok(())
		});
		assert!(matches!(result, Err(DoorError::Interrupted)));
		assert!(sent.is_empty());
	}
}

/// A rotating prop door with checked `CBasePropDoor` datamap lineage.
/// Native blocking callbacks own its linked-door stop and resume behavior.
#[derive(Debug, Clone, Copy)]
pub struct PropDoor<'s>(Objective<'s>);

impl<'s> PropDoor<'s> {
	/// Checks the map class and native lineage without changing door state.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, DoorError> {
		if entity.class_name() != c"prop_door_rotating" {
			return Err(ObjectiveError::WrongClass {
				expected: "prop_door_rotating",
			}
			.into());
		}
		let door = Self(Objective::new(
			server,
			entity,
			c"CBasePropDoor",
			"prop_door_rotating",
		)?);
		door.0.check_live()?;
		Ok(door)
	}

	/// The underlying checked prop door.
	pub fn entity(self) -> Entity<'s> {
		self.0.entity()
	}
}
