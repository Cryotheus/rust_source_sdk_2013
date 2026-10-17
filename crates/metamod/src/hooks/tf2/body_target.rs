//! TF2 body target hooks, which run before the game's
//! `CBaseEntity::BodyTarget` for the entities of the classes they cover.
//!
//! The game asks an entity's `BodyTarget` where to aim at it from a point.
//! TF2's explosions ask it of each entity in their radius, then at once trace
//! from where they burst to the point it returns, and spare the entity if the
//! trace stops at something else first. NPCs and bots aim with it too,
//! noisily. A player's is their eyes when the aim is not noisy, and most other
//! entities' their centre. The hooks cover classes as
//! [`crate::hooks::tf2::class`] describes.
//!
//! The method returns a `Vector` by value: under the MSVC ABI through a
//! hidden result pointer, and under the Itanium ABI in registers. The hooks
//! only tell of each call, and leave the point to the game.
//!
//! Under SourceHook, `BodyTarget` takes a hook manager, which every handle
//! shares.

#[cfg(test)]
#[path = "../../tests/hooks/tf2/body_target.rs"]
mod tests;

use crate::MetamodApi;
use crate::hook::{HookAction, HookCall, HookTiming, Signature, VirtualFunction};
use crate::hooks::tf2::class::ClassHooks;
use source_sdk_2013::entities::Entity;
use source_sdk_2013::math::Vector;
use source_sdk_2013::raw::tf2::virtuals::{BODY_TARGET_SLOT, BodyTargetFn as BodyTarget};
use source_sdk_2013::tf2::class_targets::BaseEntity;
use source_sdk_2013::{Server, ServerBinding};

/// A callback-scoped server, the entity asked, the point the aim is from, and
/// whether the aim is noisy, as an NPC's or a bot's is, and explosions' is
/// not. A panic is contained by the hook dispatcher.
pub type BodyTargetFn = for<'s> fn(Server<'s>, Entity<'s>, Vector, bool);

/// `BodyTarget` in an entity's primary vtable.
const BODY_TARGET: VirtualFunction<BodyTarget> = VirtualFunction::new(BODY_TARGET_SLOT);

impl MetamodApi<'_> {
	/// Runs `callback` before each `CBaseEntity::BodyTarget` of the entities
	/// of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them: as the game asks where to aim at
	/// one of them.
	///
	/// The game's method still runs, and decides the point. `binding` must
	/// describe the running server. The callback must not delete entities
	/// immediately, as [`Server::new`] requires.
	pub fn hook_body_targets(
		self,
		binding: ServerBinding,
		callback: BodyTargetFn,
	) -> ClassHooks<BaseEntity> {
		ClassHooks::new(binding, callback, BODY_TARGET, &[HookTiming::Pre], observe)
	}
}

/// Tells `callback` of the call, which goes on.
fn observe<'s>(
	server: Server<'s>,
	callback: BodyTargetFn,
	entity: Entity<'s>,
	call: &HookCall<'_, BodyTarget>,
) -> HookAction<<BodyTarget as Signature>::Output> {
	#[cfg(target_os = "windows")]
	let (_, from, noisy) = call.args();

	#[cfg(not(target_os = "windows"))]
	let (from, noisy) = call.args();

	// SAFETY: The game passes the point by reference, so it lives through
	// the call.
	if let Some(from) = unsafe { from.as_ref() } {
		callback(server, entity, Vector::from(*from), noisy);
	}

	HookAction::Ignore
}
