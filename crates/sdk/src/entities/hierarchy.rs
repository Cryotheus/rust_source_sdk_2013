//! Parenting entities to each other, so that one moves with another, as the
//! `SetParent`, `SetParentAttachment` and `ClearParent` inputs do.
//!
//! [`Entity::move_parent`] reads the entity an entity moves with.

#[cfg(test)]
#[path = "../tests/entities/hierarchy.rs"]
mod tests;

use crate::entities::fields::{BaseField, FieldError};
use crate::entities::{Entity, EntityHandle};
use crate::inputs::{InputError, InputValue};
use crate::interfaces::ServerTools;
use std::ffi::CStr;
use std::num::NonZeroU8;

/// The most move children [`ServerTools::move_children`] lists of one entity:
/// as many as the entity list holds, so a longer chain of peers repeats an
/// entity, and has a cycle, which the game does not make.
const MAX_MOVE_CHILDREN: usize = sdk_raw::entities::NUM_ENT_ENTRIES;

/// The longest chain of move parents [`ServerTools::set_parent`] climbs to
/// look for the child: one longer than the entity list repeats an entity, so
/// has a cycle already.
const MAX_PARENT_DEPTH: usize = sdk_raw::entities::NUM_ENT_ENTRIES;

/// `m_hMoveChild`.
static MOVE_CHILD: BaseField<EntityHandle> =
	BaseField::new(c"m_hMoveChild", sys::_fieldtypes_FIELD_EHANDLE);

/// `m_hMovePeer`.
static MOVE_PEER: BaseField<EntityHandle> =
	BaseField::new(c"m_hMovePeer", sys::_fieldtypes_FIELD_EHANDLE);

/// `m_iParentAttachment`.
static PARENT_ATTACHMENT: BaseField<u8> =
	BaseField::new(c"m_iParentAttachment", sys::_fieldtypes_FIELD_CHARACTER);

