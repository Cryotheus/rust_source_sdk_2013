//! TF2 physics collision hooks, which run before the game's
//! `CBaseEntity::ForceVPhysicsCollide` for the entities of the classes they
//! cover, and may make pairs of physics objects collide that the collision
//! groups keep apart.
//!
//! VPhysics asks the game whether two physics objects collide as they come
//! close, and the game asks each of their entities whether to force the pair
//! to collide, before it checks anything else, such as the game rules'
//! `ShouldCollide` on the collision groups, which keeps debris from colliding
//! with debris. VPhysics keeps a pair's answer while the objects stay close,
//! unless one of their entities' collision rules change, as its collision
//! group does. The hooks cover classes as [`crate::class_hooks`] describes.
//!
//! Under SourceHook, `ForceVPhysicsCollide` takes a hook manager, which every
//! handle shares.

#[cfg(test)]
#[path = "tests/physics_collision_hooks.rs"]
mod tests;

use crate::MetamodApi;
use crate::class_hooks::ClassHooks;
use crate::hook::{HookAction, HookCall, HookTiming, VirtualFunction};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::virtuals::{
	FORCE_VPHYSICS_COLLIDE_SLOT, ForceVPhysicsCollideFn as ForceVPhysicsCollide,
};
use source_sdk_2013::tf2::class_targets::BaseEntity;
use source_sdk_2013::{Server, ServerBinding};
use std::ptr::NonNull;

/// A callback-scoped server, the entity asked, and the entity whose physics
/// object came close to its own. A panic is contained by the hook dispatcher,
/// and lets the game decide.
pub type ForceCollideFn = for<'s> fn(Server<'s>, Entity<'s>, Entity<'s>) -> ForceCollideAction;

/// `ForceVPhysicsCollide` in an entity's primary vtable.
const FORCE_VPHYSICS_COLLIDE: VirtualFunction<ForceVPhysicsCollide> =
	VirtualFunction::new(FORCE_VPHYSICS_COLLIDE_SLOT);

/// Whether a pair of physics objects must collide.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ForceCollideAction {
	/// Lets the game decide, which only forces the pair for a few brush
	/// entities, then goes on with the rest of its checks.
	#[default]
	Continue,

	/// Makes the pair collide, without asking the game's method or checking
	/// the collision groups.
	Collide,
}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CBaseEntity::ForceVPhysicsCollide` of the
	/// entities of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them: as VPhysics asks whether one of their
	/// physics objects collides with another entity's.
	///
	/// [`ForceCollideAction::Collide`] skips the game's method, and the hooks
	/// of other plugins that would run after this one. The game asks the other
	/// entity too, unless this one forces the pair, so a pair of covered
	/// entities may call the callback twice. The game only asks of pairs of
	/// different entities, as they come close, and VPhysics keeps the answer
	/// while they stay close, so the callback runs far less often than a
	/// trace's, but should still be cheap. `binding` must describe the running
	/// server. The callback must not delete entities immediately, as
	/// [`Server::new`] requires.
	pub fn hook_force_physics_collisions(
		self,
		binding: ServerBinding,
		callback: ForceCollideFn,
	) -> ClassHooks<BaseEntity> {
		ClassHooks::new(
			binding,
			callback,
			FORCE_VPHYSICS_COLLIDE,
			&[HookTiming::Pre],
			decide,
		)
	}
}

/// Asks `callback` whether `entity`'s physics object must collide with the
/// other entity's.
fn decide<'s>(
	server: Server<'s>,
	callback: ForceCollideFn,
	entity: Entity<'s>,
	call: &HookCall<'_, ForceVPhysicsCollide>,
) -> HookAction<bool> {
	// An earlier hook already forced the pair.
	if call.superseded() == Some(true) {
		return HookAction::Ignore;
	}

	let (other,) = call.args();

	let Some(other) = NonNull::new(other) else {
		return HookAction::Ignore;
	};

	// SAFETY: VPhysics passes the live entity owning the other physics object,
	// which stays in the entity list through the call, on the main thread
	// during the engine's invocation the dispatcher runs in.
	let other = unsafe { Entity::from_live(server, other) };

	match callback(server, entity, other) {
		ForceCollideAction::Continue => HookAction::Ignore,
		ForceCollideAction::Collide => HookAction::Supersede(true),
	}
}
