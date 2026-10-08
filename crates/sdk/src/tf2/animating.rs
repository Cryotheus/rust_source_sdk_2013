//! What TF2's animating entities, the game's `CBaseAnimating`s, expose to
//! VScript about their models, called through the members' native bindings.
//!
//! [`lookup_attachment`] finds an attachment by name. [`Animating`] wraps an
//! animating entity, such as a prop, a NextBot actor or a player, for the rest
//! (`game/server/baseanimating.cpp:269-310`): its sequences and how they play,
//! its activities, bones and pose parameters, its bodygroups and skin, its
//! model's scale, and changing its model.
//!
//! Sequences, bones, pose parameters and bodygroups are numbered by the
//! entity's model, so a number found on one model means nothing on another.
//! Activities are numbered by the game, alike for every model. The game
//! treats a sequence the model lacks as its first.
//!
//! # Unverified
//!
//! [`Animating`] follows the script descriptors of Valve's Source SDK 2013,
//! and has not been tested on a live server.

#[cfg(test)]
#[path = "../tests/tf2/animating.rs"]
mod tests;

use crate::entities::Entity;
use crate::interfaces::ModelInfo;
use crate::tf2::script_binding::{self as binding, BindingError, FLOAT, INT, VOID, float, string};
use crate::{Game, Server};
use sdk_raw::tf2::script_binding::{BOOL, int};
use std::ffi::{CStr, CString, c_int};

/// The data map and script class of every animating entity.
const ANIMATING_CLASS: &CStr = c"CBaseAnimating";

/// Why [`lookup_attachment`] could not look an attachment up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttachmentError {
	/// The entity is marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The server does not run TF2.
	#[error("attachment lookups require a TF2 server")]
	NotTf2,

	/// The game returned an index outside the range of attachments.
	#[error("the game returned the attachment index {0}, out of range")]
	OutOfRange(c_int),

	/// The binding adapter of the entity's `LookupAttachment` reported
	/// failure.
	#[error("the native LookupAttachment method rejected its arguments")]
	Rejected,

	/// The entity is not an animating entity, or its script class descriptors
	/// lack `LookupAttachment`, or its signature differs from the SDK's.
	#[error("the entity does not expose the expected native LookupAttachment method")]
	UnsupportedMethod,
}

impl From<BindingError> for AttachmentError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// The index of the attachment named `name` on the model of `entity`, an
/// animating entity, as [`ParticleAttachment::Point`] takes it: numbered from
/// 1 in the order the model declares its attachments. `None` if the model has
/// no attachment of the name, or the entity has no model.
///
/// This calls the `LookupAttachment` member that `CBaseAnimating` exposes to
/// VScript (`game/server/baseanimating.cpp`) on the entity, through its native
/// binding, without a script VM. The game compares names without regard to
/// case.
///
/// [`ParticleAttachment::Point`]: crate::interfaces::temp_entities::ParticleAttachment::Point
#[doc(alias("LookupAttachment"))]
pub fn lookup_attachment(
	server: Server<'_>,
	entity: Entity<'_>,
	name: &CStr,
) -> Result<Option<u8>, AttachmentError> {
	if server.game() != Game::TeamFortress2 {
		return Err(AttachmentError::NotTf2);
	}

	if entity.is_marked_for_deletion() {
		return Err(AttachmentError::MarkedForDeletion);
	}

	// SAFETY: The checked `CBaseAnimating` member reads the entity's model,
	// and creates and frees no entities. It reads the name only during the
	// call.
	let result = unsafe {
		binding::call(
			entity,
			ANIMATING_CLASS,
			c"LookupAttachment",
			&mut [binding::string(name)],
			binding::INT,
		)?
	};

	// SAFETY: `call` checked FIELD_INTEGER before returning.
	let index = unsafe { result.__bindgen_anon_1.m_int };

	match index {
		0 => Ok(None),
		index => u8::try_from(index)
			.map(Some)
			.map_err(|_| AttachmentError::OutOfRange(index)),
	}
}

/// Why an animating entity could not be wrapped or controlled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AnimatingError {
	/// The entity is marked for deletion.
	#[error("the entity is marked for deletion")]
	MarkedForDeletion,

	/// The model given is not precached, or is a dynamic model.
	#[error("the model is not precached")]
	ModelNotPrecached,

	/// The entity is not an animating entity: its data maps lack
	/// `CBaseAnimating`'s.
	#[error("the entity is not an animating entity")]
	NotAnimating,

	/// The server does not run TF2.
	#[error("animating entities require a TF2 server")]
	NotTf2,

	/// The binding adapter of the member reported failure.
	#[error("the native method rejected its arguments")]
	Rejected,

	/// The entity's script class descriptors lack the member, or its
	/// signature differs from the SDK's.
	#[error("the entity does not expose the expected native method")]
	UnsupportedMethod,
}

