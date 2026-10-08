//! TF2 crit hooks, which run after the game decides whether an attack of a
//! weapon of the classes they cover crits, and may change the decision.
//!
//! As a weapon attacks, TF2's `CTFWeaponBase::CalcIsAttackCritical` decides
//! whether the attack crits, once a frame, through one of two methods: with
//! random crits on, `CalcIsAttackCriticalHelper`, which rolls for a random
//! crit unless a crit boost or the weapon's own rules decide, and otherwise
//! `CalcIsAttackCriticalHelperNoCrits`, from crit boosts and the weapon's
//! rules alone. Neither runs for the players of a team that won the round,
//! whose attacks all crit. [`CritCheck`] names the methods: hook both to see
//! every decision.
//!
//! The hooks cover classes as [`crate::class_hooks`] describes. TF2's weapons
//! have a class for each kind of weapon, too many to name ahead: cover each
//! weapon's class as the weapon appears, with [`ClassHooks::cover_entity`],
//! such as once a player equips it.
//!
//! Under SourceHook, each method takes a hook manager, which every handle
//! hooking it shares.

#[cfg(test)]
#[path = "tests/crit_hooks.rs"]
mod tests;

use crate::MetamodApi;
use crate::class_hooks::ClassHooks;
use crate::hook::{HookAction, HookTiming, VirtualFunction};
use source_sdk_2013::entities::Entity;

use source_sdk_2013::raw::tf2::virtuals::{
	CALC_IS_ATTACK_CRITICAL_HELPER_NO_CRITS_SLOT, CALC_IS_ATTACK_CRITICAL_HELPER_SLOT, PredicateFn,
};

use source_sdk_2013::tf2::class_targets::TfWeapon;
use source_sdk_2013::{Server, ServerBinding};

/// A callback-scoped server, the attacking weapon, and whether the game
/// decided the attack crits, deciding whether it does. A panic is contained by
/// the hook dispatcher, and keeps the game's decision.
pub type CritFn = for<'s> fn(Server<'s>, Entity<'s>, bool) -> CritAction;

/// Whether an attack crits.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum CritAction {
	/// Keeps the decision as the game, or an earlier hook, made it.
	#[default]
	Continue,

	/// Has the attack crit.
	Crit,

	/// Keeps the attack from critting.
	NoCrit,
}

/// One of the methods with which TF2 decides whether a weapon's attack crits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CritCheck {
	/// `CTFWeaponBase::CalcIsAttackCriticalHelper`, while random crits are
	/// on: rolls for a random crit unless a crit boost or the weapon's own
	/// rules decide.
	#[doc(alias("CalcIsAttackCriticalHelper"))]
	RandomCrits,

	/// `CTFWeaponBase::CalcIsAttackCriticalHelperNoCrits`, while random crits
	/// are off, as `tf_weapon_criticals 0` or the weapon turns them off:
	/// decides from crit boosts and the weapon's own rules alone.
	#[doc(alias("CalcIsAttackCriticalHelperNoCrits"))]
	NoRandomCrits,
}

impl CritCheck {
	/// The method in a TF2 weapon's primary vtable.
	const fn function(self) -> VirtualFunction<PredicateFn> {
		VirtualFunction::new(match self {
			Self::RandomCrits => CALC_IS_ATTACK_CRITICAL_HELPER_SLOT,
			Self::NoRandomCrits => CALC_IS_ATTACK_CRITICAL_HELPER_NO_CRITS_SLOT,
		})
	}
}

impl MetamodApi<'_> {
	/// Runs `callback` after each `check` of the weapons of the classes the
	/// returned hooks cover, which are none until [`ClassHooks::cover`] covers
	/// them: once the game decided whether the weapon's attack crits.
	///
	/// [`CritAction::Crit`] and [`CritAction::NoCrit`] change the decision,
	/// which the game then uses for the attack's damage and effects. What the
	/// method did besides deciding stays done, such as its bookkeeping of the
	/// weapon's random crits. The attacker's client predicts
	/// its own attacks' crits, so it can show the attack's effects on its own
	/// screen as the game decided.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_crit_checks(
		self,
		check: CritCheck,
		binding: ServerBinding,
		callback: CritFn,
	) -> ClassHooks<TfWeapon> {
		ClassHooks::new(
			binding,
			callback,
			check.function(),
			&[HookTiming::Post],
			|server, callback, weapon, call| {
				let crit = call.return_value().unwrap_or_default();

				match callback(server, weapon, crit) {
					CritAction::Continue => HookAction::Ignore,
					CritAction::Crit => HookAction::Override(true),
					CritAction::NoCrit => HookAction::Override(false),
				}
			},
		)
	}
}
