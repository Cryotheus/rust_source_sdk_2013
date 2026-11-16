//! TF2 building detonation hooks, which run before the game's
//! `CBaseObject::DetonateObject` for the buildings of the classes they cover,
//! and may keep the building.
//!
//! `DetonateObject` destroys a building as if it died by its own hand: it
//! calls `Killed` with the building as attacker and inflictor, which explodes
//! it into gibs and removes it. The game detonates a building as its
//! Engineer's PDA destroys it, as a sentry gun or teleporter finds itself in
//! a `func_nobuild` brush, and as it removes a player's buildings with
//! explosions (`CTFPlayer::RemoveAllObjects(true)`): as they change team or
//! leave. A building it removes without explosions, such as as its builder
//! changes class, does not reach it.
//!
//! [`MetamodApi::hook_building_detonations`] runs a callback before each, which
//! may skip it: the building stays as it was, in its builder's list of
//! buildings too. `RemoveAllObjects` still fires its `object_removed` event for
//! each building before detonating it.
//!
//! The hooks cover classes as [`crate::hooks::tf2::class`] describes: cover the
//! buildings' classes, those of
//! [`ClassTargets::objects`](source_sdk_2013::tf2::class_targets::ClassTargets::objects),
//! to hook every building's. The dispenser overrides the method, and calls
//! `CBaseObject`'s past the vtable, so the callback runs once for each.
//!
//! Under SourceHook, `DetonateObject` takes a hook manager, which every handle
//! shares.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/detonation.rs"]
mod tests;

use crate::MetamodApi;
use crate::hook::{HookAction, HookTiming, VirtualFunction};
use crate::hooks::tf2::class::ClassHooks;
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::buildings::DETONATE_OBJECT_SLOT;
use source_sdk_2013::raw::tf2::virtuals::EntityFn;
use source_sdk_2013::tf2::class_targets::BaseObject;
use source_sdk_2013::{Server, ServerBinding};

/// A callback-scoped server and the building about to be detonated, deciding
/// whether it is. A panic is contained by the hook dispatcher, and lets the
/// detonation go on.
pub type DetonationFn = for<'s> fn(Server<'s>, Entity<'s>) -> DetonationAction;

/// `DetonateObject` in a TF2 building's primary vtable.
const DETONATE_OBJECT: VirtualFunction<EntityFn> = VirtualFunction::new(DETONATE_OBJECT_SLOT);

/// Whether a building is detonated.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum DetonationAction {
	/// Lets the game detonate the building, through the other plugins' hooks.
	#[default]
	Continue,

	/// Keeps the building as it is: neither the game's method nor the hooks of
	/// other plugins that would run after this one run.
	Keep,
}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CBaseObject::DetonateObject` of the
	/// buildings of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them, and lets it keep the building.
	///
	/// A callback is not asked once an earlier hook skipped the method.
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_building_detonations(
		self,
		binding: ServerBinding,
		callback: DetonationFn,
	) -> ClassHooks<BaseObject> {
		ClassHooks::new(
			binding,
			callback,
			DETONATE_OBJECT,
			&[HookTiming::Pre],
			|server, callback, building, call| {
				if call.superseded() == Some(true) {
					return HookAction::Ignore;
				}

				match callback(server, building) {
					DetonationAction::Continue => HookAction::Ignore,
					DetonationAction::Keep => HookAction::Supersede(()),
				}
			},
		)
	}
}
