//! TF2 autobalance hooks, which run after the game's
//! `CTFPlayer::CanBeAutobalanced` for the players of the classes they cover,
//! as TF2's autobalance looks for players to move to the other team, and may
//! change whether a player can be.
//!
//! With `mp_autoteambalance` on, TF2 moves players from a team with more of
//! them to the other, choosing among those its method allows: it refuses
//! bots, coaches and their students, and players in a duel, a kart or ghost
//! mode. The hooks cover classes as [`crate::class_hooks`] describes: cover
//! the players' classes, those of
//! [`ClassTargets::players`](source_sdk_2013::tf2::class_targets::ClassTargets::players),
//! to hook every player's.
//!
//! Under SourceHook, `CanBeAutobalanced` takes a hook manager, which every
//! handle shares.

#[cfg(test)]
#[path = "tests/autobalance_hooks.rs"]
mod tests;

use crate::MetamodApi;
use crate::class_hooks::ClassHooks;
use crate::hook::{HookAction, HookTiming, VirtualFunction};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::virtuals::{CAN_BE_AUTOBALANCED_SLOT, PredicateFn};
use source_sdk_2013::tf2::class_targets::TfPlayer;
use source_sdk_2013::{Server, ServerBinding};

/// A callback-scoped server, the player, and whether the game allows
/// autobalancing them, deciding whether it may. A panic is contained by the
/// hook dispatcher, and keeps the game's decision.
pub type AutobalanceFn = for<'s> fn(Server<'s>, Entity<'s>, bool) -> AutobalanceAction;

/// `CanBeAutobalanced` in a TF2 player's primary vtable.
const CAN_BE_AUTOBALANCED: VirtualFunction<PredicateFn> =
	VirtualFunction::new(CAN_BE_AUTOBALANCED_SLOT);

/// Whether TF2's autobalance may move a player to the other team.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum AutobalanceAction {
	/// Keeps the decision as the game, or an earlier hook, made it.
	#[default]
	Continue,

	/// Allows moving the player.
	Allow,

	/// Keeps the player on their team.
	Refuse,
}

impl MetamodApi<'_> {
	/// Runs `callback` after each `CTFPlayer::CanBeAutobalanced` of the
	/// players of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them: once the game decided whether its
	/// autobalance may move the player to the other team.
	///
	/// [`AutobalanceAction::Allow`] and [`AutobalanceAction::Refuse`] change
	/// the decision. The autobalance still moves only players of the team
	/// with more of them, and only while it is on.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_autobalance_checks(
		self,
		binding: ServerBinding,
		callback: AutobalanceFn,
	) -> ClassHooks<TfPlayer> {
		ClassHooks::new(
			binding,
			callback,
			CAN_BE_AUTOBALANCED,
			&[HookTiming::Post],
			|server, callback, player, call| {
				let allowed = call.return_value().unwrap_or_default();

				match callback(server, player, allowed) {
					AutobalanceAction::Continue => HookAction::Ignore,
					AutobalanceAction::Allow => HookAction::Override(true),
					AutobalanceAction::Refuse => HookAction::Override(false),
				}
			},
		)
	}
}
