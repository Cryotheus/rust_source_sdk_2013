//! Hand-written ABI of TF2's airblast pushing players and deflecting
//! projectiles: the vtable slots of `CTFWeaponBase::DeflectProjectiles`, which
//! runs a weapon's airblast, and of `CTFWeaponBase::DeflectPlayer` and
//! `CTFWeaponBase::DeflectEntity`, which the flame throwers override to push
//! each player it reaches and deflect each other entity, their signatures,
//! and the search for the vtables of the weapon classes that push.
//!
//! A flame thrower's airblast (`CTFFlameThrower::FireAirBlast`) runs
//! `DeflectProjectiles`, which every weapon class inherits: it goes through
//! the players and projectiles in a sphere in front of the weapon's owner, and
//! calls the weapon's `DeflectPlayer` through its vtable for each player but
//! the owner, and `DeflectEntity` for everything else. `CTFWeaponBase`'s own
//! `DeflectPlayer` does nothing. [`FLAME_THROWER_CLASS`] overrides it, and
//! [`DRAGONS_FURY_CLASS`] inherits the override, which extinguishes a burning
//! teammate, and pushes and stuns a player of the other team. It returns
//! whether it pushed the player, which the airblast only uses to pick its
//! sound and have its owner speak. A flame thrower whose attributes make its
//! airblast push its owner instead calls `DeflectPlayer` with the owner as
//! both players.
//!
//! `CTFWeaponBase`'s own `DeflectEntity` sends what it is given back where
//! the owner aims, and hands a projectile to the owner. The flame throwers'
//! override, which the Dragon's Fury inherits too, leaves alone the
//! projectiles of the owner's team, and every projectile for a flame thrower
//! whose attributes forbid deflecting them. It destroys them for one whose
//! attributes say so, and otherwise deflects them as `CTFWeaponBase`'s does.

use crate::interfaces::CreateInterfaceFn;
use crate::util::{self, Image};
use crate::vtable_slot;
use std::ffi::c_void;
use std::ptr::NonNull;

/// The signature of `CTFWeaponBase::DeflectEntity`,
/// `bool (CBaseEntity *, CTFPlayer *, Vector &)`, with the weapon as its
/// receiver, the entity other than a player its airblast reached as `target`,
/// and the weapon's owner as `owner`, which returns whether it deflected the
/// entity. `forward` is where the owner aims, which the function only reads.
///
/// As for [`DeflectPlayerFn`], the receiver and the owner are entities.
#[doc(alias("DeflectEntity"))]
pub type DeflectEntityFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	target: *mut sys::CBaseEntity,
	owner: *mut sys::CBaseEntity,
	forward: *mut sys::Vector,
) -> bool;

/// The signature of `CTFWeaponBase::DeflectPlayer`,
/// `bool (CTFPlayer *, CTFPlayer *, Vector &)`, with the weapon as its
/// receiver, the player its airblast reached as `target`, and the weapon's
/// owner as `owner`, which returns whether it pushed the player. `forward` is
/// where the owner aims, which the function only reads.
///
/// The generated method takes a `CTFWeaponBase` receiver and `CTFPlayer`
/// players. They are the weapon's and the players' entities, at the same
/// addresses, as [`weapons`](crate::tf2::weapons) and [the `tf2`
/// module](crate::tf2) assert, so the method can be called and hooked with
/// entities.
#[doc(alias("DeflectPlayer"))]
pub type DeflectPlayerFn = unsafe extern "C" fn(
	this: *mut sys::CBaseEntity,
	target: *mut sys::CBaseEntity,
	owner: *mut sys::CBaseEntity,
	forward: *mut sys::Vector,
) -> bool;

/// The signature of `CTFWeaponBase::DeflectProjectiles`, `bool ()`, with the
/// weapon as its receiver, which runs its airblast. As for
/// [`DeflectPlayerFn`], the receiver is the weapon's entity.
#[doc(alias("DeflectProjectiles"))]
pub type DeflectProjectilesFn = unsafe extern "C" fn(this: *mut sys::CBaseEntity) -> bool;

// `CTFWeaponBase` (`game/shared/tf/tf_weaponbase.h`) declares, for the game
// server (`GAME_DLL`), `DeflectProjectiles()`, then
// `DeflectPlayer(CTFPlayer *, CTFPlayer *, Vector &)` and
// `DeflectEntity(CBaseEntity *, CTFPlayer *, Vector &)` as virtual methods of
// its own, which the generated vtables put at 422 to 424 on Windows, and at
// 429 to 431 on Linux. Linux is one past Windows for the Itanium ABI's
// second destructor slot, five more for the methods of `IHasAttributes` that
// `CEconEntity` overrides, and one more for `GetOwnerViaInterface`, which
// `CTFWeaponBase` overrides from `IHasOwner`: that ABI also gives each
// override of a secondary base's method a slot of the primary vtable. The
// layouts match SourceMod's gamedata as far as `CBaseCombatWeapon::Reload`, at
// 284 and 290, but no binary confirmed these slots. So before hooking,
// `metamod_source`'s airblast hooks check that `DeflectProjectiles`, which no
// class overrides, is one function in the vtables of both classes that push
// and of [`REFERENCE_CLASS`], and that `DeflectPlayer` is one function in the
// classes that push, and another in the reference, which keeps
// `CTFWeaponBase`'s. The hooks of `DeflectEntity` also check that it is laid
// out as `DeflectPlayer` is, with functions of its own.
const _: () = {
	let deflect_entity = vtable_slot!(
		sys::CTFWeaponBase__bindgen_vtable,
		CTFWeaponBase_DeflectEntity
	);
	let deflect_player = vtable_slot!(
		sys::CTFWeaponBase__bindgen_vtable,
		CTFWeaponBase_DeflectPlayer
	);
	let deflect_projectiles = vtable_slot!(
		sys::CTFWeaponBase__bindgen_vtable,
		CTFWeaponBase_DeflectProjectiles
	);

	assert!(
		DEFLECT_ENTITY_SLOT == deflect_entity
			&& DEFLECT_PLAYER_SLOT == deflect_player
			&& DEFLECT_PROJECTILES_SLOT == deflect_projectiles
	);
};

