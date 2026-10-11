//! TF2 collision hooks, which run before the game's
//! `CBaseEntity::ShouldCollide` for the entities of the classes they cover,
//! and may decide whether traces hit them.
//!
//! The game's trace filters ask each entity in a trace's way whether it
//! collides with the trace's collision group and contents mask, such as
//! player movement's, which TF2 passes through teammates with. The hooks cover
//! classes as [`crate::hooks::tf2::class`] describes.
//!
//! Under SourceHook, `ShouldCollide` takes a hook manager, which every handle
//! shares.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/collision.rs"]
mod tests;

use crate::MetamodApi;
use crate::hooks::tf2::class::ClassHooks;
use crate::hook::{HookAction, HookCall, HookTiming, VirtualFunction};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::virtuals::{SHOULD_COLLIDE_SLOT, ShouldCollideFn as ShouldCollide};
use source_sdk_2013::tf2::class_targets::BaseEntity;
use source_sdk_2013::{Server, ServerBinding};
use std::ffi::{c_int, c_uint};

/// A callback-scoped server, the entity in the trace's way, the trace's
/// collision group, and its contents mask, of
/// [`engine_trace`](source_sdk_2013::interfaces::engine_trace)'s `CONTENTS_*`
/// bits. [`TfCollisionGroup::from_raw`](source_sdk_2013::tf2::collision::TfCollisionGroup::from_raw)
/// converts the group, those TF2 shares with other Source games included. A
/// panic is contained by the hook dispatcher, and lets the game decide.
pub type ShouldCollideFn = for<'s> fn(Server<'s>, Entity<'s>, c_int, c_uint) -> CollideAction;

/// `ShouldCollide` in an entity's primary vtable.
const SHOULD_COLLIDE: VirtualFunction<ShouldCollide> = VirtualFunction::new(SHOULD_COLLIDE_SLOT);

/// Whether a trace collides with an entity in its way.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CollideAction {
	/// Lets the game decide.
	#[default]
	Continue,

	/// Tells the trace filter the entity collides with the trace, without
	/// asking the game's method. The filter's other checks, such as that of
	/// the game rules on the collision groups, can still let the trace pass.
	Collide,

	/// Has the trace pass through the entity, without asking the game.
	Pass,
}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CBaseEntity::ShouldCollide` of the
	/// entities of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them: as a trace filter asks one of them
	/// whether the trace collides with it.
	///
	/// [`CollideAction::Collide`] and [`CollideAction::Pass`] skip the game's
	/// method, and the hooks of other plugins that would run after this one.
	/// The game's filters skip the entities not solid to the trace's contents
	/// mask, and the entity the trace passes, before asking the entity, and
	/// ask the game rules of the collision groups after it. The method runs
	/// for each entity in the way of each trace, so the callback should be
	/// cheap. `binding` must describe
	/// the running server. The callback must not delete entities immediately,
	/// as [`Server::new`] requires.
	pub fn hook_should_collide(
		self,
		binding: ServerBinding,
		callback: ShouldCollideFn,
	) -> ClassHooks<BaseEntity> {
		ClassHooks::new(
			binding,
			callback,
			SHOULD_COLLIDE,
			&[HookTiming::Pre],
			decide,
		)
	}
}

/// Asks `callback` whether the trace collides with `entity`.
fn decide<'s>(
	server: Server<'s>,
	callback: ShouldCollideFn,
	entity: Entity<'s>,
	call: &HookCall<'_, ShouldCollide>,
) -> HookAction<bool> {
	// An earlier hook already decided.
	if call.superseded() == Some(true) {
		return HookAction::Ignore;
	}

	let (group, contents) = call.args();

	match callback(server, entity, group, contents.cast_unsigned()) {
		CollideAction::Continue => HookAction::Ignore,
		CollideAction::Collide => HookAction::Supersede(true),
		CollideAction::Pass => HookAction::Supersede(false),
	}
}