impl From<BindingError> for AnimatingError {
	fn from(error: BindingError) -> Self {
		match error {
			BindingError::Unavailable | BindingError::SignatureMismatch => Self::UnsupportedMethod,
			BindingError::Rejected => Self::Rejected,
		}
	}
}

/// The index a lookup returned, or `None` for the game's -1, which it returns
/// for a name the model lacks.
const fn found(index: c_int) -> Option<c_int> {
	if index < 0 { None } else { Some(index) }
}

/// An animating entity (`CBaseAnimating`) within the current engine callback,
/// for the members it exposes to VScript, which [`Animating::new`] checks it
/// has.
///
/// Lookups by name compare names without regard to case. On an entity without
/// a model, sequence, activity and pose parameter lookups find 0, and
/// bone and bodygroup lookups find nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Animating<'s> {
	entity: Entity<'s>,
}

impl<'s> Animating<'s> {
	/// Wraps `entity`, an animating entity. Fails with
	/// [`AnimatingError::NotTf2`] unless the server runs TF2, with
	/// [`AnimatingError::MarkedForDeletion`] for an entity marked for
	/// deletion, and with [`AnimatingError::NotAnimating`] for an entity whose
	/// data maps lack `CBaseAnimating`'s.
	pub fn new(server: Server<'s>, entity: Entity<'s>) -> Result<Self, AnimatingError> {
		if server.game() != Game::TeamFortress2 {
			return Err(AnimatingError::NotTf2);
		}

		if entity.is_marked_for_deletion() {
			return Err(AnimatingError::MarkedForDeletion);
		}

		if !entity.is_a(ANIMATING_CLASS) {
			return Err(AnimatingError::NotAnimating);
		}

		Ok(Self { entity })
	}

	/// The index of the bone an attachment follows, numbered as
	/// [`lookup_attachment`] numbers attachments, or 0 for an attachment the
	/// model lacks (`GetAttachmentBone`).
	#[doc(alias("GetAttachmentBone"))]
	pub fn attachment_bone(self, attachment: u8) -> Result<c_int, AnimatingError> {
		// SAFETY: The member reads the entity's model.
		unsafe { self.int(c"GetAttachmentBone", &mut [int(attachment.into())]) }
	}

	/// The part a bodygroup shows (`GetBodygroup`).
	#[doc(alias("GetBodygroup"))]
	pub fn bodygroup(self, group: c_int) -> Result<c_int, AnimatingError> {
		// SAFETY: The member reads the entity's model and body.
		unsafe { self.int(c"GetBodygroup", &mut [int(group)]) }
	}

	/// The name of a bodygroup, or an empty string for one the model lacks
	/// (`GetBodygroupName`).
	#[doc(alias("GetBodygroupName"))]
	pub fn bodygroup_name(self, group: c_int) -> Result<Option<CString>, AnimatingError> {
		// SAFETY: The member returns a name from the model's data, which stays
		// loaded, or a static string.
		unsafe { self.string(c"GetBodygroupName", &mut [int(group)]) }
	}

	/// The name of a part of a bodygroup, or an empty string for one the model
	/// lacks (`GetBodygroupPartName`).
	#[doc(alias("GetBodygroupPartName"))]
	pub fn bodygroup_part_name(
		self,
		group: c_int,
		part: c_int,
	) -> Result<Option<CString>, AnimatingError> {
		// SAFETY: As for `bodygroup_name`.
		unsafe { self.string(c"GetBodygroupPartName", &mut [int(group), int(part)]) }
	}

	/// How far the current sequence has played, from 0 to 1 (`GetCycle`).
	#[doc(alias("GetCycle"))]
	pub fn cycle(self) -> Result<f32, AnimatingError> {
		// SAFETY: The member reads a field.
		unsafe { self.float(c"GetCycle", &mut []) }
	}

