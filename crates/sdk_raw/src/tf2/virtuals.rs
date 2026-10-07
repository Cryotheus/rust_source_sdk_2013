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

use crate::abi::CppDestructors;
use crate::vtable_slot;
use std::ffi::c_int;

/// The signature of an entity's virtual methods that take and return nothing,
/// `void ()`: those of [`PRE_THINK_SLOT`], [`POST_THINK_SLOT`],
/// [`PHYSICS_SIMULATE_SLOT`] and [`UPDATE_ON_REMOVE_SLOT`].
pub type EntityFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity);

/// The signature of `CBaseEntity::ShouldCollide`,
/// `bool (int collisionGroup, int contentsMask) const`.
#[doc(alias("ShouldCollide"))]
pub type ShouldCollideFn =
	unsafe extern "C" fn(this: *mut sys::CBaseEntity, group: c_int, contents: c_int) -> bool;

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
