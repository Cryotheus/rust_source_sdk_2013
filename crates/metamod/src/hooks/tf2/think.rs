//! TF2 think and simulation hooks, which run before and after the game's
//! `CTFPlayer::PreThink` or `PostThink` for the players of the classes they
//! cover, or its `CBaseEntity::Think` or `PhysicsSimulate` for the entities.
//!
//! The game runs each player's commands as their client sends them. Around
//! the movement of each, it calls `PreThink`, in which TF2 updates the
//! player's conditions, and `PostThink`, in which it runs their taunts'
//! attacks. `PhysicsSimulate` runs each entity's movement and thinks once a
//! tick, and a player's commands, calling `Think` as the entity's main think
//! comes due. The hooks cover classes as [`crate::hooks::tf2::class`] describes:
//! cover the players' classes, those of
//! [`ClassTargets::players`](source_sdk_2013::tf2::class_targets::ClassTargets::players),
//! to hook every player's thinks.
//!
//! Under SourceHook, `PreThink`, `PostThink`, `Think` and `PhysicsSimulate`
//! each take a hook manager, which every handle hooking it shares.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/think.rs"]
mod tests;

use crate::MetamodApi;
use crate::hooks::tf2::class::ClassHooks;
use crate::hook::{HookAction, HookCall, HookTiming, VirtualFunction};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::entities::THINK_SLOT;

use source_sdk_2013::raw::tf2::virtuals::{
	EntityFn, PHYSICS_SIMULATE_SLOT, POST_THINK_SLOT, PRE_THINK_SLOT,
};

use source_sdk_2013::tf2::class_targets::{BaseEntity, TfPlayer};
use source_sdk_2013::{Server, ServerBinding};

/// A callback-scoped server, whether the game's method is about to run or
/// ran, and the player or entity it runs for. A panic is contained by the
/// hook dispatcher.
pub type ThinkFn = for<'s> fn(Server<'s>, HookTiming, Entity<'s>);

/// Before the game's method, then after it.
const AROUND: &[HookTiming] = &[HookTiming::Post, HookTiming::Pre];

/// `PhysicsSimulate` in an entity's primary vtable.
const PHYSICS_SIMULATE: VirtualFunction<EntityFn> = VirtualFunction::new(PHYSICS_SIMULATE_SLOT);

/// `Think` in an entity's primary vtable.
const THINK: VirtualFunction<EntityFn> = VirtualFunction::new(THINK_SLOT);

/// A TF2 player's method that runs around the movement of each of their
/// commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlayerThink {
	/// `CTFPlayer::PreThink`, before the movement, in which TF2 updates the
	/// player's conditions and the time they idled.
	PreThink,

	/// `CTFPlayer::PostThink`, after the movement, in which TF2 updates the
	/// player's animations and runs their taunt's attack.
	PostThink,
}

impl PlayerThink {
	/// The method in a TF2 player's primary vtable.
	const fn function(self) -> VirtualFunction<EntityFn> {
		VirtualFunction::new(match self {
			Self::PreThink => PRE_THINK_SLOT,
			Self::PostThink => POST_THINK_SLOT,
		})
	}
}

impl MetamodApi<'_> {
	/// Runs `callback` before and after each `think` of the players of the
	/// classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_player_thinks(
		self,
		think: PlayerThink,
		binding: ServerBinding,
		callback: ThinkFn,
	) -> ClassHooks<TfPlayer> {
		ClassHooks::new(binding, callback, think.function(), AROUND, observe)
	}

	/// Runs `callback` before and after each `CBaseEntity::PhysicsSimulate` of
	/// the entities of the classes the returned hooks cover, which are none
	/// until [`ClassHooks::cover`] covers them.
	///
	/// An entity simulates its move parent before itself, so the method can
	/// run again in a tick, for an entity that already simulated in it, and
	/// then returns at once. The hooks still run.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_simulations(
		self,
		binding: ServerBinding,
		callback: ThinkFn,
	) -> ClassHooks<BaseEntity> {
		ClassHooks::new(binding, callback, PHYSICS_SIMULATE, AROUND, observe)
	}

	/// Runs `callback` before and after each `CBaseEntity::Think` of the
	/// entities of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them.
	///
	/// The engine calls the method as it simulates the entity, once the
	/// entity's main think comes due, and the method runs the think function
	/// the entity set, or what its class overrides it with. The think contexts
	/// of [`think`](source_sdk_2013::raw::entities::think) run without it, and
	/// entities with no think scheduled skip it.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_thinks(self, binding: ServerBinding, callback: ThinkFn) -> ClassHooks<BaseEntity> {
		ClassHooks::new(binding, callback, THINK, AROUND, observe)
	}
}

/// Tells `callback` of the call, which goes on.
fn observe<'s>(
	server: Server<'s>,
	callback: ThinkFn,
	entity: Entity<'s>,
	call: &HookCall<'_, EntityFn>,
) -> HookAction<()> {
	callback(server, call.timing(), entity);
	HookAction::Ignore
}