// The generated methods have these parameters and return a `bool`.
const _: fn(
	&sys::CTFWeaponBase__bindgen_vtable,
) -> unsafe extern "C" fn(
	*mut sys::CTFWeaponBase,
	*mut sys::CBaseEntity,
	*mut sys::CTFPlayer,
	*mut sys::Vector,
) -> bool = |vtable| vtable.CTFWeaponBase_DeflectEntity;

const _: fn(
	&sys::CTFWeaponBase__bindgen_vtable,
) -> unsafe extern "C" fn(
	*mut sys::CTFWeaponBase,
	*mut sys::CTFPlayer,
	*mut sys::CTFPlayer,
	*mut sys::Vector,
) -> bool = |vtable| vtable.CTFWeaponBase_DeflectPlayer;

const _: fn(
	&sys::CTFWeaponBase__bindgen_vtable,
) -> unsafe extern "C" fn(*mut sys::CTFWeaponBase) -> bool =
	|vtable| vtable.CTFWeaponBase_DeflectProjectiles;

/// The slot of `CTFWeaponBase::DeflectEntity` in a TF2 weapon's primary
/// vtable, just after [`DEFLECT_PLAYER_SLOT`]. [`FLAME_THROWER_CLASS`]
/// overrides it, and every other weapon class but [`DRAGONS_FURY_CLASS`],
/// which inherits the override, keeps `CTFWeaponBase`'s.
#[doc(alias("DeflectEntity"))]
pub const DEFLECT_ENTITY_SLOT: usize = DEFLECT_PLAYER_SLOT + 1;

/// The slot of `CTFWeaponBase::DeflectPlayer` in a TF2 weapon's primary
/// vtable. [`FLAME_THROWER_CLASS`] overrides it, and every other weapon class
/// but [`DRAGONS_FURY_CLASS`], which inherits the override, keeps
/// `CTFWeaponBase`'s.
#[doc(alias("DeflectPlayer"))]
pub const DEFLECT_PLAYER_SLOT: usize = cfg_select! {
	target_os = "windows" => 423,
	target_os = "linux" => 430,
};

/// The slot of `CTFWeaponBase::DeflectProjectiles` in a TF2 weapon's primary
/// vtable, just before [`DEFLECT_PLAYER_SLOT`]. Every weapon class keeps
/// `CTFWeaponBase`'s.
#[doc(alias("DeflectProjectiles"))]
pub const DEFLECT_PROJECTILES_SLOT: usize = DEFLECT_PLAYER_SLOT - 1;

/// The name of the class of the Dragon's Fury
/// (`tf_weapon_rocketlauncher_fireball`), which derives from
/// [`FLAME_THROWER_CLASS`] and keeps its `DeflectPlayer`, in the run-time type
/// information.
pub const DRAGONS_FURY_CLASS: &str = "CTFWeaponFlameBall";

/// The name of the class of the Pyro's flame throwers
/// (`tf_weapon_flamethrower`), which overrides `DeflectPlayer`, in the
/// run-time type information.
pub const FLAME_THROWER_CLASS: &str = "CTFFlameThrower";

/// The name of the class of the Soldier's rocket launchers
/// (`tf_weapon_rocketlauncher`), in the run-time type information. It
/// overrides neither `DeflectProjectiles` nor `DeflectPlayer`, so its vtable
/// holds `CTFWeaponBase`'s, to compare those of the classes that push with.
pub const REFERENCE_CLASS: &str = "CTFRocketLauncher";

/// Finds the unique primary vtables of [`FLAME_THROWER_CLASS`],
/// [`DRAGONS_FURY_CLASS`] and [`REFERENCE_CLASS`], in that order, whose
/// [`DEFLECT_ENTITY_SLOT`] entries are executable, from the run-time type
/// information of the module whose `CreateInterface` export is `factory`, such
/// as the game server module. Each is `None` if there is no such table, or
/// more than one. The module is snapshot and searched once for all three.
///
/// The search does not check that the classes derive from `CTFWeaponBase`,
/// so that the slots hold its methods: any class with that many virtual
/// methods passes. The addresses are metadata from the snapshot: they do not
/// keep the module loaded, and the tables are the classes' only while it
/// stays loaded.
///
/// # Safety
///
/// `factory` must be the `CreateInterface` export of a module that stays
/// loaded throughout this call.
pub unsafe fn find_airblast_vtables(
	factory: CreateInterfaceFn,
) -> Result<[Option<NonNull<*mut c_void>>; 3], util::Error> {
	// SAFETY: The factory is an executable address in its module, which the
	// caller keeps loaded while it is inspected.
	let image = unsafe { Image::load(factory as usize) }?;
	let mut tables = image
		.primary_vtables(
			&[FLAME_THROWER_CLASS, DRAGONS_FURY_CLASS, REFERENCE_CLASS],
			DEFLECT_ENTITY_SLOT,
		)
		.into_iter()
		.map(|table| NonNull::new(table? as *mut *mut c_void));

	Ok([
		tables.next().flatten(),
		tables.next().flatten(),
		tables.next().flatten(),
	])
}
