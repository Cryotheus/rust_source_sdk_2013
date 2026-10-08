//! Hand-written ABI of the virtual methods of TF2's entities, players and
//! weapons that class-wide hooks hook: their vtable slots, from the generated
//! bindings and checked against SourceMod's gamedata, and their signatures,
//! with an entity as the receiver.
//!
//! A class deriving from one that declares a method keeps its slot when it
//! overrides it, so one slot holds the method in the tables of every class of
//! that kind. SourceMod's `gamedata/sdkhooks.games/engine.ep2v.txt` lists the
//! slots in its `tf` section. Linux's are mostly one more than Windows', as
//! Itanium vtables start with two destructor slots instead of MSVC's one, and
//! further apart where MSVC groups the overloads a class declares.
//!
//! The signatures take `*mut sys::CBaseEntity` as `this`, whatever class
//! declares the method, so that plugins hooking a method with the same
//! signature type share its hooks.

use super::player::WEAPON_SWITCH_SLOT;
use crate::abi::CppDestructors;
use crate::entities::health::TF2_GET_MAX_HEALTH_SLOT;
use crate::vtable_slot;
use std::ffi::{c_char, c_int};

/// The signature of an entity's virtual methods that take and return nothing,
/// `void ()`: those of [`PRE_THINK_SLOT`], [`POST_THINK_SLOT`],
/// [`PHYSICS_SIMULATE_SLOT`] and [`UPDATE_ON_REMOVE_SLOT`].
pub type EntityFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity);

/// The signature of `CBaseEntity::ForceVPhysicsCollide`,
/// `bool (CBaseEntity *entity)`, at [`FORCE_VPHYSICS_COLLIDE_SLOT`].
#[doc(alias("ForceVPhysicsCollide"))]
pub type ForceVPhysicsCollideFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, other: *mut sys::CBaseEntity) -> bool;

/// The signature of `CTFPlayer::GiveNamedItem`,
/// `CBaseEntity *(const char *name, int subType, const CEconItemView *item, bool force)`.
#[doc(alias("GiveNamedItem"))]
pub type GiveNamedItemFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	name: *const c_char,
	subtype: c_int,
	item: *const sys::CEconItemView,
	force: bool,
) -> *mut sys::CBaseEntity;

/// The signature of `CBaseEntity::GetMaxHealth`, `int () const`, at
/// [`TF2_GET_MAX_HEALTH_SLOT`].
#[doc(alias("GetMaxHealth"))]
pub type MaxHealthFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity) -> c_int;

/// The signature of an entity's virtual methods that take nothing and return
/// a `bool`, `bool ()`: those of [`RELOAD_SLOT`],
/// [`CALC_IS_ATTACK_CRITICAL_HELPER_SLOT`],
/// [`CALC_IS_ATTACK_CRITICAL_HELPER_NO_CRITS_SLOT`] and
/// [`CAN_BE_AUTOBALANCED_SLOT`].
pub type PredicateFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity) -> bool;

/// The signature of `CBasePlayer::PlayerRunCommand`,
/// `void (CUserCmd *command, IMoveHelper *helper)`.
#[doc(alias("PlayerRunCommand"))]
pub type RunCommandFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	command: *mut sys::CUserCmd,
	helper: *mut sys::IMoveHelper,
);

/// The signature of `CBaseEntity::ShouldCollide`,
/// `bool (int collisionGroup, int contentsMask) const`.
#[doc(alias("ShouldCollide"))]
pub type ShouldCollideFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, group: c_int, contents: c_int) -> bool;

/// The signature of `CTFWeaponBaseMelee::GetSmackTime`,
/// `float (int weaponMode)`, at [`GET_SMACK_TIME_SLOT`].
#[doc(alias("GetSmackTime"))]
pub type SmackTimeFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity, mode: c_int) -> f32;

/// The signature of a character's virtual methods that take a weapon and
/// return nothing, `void (CBaseCombatWeapon *weapon)`: that of
/// [`WEAPON_EQUIP_SLOT`].
pub type WeaponFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, weapon: *mut sys::CBaseCombatWeapon);

/// The signature of a character's virtual methods that take a weapon and
/// return a `bool`, `bool (CBaseCombatWeapon *weapon)`: that of
/// [`WEAPON_CAN_SWITCH_TO_SLOT`].
pub type WeaponPredicateFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, weapon: *mut sys::CBaseCombatWeapon) -> bool;

