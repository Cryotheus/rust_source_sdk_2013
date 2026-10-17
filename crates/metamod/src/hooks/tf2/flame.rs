//! TF2 flame collision hooks, which run before the game's
//! `CTFPointManager::ShouldCollide` for the point managers of the classes
//! they cover, such as a flame thrower's flames, and may decide which
//! entities the points touch.
//!
//! A flame thrower's flames are the points of one `CTFFlameManager`, a
//! trigger the size of their bounds. As the engine tells it of a solid entity
//! it overlaps, the manager asks the method whether its points touch the
//! entity, and if so, burns it with the first point whose sweep meets it, at
//! most once every burn interval. The game's flames touch living players and
//! NPCs, enemy buildings, breakables, pumpkin bombs and a few other props.
//! The others stop the flames as a wall does, unhurt.
//!
//! The hooks cover classes as [`crate::hooks::tf2::class`] describes: cover
//! `CTFFlameManager`, which
//! [`ClassTargets::find_kind`](source_sdk_2013::tf2::class_targets::ClassTargets::find_kind)
//! finds as a [`TfPointManager`], to hook the flames'.
//!
//! Under SourceHook, `ShouldCollide` takes a hook manager, which every handle
//! shares.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/flame.rs"]
mod tests;

use crate::MetamodApi;
use crate::hook::{HookAction, HookCall, HookTiming, VirtualFunction};
use crate::hooks::tf2::class::ClassHooks;
use crate::hooks::tf2::collision::CollideAction;
use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::virtuals::{
	POINT_SHOULD_COLLIDE_SLOT, PointShouldCollideFn as PointShouldCollide,
};

use source_sdk_2013::tf2::class_targets::TfPointManager;
use source_sdk_2013::{Server, ServerBinding};
use std::ptr::NonNull;

/// A callback-scoped server, the point manager, such as a flame thrower's
/// flames, and the entity it overlaps. A panic is contained by the hook
/// dispatcher, and lets the game decide.
pub type FlameCollideFn = for<'s> fn(Server<'s>, Entity<'s>, Entity<'s>) -> CollideAction;

/// `ShouldCollide` in a point manager's primary vtable.
const POINT_SHOULD_COLLIDE: VirtualFunction<PointShouldCollide> =
	VirtualFunction::new(POINT_SHOULD_COLLIDE_SLOT);

impl MetamodApi<'_> {
	/// Runs `callback` before each `CTFPointManager::ShouldCollide` of the
	/// point managers of the classes the returned hooks cover, which are none
	/// until [`ClassHooks::cover`] covers them: as one asks whether its points
	/// touch an entity it overlaps.
	///
	/// [`CollideAction::Collide`] has the points touch the entity, so that the
	/// flames burn it as they burn a player, and [`CollideAction::Pass`] keeps
	/// them from it. Both skip the game's method, and the hooks of other
	/// plugins that would run after this one. A manager asks of each entity
	/// it overlaps at every tick, so the callback should be cheap. `binding`
	/// must describe the running server. The callback must not delete
	/// entities immediately, as [`Server::new`] requires.
	pub fn hook_flame_collisions(
		self,
		binding: ServerBinding,
		callback: FlameCollideFn,
	) -> ClassHooks<TfPointManager> {
		ClassHooks::new(
			binding,
			callback,
			POINT_SHOULD_COLLIDE,
			&[HookTiming::Pre],
			decide,
		)
	}
}

/// Asks `callback` whether the points of `manager` touch the entity it
/// overlaps.
fn decide<'s>(
	server: Server<'s>,
	callback: FlameCollideFn,
	manager: Entity<'s>,
	call: &HookCall<'_, PointShouldCollide>,
) -> HookAction<bool> {
	// An earlier hook already decided.
	if call.superseded() == Some(true) {
		return HookAction::Ignore;
	}

	let (other,) = call.args();

	let Some(other) = NonNull::new(other) else {
		return HookAction::Ignore;
	};

	// SAFETY: The game asks of a live entity, which stays in the entity list
	// through the call, on the main thread during the engine's invocation the
	// dispatcher runs in.
	let other = unsafe { Entity::from_live(server, other) };

	match callback(server, manager, other) {
		CollideAction::Continue => HookAction::Ignore,
		CollideAction::Collide => HookAction::Supersede(true),
		CollideAction::Pass => HookAction::Supersede(false),
	}
}
