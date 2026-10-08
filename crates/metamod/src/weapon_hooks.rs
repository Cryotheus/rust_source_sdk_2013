//! TF2 weapon hooks, which run around the methods of the weapons, and of the
//! players carrying them, of the classes they cover: as a weapon runs its
//! attacks or reloads, as a melee weapon swings, and before a player switches
//! to a weapon, or after they pick one up.
//!
//! - [`MetamodApi::hook_melee_smacks`] runs before
//!   `CTFWeaponBaseMelee::GetSmackTime`, as a melee weapon swings and times
//!   when the swing lands, and can keep it from landing.
//! - [`MetamodApi::hook_weapon_frames`] runs around
//!   `CBaseCombatWeapon::ItemPostFrame`, in which a player's active weapon
//!   runs its attacks and reloads, after the movement of each of the player's
//!   commands.
//! - [`MetamodApi::hook_weapon_reloads`] runs before `CBaseCombatWeapon::Reload`,
//!   as a weapon starts or continues its reload, and can refuse it.
//! - [`MetamodApi::hook_weapon_switches`] runs before
//!   `CTFPlayer::Weapon_Switch`, as a player switches to a weapon they carry,
//!   and can refuse the switch.
//! - [`MetamodApi::hook_weapon_can_switch_to`] runs before
//!   `CTFPlayer::Weapon_CanSwitchTo`, which `Weapon_Switch` and the game's
//!   choice of a weapon to switch to ask, and can refuse the weapon.
//! - [`MetamodApi::hook_weapon_equips`] runs after `CTFPlayer::Weapon_Equip`,
//!   once a player carries a weapon they picked up or were given.
//!
//! The hooks cover classes as [`crate::class_hooks`] describes. The players'
//! classes are those of
//! [`ClassTargets::players`](source_sdk_2013::tf2::class_targets::ClassTargets::players).
//! TF2's weapons have a class for each kind of weapon, too many to name ahead:
//! cover each weapon's class as the weapon appears, with
//! [`ClassHooks::cover_entity`], such as once a player equips it.
//!
//! Under SourceHook, each of the methods takes a hook manager, which every
//! handle hooking it shares.

#[cfg(test)]
#[path = "tests/weapon_hooks.rs"]
mod tests;

use crate::MetamodApi;
use crate::class_hooks::ClassHooks;
use crate::hook::{HookAction, HookCall, HookTiming, Signature, VirtualFunction};
use source_sdk_2013::entities::Entity;
use source_sdk_2013::raw::tf2::player::WEAPON_SWITCH_SLOT;

use source_sdk_2013::raw::tf2::virtuals::{
	EntityFn, GET_SMACK_TIME_SLOT, ITEM_POST_FRAME_SLOT, PredicateFn, RELOAD_SLOT, SmackTimeFn,
	WEAPON_CAN_SWITCH_TO_SLOT, WEAPON_EQUIP_SLOT, WeaponFn, WeaponPredicateFn, WeaponSwitchFn,
};

use source_sdk_2013::tf2::class_targets::{CombatWeapon, TfMeleeWeapon, TfPlayer};
use source_sdk_2013::{Server, ServerBinding, sys};
use std::ptr::NonNull;

/// A callback-scoped server, whether the game's method is about to run or
/// ran, and the weapon it runs for. A panic is contained by the hook
/// dispatcher.
pub type WeaponFrameFn = for<'s> fn(Server<'s>, HookTiming, Entity<'s>);

/// A callback-scoped server and the weapon about to reload, deciding whether
/// it may. A panic is contained by the hook dispatcher, and lets the game
/// decide.
pub type ReloadFn = for<'s> fn(Server<'s>, Entity<'s>) -> WeaponAction;

/// A callback-scoped server, the player, and the weapon they are about to
/// switch to, deciding whether they may. A panic is contained by the hook
/// dispatcher, and lets the game decide.
pub type SwitchFn = for<'s> fn(Server<'s>, Entity<'s>, Entity<'s>) -> WeaponAction;

/// A callback-scoped server, the player, and the weapon they now carry. A
/// panic is contained by the hook dispatcher.
pub type EquipFn = for<'s> fn(Server<'s>, Entity<'s>, Entity<'s>);

/// A callback-scoped server and the melee weapon swinging, deciding whether
/// its swing lands. A panic is contained by the hook dispatcher, and lets the
/// swing land.
pub type SmackFn = for<'s> fn(Server<'s>, Entity<'s>) -> SmackAction;

/// Before the game's method, then after it.
const AROUND: &[HookTiming] = &[HookTiming::Post, HookTiming::Pre];