/// The signature of `CBaseCombatCharacter::Weapon_Switch`,
/// `bool (CBaseCombatWeapon *weapon, int viewModelIndex)`, at
/// [`WEAPON_SWITCH_SLOT`].
#[doc(alias("Weapon_Switch"))]
pub type WeaponSwitchFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	weapon: *mut sys::CBaseCombatWeapon,
	view_model: c_int,
) -> bool;

// SourceMod's slots on Windows: `ShouldCollide` at 17, `Blocked` at 108, which
// `EndBlocked` follows before `PhysicsSimulate`, and `PreThink` and
// `PostThink` at 344 and 345.
const _: () = {
	assert!(SHOULD_COLLIDE_SLOT == 16 + CppDestructors::VTABLE_SLOTS);
	assert!(PHYSICS_SIMULATE_SLOT == 109 + CppDestructors::VTABLE_SLOTS);
	assert!(PRE_THINK_SLOT == 343 + CppDestructors::VTABLE_SLOTS);
	assert!(POST_THINK_SLOT == PRE_THINK_SLOT + 1);
};

// The generated methods have these signatures, but for their receivers.
const _: () = {
	let _: fn(&sys::CBaseEntity__bindgen_vtable) -> EntityFn =
		|vtable| vtable.CBaseEntity_PhysicsSimulate;

	let _: fn(&sys::CBaseEntity__bindgen_vtable) -> EntityFn =
		|vtable| vtable.CBaseEntity_UpdateOnRemove;

	let _: fn(
		&sys::CBaseEntity__bindgen_vtable,
	) -> unsafe extern "C" fn(*const sys::CBaseEntity, c_int, c_int) -> bool =
		|vtable| vtable.CBaseEntity_ShouldCollide;

	let _: fn(&sys::CTFPlayer__bindgen_vtable) -> unsafe extern "C" fn(*mut sys::CTFPlayer) =
		|vtable| vtable.CTFPlayer_PreThink;

	let _: fn(&sys::CTFPlayer__bindgen_vtable) -> unsafe extern "C" fn(*mut sys::CTFPlayer) =
		|vtable| vtable.CTFPlayer_PostThink;
};

// Players, buildings and weapons override these in `CBaseEntity`'s slots.
const _: () = {
	use sys::{
		CBaseObject__bindgen_vtable as Object, CTFPlayer__bindgen_vtable as Player,
		CTFWeaponBase__bindgen_vtable as Weapon,
	};

	assert!(PHYSICS_SIMULATE_SLOT == vtable_slot!(Player, CTFPlayer_PhysicsSimulate));
	assert!(PHYSICS_SIMULATE_SLOT == vtable_slot!(Object, CBaseObject_PhysicsSimulate));
	assert!(PHYSICS_SIMULATE_SLOT == vtable_slot!(Weapon, CTFWeaponBase_PhysicsSimulate));
	assert!(SHOULD_COLLIDE_SLOT == vtable_slot!(Player, CTFPlayer_ShouldCollide));
	assert!(SHOULD_COLLIDE_SLOT == vtable_slot!(Object, CBaseObject_ShouldCollide));
	assert!(UPDATE_ON_REMOVE_SLOT == vtable_slot!(Player, CTFPlayer_UpdateOnRemove));
	assert!(UPDATE_ON_REMOVE_SLOT == vtable_slot!(Object, CBaseObject_UpdateOnRemove));
	assert!(UPDATE_ON_REMOVE_SLOT == vtable_slot!(Weapon, CTFWeaponBase_UpdateOnRemove));
};

// `game/server/baseentity.h` declares no virtual method between
// `PhysicsSimulate` and `UpdateOnRemove`.
const _: () = assert!(UPDATE_ON_REMOVE_SLOT == PHYSICS_SIMULATE_SLOT + 1);

// SourceMod's `gamedata/sdkhooks.games/engine.ep2v.txt` lists `VPhysicsUpdate`
// at 164 on Windows and 165 on Linux in its `tf` section, which
// `game/server/baseentity.h` declares after `ForceVPhysicsCollide` and
// `VPhysicsDestroyObject`.
const _: () = {
	let vphysics_update =
		vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_VPhysicsUpdate);

	assert!(vphysics_update == 163 + CppDestructors::VTABLE_SLOTS);
	assert!(FORCE_VPHYSICS_COLLIDE_SLOT + 2 == vphysics_update);

	let _: fn(&sys::CBaseEntity__bindgen_vtable) -> ForceVPhysicsCollideFn =
		|vtable| vtable.CBaseEntity_ForceVPhysicsCollide;
};

