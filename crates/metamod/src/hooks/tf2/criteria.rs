//! TF2 response criteria hooks, which run after the game's
//! `CBaseEntity::ModifyOrAppendCriteria` for the entities of the classes they
//! cover, and may change the criteria the entity appended.
//!
//! Before the game picks a response to a concept an entity speaks, such as a
//! TF2 player's taunt, voice command or pain, it builds a criteria set: the
//! concept's, then those the entity appends through this method, after which
//! the response system picks the rule the set matches best. A change after
//! the method so decides which rule matches, such as which scene a taunt
//! plays. The hooks cover classes as [`crate::hooks::tf2::class`] describes:
//! cover the players' classes, those of
//! [`ClassTargets::players`](source_sdk_2013::tf2::class_targets::ClassTargets::players),
//! to hook every player's.
//!
//! Under SourceHook, `ModifyOrAppendCriteria` takes a hook manager, which
//! every handle shares.

use crate::MetamodApi;
use crate::hook::{HookAction, HookCall, HookTiming, VirtualFunction};
use crate::hooks::tf2::class::ClassHooks;
use source_sdk_2013::criteria::CriteriaSet;
use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::criteria::{
	MODIFY_OR_APPEND_CRITERIA_SLOT, ModifyOrAppendCriteriaFn as ModifyOrAppendCriteria,
};

use source_sdk_2013::tf2::class_targets::BaseEntity;
use source_sdk_2013::{Server, ServerBinding};
use std::ptr::NonNull;

/// A callback-scoped server, the entity, and the set it appended its criteria
/// to, to read or change. A panic is contained by the hook dispatcher.
pub type CriteriaFn = for<'s> fn(Server<'s>, Entity<'s>, CriteriaSet<'s>);

/// `ModifyOrAppendCriteria` in an entity's primary vtable.
const MODIFY_OR_APPEND: VirtualFunction<ModifyOrAppendCriteria> =
	VirtualFunction::new(MODIFY_OR_APPEND_CRITERIA_SLOT);

impl MetamodApi<'_> {
	/// Runs `callback` after each `CBaseEntity::ModifyOrAppendCriteria` of
	/// the entities of the classes the returned hooks cover, which are none
	/// until [`ClassHooks::cover`] covers them.
	///
	/// The game calls it once for each concept an entity speaks, which for a
	/// player is often, so the callback should be cheap. `binding` must
	/// describe the running server. The callback must not delete entities
	/// immediately, as [`Server::new`] requires.
	pub fn hook_criteria(
		self,
		binding: ServerBinding,
		callback: CriteriaFn,
	) -> ClassHooks<BaseEntity> {
		ClassHooks::new(
			binding,
			callback,
			MODIFY_OR_APPEND,
			&[HookTiming::Post],
			appended,
		)
	}
}

/// Gives `callback` the set the entity appended to.
fn appended<'s>(
	server: Server<'s>,
	callback: CriteriaFn,
	entity: Entity<'s>,
	call: &HookCall<'_, ModifyOrAppendCriteria>,
) -> HookAction<()> {
	let (set,) = call.args();

	let Some(set) = NonNull::new(set) else {
		return HookAction::Ignore;
	};

	// SAFETY: The game passes a live set, which it keeps through the call and
	// does not touch while its hooks run, on the main thread.
	let set = unsafe { CriteriaSet::from_raw(set) };

	callback(server, entity, set);
	HookAction::Ignore
}