/// Why an entity could not be parented to another.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParentError {
	/// The parent is the child, or moves with it, so parenting them would
	/// make a cycle the game loops through forever.
	#[error("the parent is the child, or moves with it")]
	Cycle,

	/// A member the parenting reads is missing from the entities' datamaps.
	#[error(transparent)]
	Field(#[from] FieldError),

	/// The game refused the input that parents the entity.
	#[error(transparent)]
	Input(#[from] InputError),

	/// The entity was parented, but its parent has no model, or no attachment
	/// of that name, so it follows the parent's origin.
	#[error("the parent has no attachment of that name")]
	UnknownAttachment,
}

impl<'s> Entity<'s> {
	/// The first of the entities that move with the entity (`m_hMoveChild`),
	/// which is the one parented to it last, or `None` if none does. Each
	/// child's [`move_peer`](Self::move_peer) leads to the next, as
	/// [`ServerTools::move_children`] follows.
	#[doc(alias("FirstMoveChild", "m_hMoveChild"))]
	pub fn move_child(self) -> Result<Option<EntityHandle>, FieldError> {
		let child = MOVE_CHILD.read(self)?;

		Ok(child.is_valid().then_some(child))
	}

	/// The next of the entities that move with the entity's move parent
	/// (`m_hMovePeer`), which was parented to it before the entity, or `None` if
	/// the entity is the last, or has no parent.
	#[doc(alias("NextMovePeer", "m_hMovePeer"))]
	pub fn move_peer(self) -> Result<Option<EntityHandle>, FieldError> {
		let peer = MOVE_PEER.read(self)?;

		Ok(peer.is_valid().then_some(peer))
	}

	/// The attachment of its move parent's model the entity follows
	/// (`m_iParentAttachment`), or `None` if it follows the parent's origin,
	/// or has no parent.
	#[doc(alias("GetParentAttachment", "m_iParentAttachment"))]
	pub fn parent_attachment(self) -> Result<Option<NonZeroU8>, FieldError> {
		PARENT_ATTACHMENT.read(self).map(NonZeroU8::new)
	}
}

impl<'s> ServerTools<'s> {
	/// Has `child` stop moving with its move parent, which leaves it where it
	/// is in the world, through the `ClearParent` input.
	#[doc(alias("ClearParent", "AcceptEntityInput"))]
	pub fn clear_parent(self, child: Entity<'_>) -> Result<(), InputError> {
		self.accept_input(child, c"ClearParent", InputValue::Void, child, child)
	}

	/// The entities that move with `entity`, its move children, from the one
	/// parented to it last to the first, as the game walks them from its
	/// [`Entity::move_child`] through each child's [`Entity::move_peer`]
	/// (`FirstMoveChild` and `NextMovePeer`). A handle to an entity that no
	/// longer exists ends the list, as it ends the game's walk. As many are
	/// listed at most as the entity list holds.
	#[doc(alias("FirstMoveChild", "NextMovePeer"))]
	pub fn move_children(self, entity: Entity<'_>) -> Result<Vec<Entity<'s>>, FieldError> {
		let mut children = Vec::new();
		let mut next = entity.move_child()?;

		while let Some(handle) = next
			&& children.len() < MAX_MOVE_CHILDREN
		{
			let Some(child) = self.entity_by_handle(handle) else {
				break;
			};

			next = child.move_peer()?;
			children.push(child);
		}

		Ok(children)
	}

	/// Parents `child` to `parent`, so that it moves with it, keeping where
	/// it is in the world, through the `SetParent` input, which names
	/// `parent` as `!activator`. Any attachment the child followed is
	/// cleared.
	///
	/// Fails with [`ParentError::Cycle`] if `parent` is `child` or moves with
	/// it, which the game does not check.
	#[doc(alias("SetParent", "AcceptEntityInput"))]
	pub fn set_parent(self, child: Entity<'_>, parent: Entity<'_>) -> Result<(), ParentError> {
		let mut ancestor = Some(parent);

		for _ in 0..MAX_PARENT_DEPTH {
			let Some(entity) = ancestor else {
				return self
					.accept_input(
						child,
						c"SetParent",
						InputValue::String(c"!activator"),
						parent,
						child,
					)
					.map_err(ParentError::from);
			};

			if entity == child {
				return Err(ParentError::Cycle);
			}

			ancestor = entity
				.move_parent()?
				.and_then(|handle| self.entity_by_handle(handle));
		}

		Err(ParentError::Cycle)
	}

	/// Parents `child` to `parent` as [`Self::set_parent`] does, then has it
	/// follow `parent`'s model's attachment named `attachment`, as the
	/// `SetParentAttachment` input does, or `SetParentAttachmentMaintainOffset`
	/// with `keep_offset`.
	///
	/// Following an attachment stops the child moving on its own
	/// (`MOVETYPE_NONE`), and, without `keep_offset`, moves it onto the
	/// attachment, facing as it does.
	///
	/// Fails with [`ParentError::UnknownAttachment`] if `parent` has no model,
	/// or no attachment of that name, leaving `child` parented to its origin.
	#[doc(alias("SetParentAttachment", "SetParentAttachmentMaintainOffset"))]
	pub fn set_parent_attachment(
		self,
		child: Entity<'_>,
		parent: Entity<'_>,
		attachment: &CStr,
		keep_offset: bool,
	) -> Result<(), ParentError> {
		self.set_parent(child, parent)?;

		let input = if keep_offset {
			c"SetParentAttachmentMaintainOffset"
		} else {
			c"SetParentAttachment"
		};

		self.accept_input(child, input, InputValue::String(attachment), parent, child)?;

		// `SetParent` cleared the attachment, which the input only sets once
		// it finds one.
		match child.parent_attachment()? {
			Some(_) => Ok(()),
			None => Err(ParentError::UnknownAttachment),
		}
	}
}