// SourceMod's slots on Windows: `PlayerRunCmd` at 431 in
// `gamedata/sdktools.games/game.tf.txt`, `Reload` at 284 in
// `gamedata/sdkhooks.games/engine.ep2v.txt`, which is 290 on Linux, and
// `CalcIsAttackCriticalHelper` and `CalcIsAttackCriticalHelperNoCrits` at 401
// and 402 in `gamedata/sm-tf2.games.txt`, which are 408 and 409 on Linux. The
// generated binding places `GiveNamedItem` at 487 on Windows and 494 on Linux.
const _: () = {
	assert!(PLAYER_RUN_COMMAND_SLOT == 430 + CppDestructors::VTABLE_SLOTS);

	assert!(
		RELOAD_SLOT
			== cfg_select! {
				target_os = "windows" => 284,
				target_os = "linux" => 290,
			}
	);

	assert!(
		CALC_IS_ATTACK_CRITICAL_HELPER_SLOT
			== cfg_select! {
				target_os = "windows" => 401,
				target_os = "linux" => 408,
			}
	);

	assert!(
		CALC_IS_ATTACK_CRITICAL_HELPER_NO_CRITS_SLOT == CALC_IS_ATTACK_CRITICAL_HELPER_SLOT + 1
	);

	assert!(
		GIVE_NAMED_ITEM_SLOT
			== cfg_select! {
				target_os = "windows" => 487,
				target_os = "linux" => 494,
			}
	);
};

// `Reload` is the 11th virtual method the server's `CBaseCombatWeapon`
// declares after `ItemPostFrame`. `game/server/basecombatcharacter.h` declares
// `Weapon_Equip`, `Weapon_EquipAmmoOnly` and `Weapon_Drop` before
// `Weapon_Switch`, and `Weapon_ShootPosition` and `Weapon_CanSwitchTo` after
// it.
const _: () = {
	assert!(RELOAD_SLOT == ITEM_POST_FRAME_SLOT + 11);
	assert!(WEAPON_EQUIP_SLOT + 3 == WEAPON_SWITCH_SLOT);
	assert!(WEAPON_CAN_SWITCH_TO_SLOT == WEAPON_SWITCH_SLOT + 2);
};

// The generated weapon methods have these signatures, but for their
// receivers, and TF2's weapons override them in the same slots.
const _: () = {
	use sys::{
		CBaseCombatWeapon__bindgen_vtable as CombatWeapon, CTFPlayer__bindgen_vtable as Player,
		CTFWeaponBase__bindgen_vtable as Weapon,
	};

	let _: fn(&CombatWeapon) -> unsafe extern "C" fn(*mut sys::CBaseCombatWeapon) =
		|vtable| vtable.CBaseCombatWeapon_ItemPostFrame;

	let _: fn(&CombatWeapon) -> unsafe extern "C" fn(*mut sys::CBaseCombatWeapon) -> bool =
		|vtable| vtable.CBaseCombatWeapon_Reload;

	let _: fn(&Weapon) -> unsafe extern "C" fn(*mut sys::CTFWeaponBase) -> bool =
		|vtable| vtable.CTFWeaponBase_CalcIsAttackCriticalHelper;

	let _: fn(&Weapon) -> unsafe extern "C" fn(*mut sys::CTFWeaponBase) -> bool =
		|vtable| vtable.CTFWeaponBase_CalcIsAttackCriticalHelperNoCrits;

	let _: fn(
		&Player,
	) -> unsafe extern "C" fn(
		*mut sys::CTFPlayer,
		*mut sys::CUserCmd,
		*mut sys::IMoveHelper,
	) = |vtable| vtable.CTFPlayer_PlayerRunCommand;

	let _: fn(&Player) -> unsafe extern "C" fn(*mut sys::CTFPlayer, *mut sys::CBaseCombatWeapon) =
		|vtable| vtable.CTFPlayer_Weapon_Equip;

	let _: fn(
		&Player,
	)
		-> unsafe extern "C" fn(*mut sys::CTFPlayer, *mut sys::CBaseCombatWeapon) -> bool =
		|vtable| vtable.CTFPlayer_Weapon_CanSwitchTo;

	let _: fn(
		&Player,
	) -> unsafe extern "C" fn(
		*mut sys::CTFPlayer,
		*const c_char,
		c_int,
		*const sys::CEconItemView,
		bool,
	) -> *mut sys::CBaseEntity = |vtable| vtable.CTFPlayer_GiveNamedItem1;

	assert!(ITEM_POST_FRAME_SLOT == vtable_slot!(Weapon, CTFWeaponBase_ItemPostFrame));
	assert!(RELOAD_SLOT == vtable_slot!(Weapon, CTFWeaponBase_Reload));
};

