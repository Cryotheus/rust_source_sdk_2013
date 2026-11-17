//! TF2 building upgrade hooks, which run after the game's
//! `CBaseObject::CanBeUpgraded` for the buildings of the classes they cover,
//! and may change whether a wrench hit upgrades the building.
//!
//! As a wrench hits a building of its team, TF2 asks `CanBeUpgraded` before it
//! puts the hitter's metal into the building's next level. It refuses while
//! the building upgrades, for mini-buildings and disposable sentries, for a
//! player whose class cannot build the building's kind, and at level 3. A
//! refused hit still repairs and refills the building, and spends no metal on
//! upgrading it, so a hook that refuses caps the building's level where it
//! is. Allowing what the game refuses lets the hit upgrade a building its
//! rules do not, such as past level 3, which the callback must not do.
//!
//! The hooks cover classes as [`crate::hooks::tf2::class`] describes, such as
//! those of [`ClassTargets::objects`](source_sdk_2013::tf2::class_targets::ClassTargets::objects).
//! Under SourceHook, `CanBeUpgraded` takes a hook manager, which every handle
//! shares.

use crate::MetamodApi;
use crate::hook::{HookAction, HookTiming, VirtualFunction};
use crate::hooks::tf2::class::ClassHooks;
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::buildings::{CAN_BE_UPGRADED_SLOT, CanBeUpgradedFn};
use source_sdk_2013::tf2::class_targets::BaseObject;
use source_sdk_2013::{Server, ServerBinding};
use std::ptr::NonNull;

/// A callback-scoped server, the building, the player whose wrench hits it,
/// and whether the game, or an earlier hook, lets the hit upgrade it,
/// returning whether to let it instead, if that differs. A panic is contained
/// by the hook dispatcher, and keeps the game's answer.
pub type UpgradeCheckFn = for<'s> fn(Server<'s>, Entity<'s>, Entity<'s>, bool) -> Option<bool>;

/// `CanBeUpgraded(CTFPlayer *)` in a building's primary vtable.
const CAN_BE_UPGRADED: VirtualFunction<CanBeUpgradedFn> =
	VirtualFunction::new(CAN_BE_UPGRADED_SLOT);

impl MetamodApi<'_> {
	/// Runs `callback` after each `CBaseObject::CanBeUpgraded` of the
	/// buildings of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them, and answers what it returns instead,
	/// if anything; see the [module documentation](crate::hooks::tf2::upgrade).
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_upgrade_checks(
		self,
		binding: ServerBinding,
		callback: UpgradeCheckFn,
	) -> ClassHooks<BaseObject> {
		ClassHooks::new(
			binding,
			callback,
			CAN_BE_UPGRADED,
			&[HookTiming::Post],
			|server, callback, building, call| {
				let (player,) = call.args();
				let allowed = call.return_value().unwrap_or_default();

				let Some(player) = NonNull::new(player) else {
					return HookAction::Ignore;
				};

				// SAFETY: The game passes the live player whose wrench hits the
				// building, who stays in the entity list through the call.
				let player = unsafe { Entity::from_live(server, player) };

				match callback(server, building, player, allowed) {
					Some(answer) if answer != allowed => HookAction::Override(answer),
					_ => HookAction::Ignore,
				}
			},
		)
	}
}
