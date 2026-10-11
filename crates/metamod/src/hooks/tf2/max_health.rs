//! TF2 maximum health hooks, which run after the game's
//! `CBaseEntity::GetMaxHealth` for the entities of the classes they cover, and
//! may change the maximum health it returns.
//!
//! TF2's players compute theirs from their class and attributes, and most
//! other entities return their `m_iMaxHealth`. The game asks as it caps
//! healing, as a player spawns or resupplies with full health, and for the
//! maximum the scoreboard and HUD show. TF2 caps overheal from
//! `CTFPlayer::GetMaxHealthForBuffing`, which does not ask, so a changed
//! maximum leaves the overheal cap as it was. The hooks cover classes as
//! [`crate::hooks::tf2::class`] describes: cover the players' classes, those of
//! [`ClassTargets::players`](source_sdk_2013::tf2::class_targets::ClassTargets::players),
//! to hook every player's.
//!
//! Under SourceHook, `GetMaxHealth` takes a hook manager, which every handle
//! shares.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/max_health.rs"]
mod tests;

use crate::MetamodApi;
use crate::hooks::tf2::class::ClassHooks;
use crate::hook::{HookAction, HookTiming, VirtualFunction};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::entities::health::TF2_GET_MAX_HEALTH_SLOT;
use source_sdk_2013::raw::tf2::virtuals::MaxHealthFn as GetMaxHealth;
use source_sdk_2013::tf2::class_targets::BaseEntity;
use source_sdk_2013::{Server, ServerBinding};
use std::ffi::c_int;

/// A callback-scoped server, the entity, and the maximum health the game, or
/// an earlier hook, gave it, returning the maximum health to give it instead,
/// if any. A panic is contained by the hook dispatcher, and keeps the
/// maximum.
pub type MaxHealthFn = for<'s> fn(Server<'s>, Entity<'s>, c_int) -> Option<c_int>;

/// `GetMaxHealth` in an entity's primary vtable.
const GET_MAX_HEALTH: VirtualFunction<GetMaxHealth> = VirtualFunction::new(TF2_GET_MAX_HEALTH_SLOT);

impl MetamodApi<'_> {
	/// Runs `callback` after each `CBaseEntity::GetMaxHealth` of the entities
	/// of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them, and returns the maximum health it
	/// gives instead, if any.
	///
	/// The game asks often, such as each time it heals a player, so the
	/// callback should be cheap. `binding` must describe the running server.
	/// The callback must not delete entities immediately, as [`Server::new`]
	/// requires.
	pub fn hook_max_health(
		self,
		binding: ServerBinding,
		callback: MaxHealthFn,
	) -> ClassHooks<BaseEntity> {
		ClassHooks::new(
			binding,
			callback,
			GET_MAX_HEALTH,
			&[HookTiming::Post],
			|server, callback, entity, call| {
				let maximum = call.return_value().unwrap_or_default();

				match callback(server, entity, maximum) {
					Some(maximum) => HookAction::Override(maximum),
					None => HookAction::Ignore,
				}
			},
		)
	}
}
