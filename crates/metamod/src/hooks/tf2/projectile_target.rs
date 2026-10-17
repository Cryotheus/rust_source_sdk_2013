//! TF2 projectile target hooks, which run after the game's
//! `CBaseEntity::IsCombatItem` or `CBaseEntity::IsProjectileCollisionTarget`
//! for the entities of the classes they cover, and may change which entities
//! TF2's projectiles strike.
//!
//! TF2's projectiles strike players, buildings and NPCs. Most of them stop at
//! other solid entities as at a wall, without hurting them, such as at a
//! `prop_dynamic`, unless one of two methods takes the entity for a target:
//!
//! - `IsCombatItem`, which Mann vs. Machine's shields answer. An enemy's arrow
//!   or Crusader's Crossbow bolt strikes a combat item as it strikes a
//!   player, at the hitbox it hits, so that a Huntsman's arrow can headshot
//!   it, then breaks. Rockets explode on one even if it is a trigger, and
//!   grenades and the traces of bullets and explosions treat it as solid to
//!   its enemies and passable to its own team. The game
//!   asks it of the entities many traces meet, and an entity on team 0, as
//!   props are, is of the team of every other entity on it, so count an
//!   entity as one only while a projectile strikes it.
//! - `IsProjectileCollisionTarget`, which the Righteous Bison's, the Pomson's
//!   and the Dragon's Fury's projectiles ask of the entities they touch,
//!   besides players, buildings, characters and combat items, and hurt.
//!   Nothing else asks it.
//!
//! The hooks cover classes as [`crate::hooks::tf2::class`] describes, such as
//! `prop_dynamic`'s, `CDynamicProp`.
//!
//! Under SourceHook, each method takes a hook manager, which every handle
//! hooking it shares.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/projectile_target.rs"]
mod tests;

use crate::MetamodApi;
use crate::hook::{HookAction, HookTiming, VirtualFunction};
use crate::hooks::tf2::class::ClassHooks;
use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::virtuals::{
	IS_COMBAT_ITEM_SLOT, IS_PROJECTILE_COLLISION_TARGET_SLOT, PredicateFn,
};

use source_sdk_2013::tf2::class_targets::BaseEntity;
use source_sdk_2013::{Server, ServerBinding};

/// A callback-scoped server, the entity, and whether the game takes it for a
/// target of the hooked method's kind, deciding whether it is one. A panic is
/// contained by the hook dispatcher, and keeps the game's answer.
pub type TargetFn = for<'s> fn(Server<'s>, Entity<'s>, bool) -> TargetAction;

/// `IsCombatItem` in an entity's primary vtable.
const IS_COMBAT_ITEM: VirtualFunction<PredicateFn> = VirtualFunction::new(IS_COMBAT_ITEM_SLOT);

/// `IsProjectileCollisionTarget` in an entity's primary vtable.
const IS_PROJECTILE_COLLISION_TARGET: VirtualFunction<PredicateFn> =
	VirtualFunction::new(IS_PROJECTILE_COLLISION_TARGET_SLOT);

/// Whether the game takes an entity for a target of a hooked method's kind.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum TargetAction {
	/// Keeps the answer as the game, or an earlier hook, gave it.
	#[default]
	Continue,

	/// Counts the entity as a target.
	Count,

	/// Counts the entity as none.
	Exclude,
}

impl MetamodApi<'_> {
	/// Runs `callback` after each `CBaseEntity::IsCombatItem` of the entities
	/// of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them: once the game answered whether the
	/// entity is a combat item, which arrows strike and rockets explode on.
	///
	/// [`TargetAction::Count`] and [`TargetAction::Exclude`] change the
	/// answer. The game asks it of the entities in the way of many traces, so
	/// the callback should be cheap. `binding` must describe the running
	/// server. The callback must not delete entities immediately, as
	/// [`Server::new`] requires.
	pub fn hook_combat_items(
		self,
		binding: ServerBinding,
		callback: TargetFn,
	) -> ClassHooks<BaseEntity> {
		hook_targets(binding, callback, IS_COMBAT_ITEM)
	}

	/// Runs `callback` after each `CBaseEntity::IsProjectileCollisionTarget`
	/// of the entities of the classes the returned hooks cover, which are none
	/// until [`ClassHooks::cover`] covers them: once the game answered whether
	/// the Righteous Bison's, the Pomson's and the Dragon's Fury's projectiles
	/// strike the entity.
	///
	/// [`TargetAction::Count`] and [`TargetAction::Exclude`] change the
	/// answer. `binding` must describe the running server. The callback must
	/// not delete entities immediately, as [`Server::new`] requires.
	pub fn hook_projectile_collision_targets(
		self,
		binding: ServerBinding,
		callback: TargetFn,
	) -> ClassHooks<BaseEntity> {
		hook_targets(binding, callback, IS_PROJECTILE_COLLISION_TARGET)
	}
}

/// Hooks `function`, one of the methods, after the game's, with `callback`.
fn hook_targets(
	binding: ServerBinding,
	callback: TargetFn,
	function: VirtualFunction<PredicateFn>,
) -> ClassHooks<BaseEntity> {
	ClassHooks::new(
		binding,
		callback,
		function,
		&[HookTiming::Post],
		|server, callback, entity, call| {
			let counted = call.return_value().unwrap_or_default();

			match callback(server, entity, counted) {
				TargetAction::Continue => HookAction::Ignore,
				TargetAction::Count => HookAction::Override(true),
				TargetAction::Exclude => HookAction::Override(false),
			}
		},
	)
}