/// `GetSmackTime` in a TF2 melee weapon's primary vtable.
const GET_SMACK_TIME: VirtualFunction<SmackTimeFn> = VirtualFunction::new(GET_SMACK_TIME_SLOT);

/// `ItemPostFrame` in a weapon's primary vtable.
const ITEM_POST_FRAME: VirtualFunction<EntityFn> = VirtualFunction::new(ITEM_POST_FRAME_SLOT);

/// `Reload` in a weapon's primary vtable.
const RELOAD: VirtualFunction<PredicateFn> = VirtualFunction::new(RELOAD_SLOT);

/// `Weapon_CanSwitchTo` in a TF2 player's primary vtable.
const WEAPON_CAN_SWITCH_TO: VirtualFunction<WeaponPredicateFn> =
	VirtualFunction::new(WEAPON_CAN_SWITCH_TO_SLOT);

/// `Weapon_Equip` in a TF2 player's primary vtable.
const WEAPON_EQUIP: VirtualFunction<WeaponFn> = VirtualFunction::new(WEAPON_EQUIP_SLOT);

/// `Weapon_Switch` in a TF2 player's primary vtable.
const WEAPON_SWITCH: VirtualFunction<WeaponSwitchFn> = VirtualFunction::new(WEAPON_SWITCH_SLOT);

/// The time of the smack of a swing that never lands: a melee weapon smacks
/// once the time passes if it is positive (`CTFWeaponBaseMelee::ItemPostFrame`),
/// and sets this one once its swing landed.
const NO_SMACK: f32 = -1.0;

/// Whether a melee weapon's swing lands.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SmackAction {
	/// Lets the swing land, as the game times it.
	#[default]
	Continue,

	/// Keeps the swing from landing, skipping the game's method, and the hooks
	/// of other plugins that would run after this one: the weapon schedules no
	/// smack.
	Miss,
}

/// Whether a weapon may do what the game is about to have it do.
#[must_use]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum WeaponAction {
	/// Lets the game decide.
	#[default]
	Continue,

	/// Refuses, skipping the game's method, and the hooks of other plugins
	/// that would run after this one: the method returns `false`.
	Refuse,
}

