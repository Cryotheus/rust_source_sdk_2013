//! TF2 entity removal hooks, which run before the game's
//! `CBaseEntity::UpdateOnRemove` for the entities of the classes they cover.
//!
//! The game calls `UpdateOnRemove` as it removes an entity, before its
//! destructor, in which the entity's class lets go of what it holds, such as
//! a building its team's list of buildings: the callback still sees the whole
//! entity. The hooks cover classes as [`crate::hooks::tf2::class`] describes.
//!
//! Under SourceHook, `UpdateOnRemove` takes a hook manager, which every handle
//! shares.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/removal.rs"]
mod tests;

use crate::MetamodApi;
use crate::hooks::tf2::class::ClassHooks;
use crate::hook::{HookAction, HookCall, HookTiming, VirtualFunction};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::virtuals::{EntityFn, UPDATE_ON_REMOVE_SLOT};
use source_sdk_2013::tf2::class_targets::BaseEntity;
use source_sdk_2013::{Server, ServerBinding};

/// A callback-scoped server and the entity the game removes. A panic is
/// contained by the hook dispatcher.
pub type RemovalFn = for<'s> fn(Server<'s>, Entity<'s>);

/// `UpdateOnRemove` in an entity's primary vtable.
const UPDATE_ON_REMOVE: VirtualFunction<EntityFn> = VirtualFunction::new(UPDATE_ON_REMOVE_SLOT);

impl MetamodApi<'_> {
	/// Runs `callback` before each `CBaseEntity::UpdateOnRemove` of the
	/// entities of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them: as the game removes one of them.
	///
	/// The game marked the entity for deletion, and deletes it at the end of
	/// the frame, or once the method returns, if it removes it immediately.
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_removals(
		self,
		binding: ServerBinding,
		callback: RemovalFn,
	) -> ClassHooks<BaseEntity> {
		ClassHooks::new(
			binding,
			callback,
			UPDATE_ON_REMOVE,
			&[HookTiming::Pre],
			observe,
		)
	}
}

/// Tells `callback` of the removal, which goes on.
fn observe<'s>(
	server: Server<'s>,
	callback: RemovalFn,
	entity: Entity<'s>,
	_call: &HookCall<'_, EntityFn>,
) -> HookAction<()> {
	callback(server, entity);
	HookAction::Ignore
}
