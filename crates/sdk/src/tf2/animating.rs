//! What TF2's animating entities, the game's `CBaseAnimating`s, expose to
//! VScript about their models, read through the members' native bindings.

#[cfg(test)]
#[path = "../tests/tf2/animating.rs"]
mod tests;

use crate::entities::Entity;
use crate::tf2::script_binding::{self as binding, BindingError};
use crate::{Game, Server};
use std::ffi::{CStr, c_int};

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
			c"CBaseAnimating",
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