impl MetamodApi<'_> {
	/// Runs `callback` before each `CTFWeaponBaseMelee::GetSmackTime` of the
	/// melee weapons of the classes the returned hooks cover, which are none
	/// until [`ClassHooks::cover`] covers them: as a melee weapon swings, and
	/// times its smack, when the swing lands, at which the weapon traces in
	/// front of its owner, and hits and damages what it finds. The Hot Hand
	/// times its second slap the same way.
	///
	/// [`SmackAction::Miss`] keeps the swing from landing: it traces, hits
	/// and damages nothing, and plays no hit sound. Its animations, its
	/// sound, and the times of the weapon's next attacks stay as the game set
	/// them. The swinging player's client predicts its own smacks, so it may
	/// still show a hit on its own screen.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_melee_smacks(
		self,
		binding: ServerBinding,
		callback: SmackFn,
	) -> ClassHooks<TfMeleeWeapon> {
		ClassHooks::new(
			binding,
			callback,
			GET_SMACK_TIME,
			&[HookTiming::Pre],
			|server, callback, weapon, call| {
				if call.superseded() == Some(true) {
					return HookAction::Ignore;
				}

				match callback(server, weapon) {
					SmackAction::Continue => HookAction::Ignore,
					SmackAction::Miss => HookAction::Supersede(NO_SMACK),
				}
			},
		)
	}

	/// Runs `callback` before each `CTFPlayer::Weapon_CanSwitchTo`
	/// of the players of the classes the returned hooks cover, which are none
	/// until [`ClassHooks::cover`] covers them: as the game asks whether a
	/// player may switch to a weapon, before switching to it, and as it
	/// chooses a weapon to switch to, such as after their active weapon runs
	/// out of ammo.
	///
	/// [`WeaponAction::Refuse`] refuses the weapon, which the player keeps
	/// carrying. Refusing every weapon a player carries can leave them
	/// without an active weapon.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_weapon_can_switch_to(
		self,
		binding: ServerBinding,
		callback: SwitchFn,
	) -> ClassHooks<TfPlayer> {
		ClassHooks::new(
			binding,
			callback,
			WEAPON_CAN_SWITCH_TO,
			&[HookTiming::Pre],
			|server, callback, player, call| {
				let (weapon,) = call.args();
				decide_switch(server, callback, player, weapon, call)
			},
		)
	}

	/// Runs `callback` after each `CTFPlayer::Weapon_Equip` of the players
	/// of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them: once a player carries a weapon, as
	/// the game gives them their loadout, or they pick a weapon up.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_weapon_equips(
		self,
		binding: ServerBinding,
		callback: EquipFn,
	) -> ClassHooks<TfPlayer> {
		ClassHooks::new(
			binding,
			callback,
			WEAPON_EQUIP,
			&[HookTiming::Post],
			|server, callback, player, call| {
				let (weapon,) = call.args();

				// SAFETY: The game equips a live weapon.
				if let Some(weapon) = unsafe { weapon_entity(server, weapon) } {
					callback(server, player, weapon);
				}

				HookAction::Ignore
			},
		)
	}

	/// Runs `callback` before and after each `CBaseCombatWeapon::ItemPostFrame`
	/// of the weapons of the classes the returned hooks cover, which are none
	/// until [`ClassHooks::cover`] covers them: as a player's active weapon
	/// runs its attacks and reloads, after the movement of each of the
	/// player's commands, unless the player cannot attack yet.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_weapon_frames(
		self,
		binding: ServerBinding,
		callback: WeaponFrameFn,
	) -> ClassHooks<CombatWeapon> {
		ClassHooks::new(
			binding,
			callback,
			ITEM_POST_FRAME,
			AROUND,
			|server, callback, weapon, call| {
				callback(server, call.timing(), weapon);
				HookAction::Ignore
			},
		)
	}

	/// Runs `callback` before each `CBaseCombatWeapon::Reload` of the weapons
	/// of the classes the returned hooks cover, which are none until
	/// [`ClassHooks::cover`] covers them: as a weapon starts its reload, or
	/// continues it, one shell at a time for weapons that reload so.
	///
	/// [`WeaponAction::Refuse`] keeps the weapon from reloading, while it
	/// would otherwise. The game asks again as the player keeps reloading,
	/// or attacking with an empty clip.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_weapon_reloads(
		self,
		binding: ServerBinding,
		callback: ReloadFn,
	) -> ClassHooks<CombatWeapon> {
		ClassHooks::new(
			binding,
			callback,
			RELOAD,
			&[HookTiming::Pre],
			|server, callback, weapon, call| {
				refuse_unless_superseded(call, || callback(server, weapon))
			},
		)
	}

	/// Runs `callback` before each `CTFPlayer::Weapon_Switch` of the players
	/// of the classes the returned hooks cover, which are none
	/// until [`ClassHooks::cover`] covers them: as a player switches to a
	/// weapon they carry, by their choice or the game's, such as to their
	/// loadout's first weapon as they spawn.
	///
	/// [`WeaponAction::Refuse`] keeps the player's active weapon.
	///
	/// `binding` must describe the running server. The callback must not
	/// delete entities immediately, as [`Server::new`] requires.
	pub fn hook_weapon_switches(
		self,
		binding: ServerBinding,
		callback: SwitchFn,
	) -> ClassHooks<TfPlayer> {
		ClassHooks::new(
			binding,
			callback,
			WEAPON_SWITCH,
			&[HookTiming::Pre],
			|server, callback, player, call| {
				let (weapon, _view_model) = call.args();
				decide_switch(server, callback, player, weapon, call)
			},
		)
	}
}

/// Asks `callback` whether `player` may switch to `weapon`.
fn decide_switch<'s, S: Signature<Output = bool>>(
	server: Server<'s>,
	callback: SwitchFn,
	player: Entity<'s>,
	weapon: *mut sys::CBaseCombatWeapon,
	call: &HookCall<'_, S>,
) -> HookAction<bool> {
	// SAFETY: The game switches to a live weapon, or none.
	let Some(weapon) = (unsafe { weapon_entity(server, weapon) }) else {
		return HookAction::Ignore;
	};

	refuse_unless_superseded(call, || callback(server, player, weapon))
}

/// Has the method return `false` if `decide` refuses, unless an earlier hook
/// already skipped it.
fn refuse_unless_superseded<S: Signature<Output = bool>>(
	call: &HookCall<'_, S>,
	decide: impl FnOnce() -> WeaponAction,
) -> HookAction<bool> {
	if call.superseded() == Some(true) {
		return HookAction::Ignore;
	}

	match decide() {
		WeaponAction::Continue => HookAction::Ignore,
		WeaponAction::Refuse => HookAction::Supersede(false),
	}
}

/// The weapon at `weapon`, or `None` for null.
///
/// # Safety
///
/// A non-null `weapon` must be a live entity of the server, for the callback
/// it is given to.
unsafe fn weapon_entity<'s>(
	server: Server<'s>,
	weapon: *mut sys::CBaseCombatWeapon,
) -> Option<Entity<'s>> {
	let weapon = NonNull::new(weapon.cast::<sys::CBaseEntity>())?;

	// SAFETY: As the caller promises. A weapon's entity base is at its start,
	// as every entity's is.
	Some(unsafe { Entity::from_live(server, weapon) })
}