// `game/shared/tf/tf_weaponbase_melee.h` declares `GetSmackTime` right after
// `Smack`, and the generated binding places it at 474 on Windows and 481 on
// Linux.
const _: () = {
	use sys::CTFWeaponBaseMelee__bindgen_vtable as Melee;

	let _: fn(&Melee) -> unsafe extern "C" fn(*mut sys::CTFWeaponBaseMelee, c_int) -> f32 =
		|vtable| vtable.CTFWeaponBaseMelee_GetSmackTime;

	assert!(GET_SMACK_TIME_SLOT == vtable_slot!(Melee, CTFWeaponBaseMelee_Smack) + 1);

	assert!(
		GET_SMACK_TIME_SLOT
			== cfg_select! {
				target_os = "windows" => 474,
				target_os = "linux" => 481,
			}
	);
};

// SourceMod's `gamedata/sdkhooks.games/engine.ep2v.txt` lists `GetMaxHealth`
// at 123 on Windows and 124 on Linux in its `tf` section, which TF2's players
// override. The generated binding places `CanBeAutobalanced` at 474 on
// Windows and 475 on Linux.
const _: () = {
	assert!(TF2_GET_MAX_HEALTH_SLOT == 122 + CppDestructors::VTABLE_SLOTS);
	assert!(CAN_BE_AUTOBALANCED_SLOT == 473 + CppDestructors::VTABLE_SLOTS);

	assert!(
		TF2_GET_MAX_HEALTH_SLOT
			== vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_GetMaxHealth)
	);

	let _: fn(
		&sys::CBaseEntity__bindgen_vtable,
	) -> unsafe extern "C" fn(*const sys::CBaseEntity) -> c_int =
		|vtable| vtable.CBaseEntity_GetMaxHealth;

	let _: fn(
		&sys::CTFPlayer__bindgen_vtable,
	) -> unsafe extern "C" fn(*mut sys::CTFPlayer) -> bool =
		|vtable| vtable.CTFPlayer_CanBeAutobalanced;
};

/// The slot of `CTFPlayer::CanBeAutobalanced` in a TF2 player's primary
/// vtable, from the generated binding: the method that decides whether the
/// game's autobalance may move the player to the other team, which it refuses
/// for bots, coaches and their students, and players in a duel, a kart or
/// ghost mode.
#[doc(alias("CanBeAutobalanced"))]
pub const CAN_BE_AUTOBALANCED_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_CanBeAutobalanced);

/// The slot of `CTFWeaponBase::CalcIsAttackCriticalHelper` in a TF2 weapon's
/// primary vtable, from the generated binding: the method that decides
/// whether an attack of the weapon crits while random crits are on, rolling
/// for a random crit unless a crit boost or the weapon's own rules decide.
#[doc(alias("CalcIsAttackCriticalHelper"))]
pub const CALC_IS_ATTACK_CRITICAL_HELPER_SLOT: usize = vtable_slot!(
	sys::CTFWeaponBase__bindgen_vtable,
	CTFWeaponBase_CalcIsAttackCriticalHelper
);

/// The slot of `CTFWeaponBase::CalcIsAttackCriticalHelperNoCrits` in a TF2
/// weapon's primary vtable, from the generated binding: the method that
/// decides whether an attack of the weapon crits while random crits are off,
/// from crit boosts and the weapon's own rules.
#[doc(alias("CalcIsAttackCriticalHelperNoCrits"))]
pub const CALC_IS_ATTACK_CRITICAL_HELPER_NO_CRITS_SLOT: usize = vtable_slot!(
	sys::CTFWeaponBase__bindgen_vtable,
	CTFWeaponBase_CalcIsAttackCriticalHelperNoCrits
);

/// The slot of `CTFWeaponBaseMelee::GetSmackTime` in a TF2 melee weapon's
/// primary vtable, from the generated binding: the method from which a melee
/// weapon's swing learns when it lands, its smack, which the weapon then
/// traces and deals its damage at.
#[doc(alias("GetSmackTime"))]
pub const GET_SMACK_TIME_SLOT: usize = vtable_slot!(
	sys::CTFWeaponBaseMelee__bindgen_vtable,
	CTFWeaponBaseMelee_GetSmackTime
);