	/// The wrapped entity.
	pub const fn entity(self) -> Entity<'s> {
		self.entity
	}

	/// The index of the bodygroup named `name`, or `None` if the model has
	/// none of the name (`FindBodygroupByName`).
	#[doc(alias("FindBodygroupByName"))]
	pub fn find_bodygroup_by_name(self, name: &CStr) -> Result<Option<c_int>, AnimatingError> {
		// SAFETY: The member reads the entity's model, and the name only
		// during the call.
		unsafe { self.int(c"FindBodygroupByName", &mut [string(name)]) }.map(found)
	}

	/// Whether the current sequence has played to its end, which a looping
	/// sequence never does (`IsSequenceFinished`).
	#[doc(alias("IsSequenceFinished"))]
	pub fn is_sequence_finished(self) -> Result<bool, AnimatingError> {
		// SAFETY: The member reads a field.
		let result = unsafe { self.call(c"IsSequenceFinished", &mut [], BOOL) }?;

		// SAFETY: `call` checked the boolean result type.
		Ok(unsafe { result.__bindgen_anon_1.m_bool })
	}

	/// The number of the activity named `name`, such as `ACT_MP_RUN_MELEE`,
	/// or `None` if no sequence of the model plays it (`LookupActivity`).
	#[doc(alias("LookupActivity"))]
	pub fn lookup_activity(self, name: &CStr) -> Result<Option<c_int>, AnimatingError> {
		// SAFETY: As for `find_bodygroup_by_name`.
		unsafe { self.int(c"LookupActivity", &mut [string(name)]) }.map(found)
	}

	/// The index of the bone named `name`, or `None` if the model has none of
	/// the name (`LookupBone`).
	#[doc(alias("LookupBone"))]
	pub fn lookup_bone(self, name: &CStr) -> Result<Option<c_int>, AnimatingError> {
		// SAFETY: As for `find_bodygroup_by_name`.
		unsafe { self.int(c"LookupBone", &mut [string(name)]) }.map(found)
	}

	/// The index of the pose parameter named `name`, such as `move_x`, or
	/// `None` if the model has none of the name (`LookupPoseParameter`).
	#[doc(alias("LookupPoseParameter"))]
	pub fn lookup_pose_parameter(self, name: &CStr) -> Result<Option<c_int>, AnimatingError> {
		// SAFETY: As for `find_bodygroup_by_name`.
		unsafe { self.int(c"LookupPoseParameter", &mut [string(name)]) }.map(found)
	}

	/// The sequence named `name`, or one that plays the activity of the name,
	/// or `None` if the model has neither (`LookupSequence`).
	#[doc(alias("LookupSequence"))]
	pub fn lookup_sequence(self, name: &CStr) -> Result<Option<c_int>, AnimatingError> {
		// SAFETY: As for `find_bodygroup_by_name`. An activity with several
		// sequences picks one at random, through the game's random stream.
		unsafe { self.int(c"LookupSequence", &mut [string(name)]) }.map(found)
	}

	/// The scale the model is drawn at, 1 by default (`GetModelScale`).
	#[doc(alias("GetModelScale"))]
	pub fn model_scale(self) -> Result<f32, AnimatingError> {
		// SAFETY: The member reads a field.
		unsafe { self.float(c"GetModelScale", &mut []) }
	}

	/// How fast the current sequence plays, 1 for its own speed
	/// (`GetPlaybackRate`).
	#[doc(alias("GetPlaybackRate"))]
	pub fn playback_rate(self) -> Result<f32, AnimatingError> {
		// SAFETY: The member reads a field.
		unsafe { self.float(c"GetPlaybackRate", &mut []) }
	}

	/// Switches to `sequence` and plays it at its own speed
	/// (`ResetSequence`). A looping sequence that is already playing plays on
	/// undisturbed. The game keeps the cycle when the sequence it leaves
	/// loops, so [`Self::set_cycle`] starts the new one from its start.
	#[doc(alias("ResetSequence"))]
	pub fn reset_sequence(self, sequence: c_int) -> Result<(), AnimatingError> {
		// SAFETY: The member sets fields and reads the entity's model.
		unsafe { self.void(c"ResetSequence", &mut [int(sequence)]) }
	}

	/// The sequence playing (`GetSequence`).
	#[doc(alias("GetSequence"))]
	pub fn sequence(self) -> Result<c_int, AnimatingError> {
		// SAFETY: The member reads a field.
		unsafe { self.int(c"GetSequence", &mut []) }
	}

	/// The name of the activity a sequence plays
	/// (`GetSequenceActivityName`). For a sequence the model lacks it is
	/// `Unknown`, which the game also logs to the console, `Not Found!` for -1
	/// and `No model!` on an entity without a model.
	#[doc(alias("GetSequenceActivityName"))]
	pub fn sequence_activity_name(
		self,
		sequence: c_int,
	) -> Result<Option<CString>, AnimatingError> {
		// SAFETY: As for `bodygroup_name`.
		unsafe { self.string(c"GetSequenceActivityName", &mut [int(sequence)]) }
	}

	/// How long a sequence plays at its own speed, in seconds
	/// (`GetSequenceDuration`).
	#[doc(alias("GetSequenceDuration"))]
	pub fn sequence_duration(self, sequence: c_int) -> Result<f32, AnimatingError> {
		// SAFETY: The member reads the entity's model.
		unsafe { self.float(c"GetSequenceDuration", &mut [int(sequence)]) }
	}

	/// The name of a sequence (`GetSequenceName`). For a sequence the model
	/// lacks it is `Unknown`, which the game also logs to the console,
	/// `Not Found!` for -1 and `No model!` on an entity without a model.
	#[doc(alias("GetSequenceName"))]
	pub fn sequence_name(self, sequence: c_int) -> Result<Option<CString>, AnimatingError> {
		// SAFETY: As for `bodygroup_name`.
		unsafe { self.string(c"GetSequenceName", &mut [int(sequence)]) }
	}

	/// Shows `part` of a bodygroup (`SetBodygroup`).
	#[doc(alias("SetBodygroup"))]
	pub fn set_bodygroup(self, group: c_int, part: c_int) -> Result<(), AnimatingError> {
		// SAFETY: The member reads the entity's model and sets its body.
		unsafe { self.void(c"SetBodygroup", &mut [int(group), int(part)]) }
	}

	/// Sets how far the current sequence has played, from 0 to 1
	/// (`SetCycle`).
	#[doc(alias("SetCycle"))]
	pub fn set_cycle(self, cycle: f32) -> Result<(), AnimatingError> {
		// SAFETY: The member sets a field.
		unsafe { self.void(c"SetCycle", &mut [float(cycle)]) }
	}

	/// Changes the entity's model, as the `SetModel` input does, keeping the
	/// sequence of the same name and the cycle if the new model has one
	/// (`SetModelSimple`).
	///
	/// Fails with [`AnimatingError::ModelNotPrecached`], before calling the
	/// member, for a model that is not precached: the member would precache
	/// it, and a model the level has not precached could overflow the
	/// engine's model table, which stops the server.
	#[doc(alias("SetModelSimple", "ScriptSetModel"))]
	pub fn set_model(self, models: ModelInfo<'_>, model: &CStr) -> Result<(), AnimatingError> {
		if models.model_index(model).is_none() {
			return Err(AnimatingError::ModelNotPrecached);
		}

		// SAFETY: The member looks the precached model up, sets it and the
		// entity's bounds, and reads the name only during the call.
		unsafe { self.void(c"SetModelSimple", &mut [string(model)]) }
	}

	/// Scales the model to `scale` over `duration` seconds, or at once for 0
	/// (`SetModelScale`).
	#[doc(alias("SetModelScale"))]
	pub fn set_model_scale(self, scale: f32, duration: f32) -> Result<(), AnimatingError> {
		// SAFETY: The member sets fields, and for a duration schedules a think
		// of the entity's own.
		unsafe { self.void(c"SetModelScale", &mut [float(scale), float(duration)]) }
	}

	/// Sets how fast the current sequence plays, 1 for its own speed
	/// (`SetPlaybackRate`).
	#[doc(alias("SetPlaybackRate"))]
	pub fn set_playback_rate(self, rate: f32) -> Result<(), AnimatingError> {
		// SAFETY: The member sets a field.
		unsafe { self.void(c"SetPlaybackRate", &mut [float(rate)]) }
	}

	/// Sets a pose parameter, and returns the value the model clamps it to
	/// (`SetPoseParameter`).
	#[doc(alias("SetPoseParameter"))]
	pub fn set_pose_parameter(self, parameter: c_int, value: f32) -> Result<f32, AnimatingError> {
		// SAFETY: The member reads the entity's model and sets a field.
		unsafe { self.float(c"SetPoseParameter", &mut [int(parameter), float(value)]) }
	}

	/// Sets the sequence playing, without restarting it or changing its speed
	/// (`SetSequence`). [`Self::reset_sequence`] plays one from its start.
	#[doc(alias("SetSequence"))]
	pub fn set_sequence(self, sequence: c_int) -> Result<(), AnimatingError> {
		// SAFETY: The member sets a field, through the entity's vtable.
		unsafe { self.void(c"SetSequence", &mut [int(sequence)]) }
	}

	/// Sets the skin the model is drawn with (`SetSkin`).
	#[doc(alias("SetSkin"))]
	pub fn set_skin(self, skin: c_int) -> Result<(), AnimatingError> {
		// SAFETY: The member sets a field.
		unsafe { self.void(c"SetSkin", &mut [int(skin)]) }
	}

	/// The skin the model is drawn with (`GetSkin`).
	#[doc(alias("GetSkin"))]
	pub fn skin(self) -> Result<c_int, AnimatingError> {
		// SAFETY: The member reads a field.
		unsafe { self.int(c"GetSkin", &mut []) }
	}

	/// Stops the current sequence where it is, as a playback rate of 0
	/// (`StopAnimation`).
	#[doc(alias("StopAnimation"))]
	pub fn stop_animation(self) -> Result<(), AnimatingError> {
		// SAFETY: The member sets a field.
		unsafe { self.void(c"StopAnimation", &mut []) }
	}

	/// Plays the current sequence on by the time since the entity last did
	/// (`StudioFrameAdvance`), as entities that animate themselves do each
	/// think.
	#[doc(alias("StudioFrameAdvance"))]
	pub fn studio_frame_advance(self) -> Result<(), AnimatingError> {
		// SAFETY: The member sets fields and reads the entity's model, through
		// the entity's vtable.
		unsafe { self.void(c"StudioFrameAdvance", &mut []) }
	}

	/// Plays the current sequence on by `interval` seconds
	/// (`StudioFrameAdvanceManual`).
	#[doc(alias("StudioFrameAdvanceManual"))]
	pub fn studio_frame_advance_manual(self, interval: f32) -> Result<(), AnimatingError> {
		// SAFETY: The member sets fields and reads the entity's model.
		unsafe { self.void(c"StudioFrameAdvanceManual", &mut [float(interval)]) }
	}

	/// Calls a member of `CBaseAnimating`'s script class, unless the entity
	/// is marked for deletion.
	///
	/// # Safety
	///
	/// As for [`binding::call`].
	unsafe fn call(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
		result_type: sys::ScriptDataType_t,
	) -> Result<sys::ScriptVariant_t, AnimatingError> {
		if self.entity.is_marked_for_deletion() {
			return Err(AnimatingError::MarkedForDeletion);
		}

		// SAFETY: As the caller promises.
		Ok(unsafe { binding::call(self.entity, ANIMATING_CLASS, name, arguments, result_type) }?)
	}

	/// Calls a member that returns a `float`.
	///
	/// # Safety
	///
	/// As for [`binding::call`].
	unsafe fn float(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
	) -> Result<f32, AnimatingError> {
		// SAFETY: As the caller promises.
		let result = unsafe { self.call(name, arguments, FLOAT) }?;

		// SAFETY: `call` checked the float result type.
		Ok(unsafe { result.__bindgen_anon_1.m_float })
	}

	/// Calls a member that returns an `int`.
	///
	/// # Safety
	///
	/// As for [`binding::call`].
	unsafe fn int(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
	) -> Result<c_int, AnimatingError> {
		// SAFETY: As the caller promises.
		let result = unsafe { self.call(name, arguments, INT) }?;

		// SAFETY: `call` checked the integer result type.
		Ok(unsafe { result.__bindgen_anon_1.m_int })
	}

	/// Calls a member that returns a C string, and copies it, unless the
	/// entity is marked for deletion.
	///
	/// # Safety
	///
	/// As for [`binding::call_string`].
	unsafe fn string(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
	) -> Result<Option<CString>, AnimatingError> {
		if self.entity.is_marked_for_deletion() {
			return Err(AnimatingError::MarkedForDeletion);
		}

		// SAFETY: As the caller promises.
		Ok(unsafe { binding::call_string(self.entity, ANIMATING_CLASS, name, arguments) }?)
	}

	/// Calls a member that returns nothing.
	///
	/// # Safety
	///
	/// As for [`binding::call`].
	unsafe fn void(
		self,
		name: &CStr,
		arguments: &mut [sys::ScriptVariant_t],
	) -> Result<(), AnimatingError> {
		// SAFETY: As the caller promises.
		unsafe { self.call(name, arguments, VOID) }.map(drop)
	}
}