/// The slot of `CTFPlayer::GiveNamedItem` in a TF2 player's primary vtable,
/// from the generated binding: the method that creates an item of a
/// classname for the player, from an economy item or else the classname's
/// stock item, and has the player pick it up.
#[doc(alias("GiveNamedItem"))]
pub const GIVE_NAMED_ITEM_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_GiveNamedItem1);

/// The slot of `CBaseCombatWeapon::ItemPostFrame` in a weapon's primary
/// vtable, from the generated binding: the method in which a player's active
/// weapon runs its attacks and reloads, after the movement of each of the
/// player's commands, unless the player cannot attack yet.
#[doc(alias("ItemPostFrame"))]
pub const ITEM_POST_FRAME_SLOT: usize = vtable_slot!(
	sys::CBaseCombatWeapon__bindgen_vtable,
	CBaseCombatWeapon_ItemPostFrame
);

/// The slot of `CTFPlayer::PlayerRunCommand` in a TF2 player's primary vtable,
/// from the generated binding: the method that runs one of the commands the
/// player's client sent, its movement and its weapons' attacks.
#[doc(alias("PlayerRunCommand"))]
pub const PLAYER_RUN_COMMAND_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_PlayerRunCommand);

/// The slot of `CBaseCombatWeapon::Reload` in a weapon's primary vtable, from
/// the generated binding: the method that starts or continues the weapon's
/// reload, and returns whether it does.
#[doc(alias("Reload"))]
pub const RELOAD_SLOT: usize = vtable_slot!(
	sys::CBaseCombatWeapon__bindgen_vtable,
	CBaseCombatWeapon_Reload
);

/// The slot of `CTFPlayer::Weapon_CanSwitchTo` in a TF2 player's primary
/// vtable, from the generated binding: the method that decides whether the
/// player may switch to a weapon they carry.
#[doc(alias("Weapon_CanSwitchTo"))]
pub const WEAPON_CAN_SWITCH_TO_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_Weapon_CanSwitchTo);

/// The slot of `CTFPlayer::Weapon_Equip` in a TF2 player's primary vtable,
/// from the generated binding: the method that adds a weapon to those the
/// player carries.
#[doc(alias("Weapon_Equip"))]
pub const WEAPON_EQUIP_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_Weapon_Equip);

/// The slot of `CBaseEntity::ForceVPhysicsCollide` in an entity's primary
/// vtable, from the generated binding: the method VPhysics's collision filter
/// asks of each entity of a pair of physics objects, before the game rules'
/// collision groups, whether the pair must collide anyway.
#[doc(alias("ForceVPhysicsCollide"))]
pub const FORCE_VPHYSICS_COLLIDE_SLOT: usize = vtable_slot!(
	sys::CBaseEntity__bindgen_vtable,
	CBaseEntity_ForceVPhysicsCollide
);

/// The slot of `CBaseEntity::PhysicsSimulate` in an entity's primary vtable,
/// from the generated binding: the method that runs an entity's movement and
/// thinks once a tick, and a player's commands their client sent since. An
/// entity simulates its move parent first, whose method returns at once if it
/// already ran in the tick.
#[doc(alias("PhysicsSimulate"))]
pub const PHYSICS_SIMULATE_SLOT: usize = vtable_slot!(
	sys::CBaseEntity__bindgen_vtable,
	CBaseEntity_PhysicsSimulate
);

/// The slot of `CTFPlayer::PostThink` in a TF2 player's primary vtable, from
/// the generated binding: the method that runs after the movement of each of
/// the player's commands.
#[doc(alias("PostThink"))]
pub const POST_THINK_SLOT: usize =
	vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_PostThink);

/// The slot of `CTFPlayer::PreThink` in a TF2 player's primary vtable, from
/// the generated binding: the method that runs before the movement of each of
/// the player's commands.
#[doc(alias("PreThink"))]
pub const PRE_THINK_SLOT: usize = vtable_slot!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_PreThink);

/// The slot of `CBaseEntity::ShouldCollide` in an entity's primary vtable,
/// from the generated binding.
#[doc(alias("ShouldCollide"))]
pub const SHOULD_COLLIDE_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_ShouldCollide);

/// The slot of `CBaseEntity::UpdateOnRemove` in an entity's primary vtable,
/// from the generated binding: the method the game calls as it removes an
/// entity, before its destructor.
#[doc(alias("UpdateOnRemove"))]
pub const UPDATE_ON_REMOVE_SLOT: usize =
	vtable_slot!(sys::CBaseEntity__bindgen_vtable, CBaseEntity_UpdateOnRemove);
