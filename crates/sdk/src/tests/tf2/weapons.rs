//! Tests of TF2 weapon inventories through the native methods of fake players
//! and weapons.

use super::*;
use crate::Module;
use crate::interfaces::ServerTools;
use crate::test_support::entities::{MOCK_EFLAGS_OFFSET, base_entity_fields};
use crate::test_support::server::{export, mock_server};
use crate::tf2::attributes::{Multiplier, catalog, trust_shipped_schema};
use sdk_raw::test_support::entities::{data_map, field};
use sdk_raw::test_support::{mock_vtable, unexpected_call};
use std::cell::Cell;
use std::ffi::c_char;
use std::mem::{offset_of, size_of};
use std::ptr::null_mut;

const EQUIP: usize =
	offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_Weapon_Equip) / size_of::<usize>();

const GET_SLOT: usize =
	offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_Weapon_GetSlot) / size_of::<usize>();

const GIVE: usize =
	offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_GiveNamedItem1) / size_of::<usize>();

const REMOVE: usize =
	offset_of!(sys::CTFPlayer__bindgen_vtable, CTFPlayer_RemovePlayerItem) / size_of::<usize>();

const WEAPON_SLOT: usize =
	offset_of!(sys::CTFWeaponBase__bindgen_vtable, CTFWeaponBase_GetSlot) / size_of::<usize>();

#[repr(C)]
struct FakeEntity {
	vtable: *const *const (),
	map: *mut sys::datamap_t,
	weapon: *mut sys::CBaseEntity,
	slot: i32,
	padding: i32,
	flags: i32,
	owner: u32,
	handle: u32,
	owner_entity: u32,
	second_weapon: *mut sys::CBaseEntity,
	equip_calls: usize,
}

thread_local! {
	static GIVE_RESULT: Cell<*mut sys::CBaseEntity> = const { Cell::new(null_mut()) };
	static INVENTORY_FULL: Cell<bool> = const { Cell::new(false) };
	static REJECT_DETACH: Cell<bool> = const { Cell::new(false) };
	static NETWORKABLE: Cell<*mut sys::IServerNetworkable> = const { Cell::new(null_mut()) };
}

unsafe extern "C" fn classname(_: *const sys::IServerNetworkable) -> *const c_char {
	c"tf_weapon_bottle".as_ptr()
}

unsafe extern "C" fn datamap(entity: *mut sys::CBaseEntity) -> *mut sys::datamap_t {
	// SAFETY: Only fake entities have this method in their vtables.
	unsafe { (*entity.cast::<FakeEntity>()).map }
}

unsafe extern "C" fn detach(
	player: *mut sys::CTFPlayer,
	weapon: *mut sys::CBaseCombatWeapon,
) -> bool {
	if REJECT_DETACH.get() {
		return false;
	}
	// SAFETY: The wrappers pass the fake player and a fake weapon, which
	// outlive the call.
	unsafe {
		let player = player.cast::<FakeEntity>();
		let weapon = weapon.cast::<sys::CBaseEntity>();
		if (*player).weapon == weapon {
			(*player).weapon = null_mut();
		} else if (*player).second_weapon == weapon {
			(*player).second_weapon = null_mut();
		} else {
			return false;
		}
		// Matches native Weapon_Detach: m_hOwnerEntity deliberately stays set.
		(*weapon.cast::<FakeEntity>()).owner = EntityHandle::INVALID.to_raw();
		true
	}
}

unsafe extern "C" fn equip(player: *mut sys::CTFPlayer, weapon: *mut sys::CBaseCombatWeapon) {
	// SAFETY: As for `detach`.
	unsafe {
		let player = player.cast::<FakeEntity>();
		let weapon = weapon.cast::<sys::CBaseEntity>();
		(*player).equip_calls += 1;
		if !INVENTORY_FULL.get() {
			if (*player).weapon.is_null() {
				(*player).weapon = weapon;
			} else {
				(*player).second_weapon = weapon;
			}
		}
		(*weapon.cast::<FakeEntity>()).owner = (*player).handle;
		(*weapon.cast::<FakeEntity>()).owner_entity = (*player).handle;
	}
}

#[test]
fn failed_replacement_restores_inventory_and_rejected_pickup_does_not_leak_weapon() {
	let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());
	let player_map = data_map(c"CTFPlayer", vec![], base);
	let weapon_map = weapon_map(base);
	let mut player_table = vec![std::ptr::null(); GIVE + 1];
	player_table[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
	player_table[GET_SLOT] = inventory_slot as *const ();
	player_table[GIVE] = give as *const ();
	player_table[EQUIP] = equip as *const ();
	player_table[REMOVE] = detach as *const ();
	let handle_slot = offset_of!(
		sys::IServerUnknown__bindgen_vtable,
		IServerUnknown_GetRefEHandle
	) / size_of::<usize>();
	player_table[handle_slot] = handle as *const ();
	let mut weapon_table = vec![std::ptr::null(); WEAPON_SLOT + 1];
	weapon_table[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
	weapon_table[WEAPON_SLOT] = weapon_slot as *const ();
	weapon_table[handle_slot] = handle as *const ();
	let networkable_slot = offset_of!(
		sys::IServerUnknown__bindgen_vtable,
		IServerUnknown_GetNetworkable
	) / size_of::<usize>();
	weapon_table[networkable_slot] = networkable as *const ();
	// SAFETY: The vtable holds only function pointers, `unexpected_call`
	// aborts whichever slot reaches it, and the patch only writes a slot of
	// the vtable being built.
	let networkable_vtable = unsafe {
		mock_vtable::<sys::IServerNetworkable__bindgen_vtable>(
			unexpected_call as *const (),
			|vtable| {
				(&raw mut (*vtable).IServerNetworkable_GetClassName).write(classname);
			},
		)
	};
	let mut networkable = sys::IServerNetworkable {
		vtable_: &*networkable_vtable,
	};
	NETWORKABLE.set(&raw mut networkable);

	let mut old = FakeEntity {
		vtable: weapon_table.as_ptr(),
		map: weapon_map,
		weapon: null_mut(),
		slot: 2,
		padding: 0,
		flags: 0,
		owner: 1,
		handle: 2,
		owner_entity: 1,
		second_weapon: null_mut(),
		equip_calls: 0,
	};

	let old_ptr = (&raw mut old).cast();
	let mut fresh = FakeEntity {
		vtable: weapon_table.as_ptr(),
		map: weapon_map,
		weapon: null_mut(),
		slot: 2,
		padding: 0,
		flags: 0,
		owner: EntityHandle::INVALID.to_raw(),
		handle: 3,
		owner_entity: 0,
		second_weapon: null_mut(),
		equip_calls: 0,
	};
	let mut player = FakeEntity {
		vtable: player_table.as_ptr(),
		map: player_map,
		weapon: old_ptr,
		slot: 2,
		padding: 0,
		flags: 0,
		owner: 0,
		handle: 1,
		owner_entity: 0,
		second_weapon: null_mut(),
		equip_calls: 0,
	};
	// SAFETY: As for the networkable's vtable.
	let tools_vtable = unsafe {
		mock_vtable::<sys::IServerTools__bindgen_vtable>(unexpected_call as *const (), |vtable| {
			(&raw mut (*vtable).IServerTools_RemoveEntity).write(remove);
		})
	};
	let mut tools = sys::IServerTools {
		vtable_: &*tools_vtable,
	};
	export(Module::GameServer, ServerTools::VERSION, &raw mut tools);
	GIVE_RESULT.set(null_mut());
	let scope = ();
	let server = mock_server(&scope);
	// SAFETY: The fake player outlives the entity's use, and its vtable
	// answers what the wrappers call of a `CTFPlayer`.
	let entity = unsafe { Entity::from_raw(NonNull::from(&mut player).cast()) };
	let inventory = PlayerWeapons::new(server, entity).unwrap();
	let weapon = inventory.get_slot(WeaponSlot::Melee).unwrap().unwrap();
	inventory.detach(weapon).unwrap();
	assert_eq!(weapon.owner().unwrap(), None);
	assert_eq!(
		old.owner_entity, 1,
		"native detach leaves the unrelated base owner intact"
	);
	assert!(inventory.get_slot(WeaponSlot::Melee).unwrap().is_none());
	inventory.equip(weapon).unwrap();
	assert_eq!(
		inventory
			.get_slot(WeaponSlot::Melee)
			.unwrap()
			.unwrap()
			.entity
			.as_ptr(),
		old_ptr
	);
	assert_eq!(player.equip_calls, 1);
	// Native creation failure must restore the previously detached weapon.
	assert!(matches!(
		// SAFETY: The mock game's native methods only record what they do,
		// and delete nothing immediately. The same holds for every creation
		// below.
		unsafe { inventory.replace(WeaponSlot::Melee, c"tf_weapon_missing", 0) },
		Err(WeaponError::CreationFailed)
	));
	assert_eq!(
		inventory
			.get_slot(WeaponSlot::Melee)
			.unwrap()
			.unwrap()
			.entity
			.as_ptr(),
		old_ptr
	);
	assert_eq!(weapon.owner().unwrap(), Some(entity.handle()));
	assert_eq!(player.equip_calls, 2);
	assert_eq!(old.flags, 0);
	// Matching owner alone cannot short-circuit reconciliation.
	// SAFETY: The field is the fake player's own, written in place as the
	// game would. The same holds for the fake entities' fields below.
	unsafe { (&raw mut player.weapon).write(null_mut()) };
	inventory.equip(weapon).unwrap();
	assert_eq!(
		inventory
			.get_slot(WeaponSlot::Melee)
			.unwrap()
			.unwrap()
			.entity
			.as_ptr(),
		old_ptr
	);
	assert_eq!(player.equip_calls, 3);

	// Touch equips the fresh weapon before GiveNamedItem returns, and the
	// native slot getter even returns it first. The snapshot must catch it.
	GIVE_RESULT.set((&raw mut fresh).cast());
	assert!(matches!(
		// SAFETY: The mock `GiveNamedItem` returns the fake weapon, which
		// outlives the test, and its pickup only records it.
		unsafe { inventory.give(c"tf_weapon_bottle", 0) },
		Err(WeaponError::SlotOccupied)
	));
	assert_eq!(
		inventory
			.get_slot(WeaponSlot::Melee)
			.unwrap()
			.unwrap()
			.entity
			.as_ptr(),
		old_ptr
	);
	assert!(player.second_weapon.is_null());
	assert_eq!(fresh.owner, EntityHandle::INVALID.to_raw());
	assert_eq!(fresh.flags, 1, "the rejected new entity must be removed");
	assert_eq!(old.flags, 0);

	// A full native inventory can set ownership without recording the new
	// weapon. Detach then fails, but cleanup must still delete that entity.
	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut player.weapon).write(null_mut());
		(&raw mut fresh.flags).write(0);
	}
	INVENTORY_FULL.set(true);
	assert!(matches!(
		// SAFETY: As for the first `give`.
		unsafe { inventory.give(c"tf_weapon_bottle", 0) },
		Err(WeaponError::Rejected)
	));
	assert!(inventory.get_slot(WeaponSlot::Melee).unwrap().is_none());
	assert_eq!(
		fresh.flags, 1,
		"failed native detach must not leak the new entity"
	);
	INVENTORY_FULL.set(false);

	// Item generation returns a spawned, unequipped entity. Exercise that
	// path independently of stock GiveNamedItem's implicit Touch pickup.
	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut player.weapon).write(old_ptr);
		(&raw mut fresh.flags).write(0);
		(&raw mut fresh.owner).write(EntityHandle::INVALID.to_raw());
	}
	let fresh_ptr = NonNull::from(&mut fresh).cast();
	assert!(matches!(
		// SAFETY: The fake weapon is live through the call, as if newly
		// created. The same holds for every `give_with` below.
		unsafe { inventory.give_with(None, || Ok(fresh_ptr), |_| Ok(())) },
		Err(WeaponError::SlotOccupied)
	));
	assert_eq!(player.weapon, old_ptr);
	assert_eq!(fresh.flags, 1);
	assert_eq!(old.flags, 0);

	assert!(matches!(
		inventory.replace_with(WeaponSlot::Melee, || Err(WeaponError::CreationFailed)),
		Err(WeaponError::CreationFailed)
	));
	assert_eq!(
		player.weapon, old_ptr,
		"missing item definition restores the old weapon"
	);

	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut fresh.flags).write(0);
		(&raw mut fresh.slot).write(0);
	}
	assert!(matches!(
		inventory.replace_with(WeaponSlot::Melee, || {
			// SAFETY: As for the first `give_with`.
			unsafe { inventory.give_with(None, || Ok(fresh_ptr), |_| Ok(())) }
		}),
		Err(WeaponError::WrongSlot)
	));
	assert_eq!(
		player.weapon, old_ptr,
		"wrong item slot restores the old weapon"
	);
	assert_eq!(fresh.flags, 1, "wrong-slot item is removed");
	assert_eq!(fresh.owner, EntityHandle::INVALID.to_raw());

	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut fresh.flags).write(0);
	}
	let rejected_detach = inventory.replace_with(WeaponSlot::Melee, || {
		// SAFETY: As for the first `give_with`.
		let replacement = unsafe { inventory.give_with(None, || Ok(fresh_ptr), |_| Ok(())) }?;
		REJECT_DETACH.set(true);
		Ok(replacement)
	});
	REJECT_DETACH.set(false);
	assert!(matches!(rejected_detach, Err(WeaponError::Rejected)));
	assert_eq!(
		fresh.flags, 1,
		"wrong-slot item is removed even when detach rejects"
	);
	assert_eq!(
		inventory
			.get_slot(WeaponSlot::Melee)
			.unwrap()
			.unwrap()
			.entity()
			.as_ptr(),
		old_ptr
	);

	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut player.weapon).write(old_ptr);
		(&raw mut player.second_weapon).write(null_mut());
		(&raw mut fresh.flags).write(0);
		(&raw mut fresh.owner).write(EntityHandle::INVALID.to_raw());
		(&raw mut fresh.map).write(data_map(c"CEconWearable", vec![], base));
	}
	assert!(matches!(
		inventory.replace_with(WeaponSlot::Melee, || {
			// SAFETY: As for the first `give_with`.
			unsafe { inventory.give_with(None, || Ok(fresh_ptr), |_| Ok(())) }
		}),
		Err(WeaponError::NotWeapon)
	));
	assert_eq!(
		player.weapon, old_ptr,
		"cosmetic item restores the old weapon"
	);
	assert_eq!(fresh.flags, 1, "nonweapon item is removed");
	assert_eq!(old.flags, 0);

	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut fresh.flags).write(0);
		(&raw mut fresh.map).write(weapon_map);
		(&raw mut fresh.slot).write(2);
	}
	assert!(matches!(
		inventory.replace_with(WeaponSlot::Melee, || {
			// SAFETY: As for the first `give_with`.
			unsafe {
				inventory.give_with(Some(c"tf_weapon_sdk_missing"), || Ok(fresh_ptr), |_| Ok(()))
			}
		}),
		Err(WeaponError::CreationFailed)
	));
	assert_eq!(fresh.flags, 1, "native classname fallback must be removed");
	assert_eq!(
		player.weapon, old_ptr,
		"override mismatch restores the old weapon"
	);
	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut fresh.flags).write(0);
	}
	let replacement = inventory
		.replace_with(WeaponSlot::Melee, || {
			// SAFETY: As for the first `give_with`.
			unsafe { inventory.give_with(Some(c"tf_weapon_bottle"), || Ok(fresh_ptr), |_| Ok(())) }
		})
		.unwrap();
	assert_eq!(replacement.entity().as_ptr(), fresh_ptr.as_ptr());
	assert_eq!(player.weapon, fresh_ptr.as_ptr());
	assert_eq!(fresh.owner, 1);
	assert_eq!(fresh.flags, 0);
	assert_eq!(
		old.flags, 1,
		"successful economy replacement removes the old weapon"
	);

	// Native slots outside 0..=255 still equip, but creation cannot snapshot
	// their occupancy, so it refuses such a new weapon and removes it.
	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut old.flags).write(0);
		(&raw mut old.slot).write(300);
	}
	inventory.equip(weapon).unwrap();
	assert_eq!(
		inventory.get_slot(300).unwrap().unwrap().entity().as_ptr(),
		old_ptr
	);
	inventory.detach(weapon).unwrap();
	let untracked = NonNull::new(old_ptr).unwrap();
	assert!(matches!(
		// SAFETY: As for the first `give_with`.
		unsafe { inventory.give_with(None, || Ok(untracked), |_| Ok(())) },
		Err(WeaponError::WrongSlot)
	));
	assert_eq!(old.flags, 1, "an untracked-slot weapon is removed");

	// A failed step after equipping, such as applying attributes, detaches
	// and removes the new weapon, which was already equipped.
	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut player.weapon).write(null_mut());
		(&raw mut player.second_weapon).write(null_mut());
		(&raw mut fresh.flags).write(0);
		(&raw mut fresh.owner).write(EntityHandle::INVALID.to_raw());
		(&raw mut fresh.slot).write(2);
	}
	assert!(matches!(
		// SAFETY: As for the first `give_with`.
		unsafe {
			inventory.give_with(
				None,
				|| Ok(fresh_ptr),
				|weapon| {
					assert_eq!(weapon.owner().unwrap(), Some(entity.handle()));
					Err(WeaponError::Rejected)
				},
			)
		},
		Err(WeaponError::Rejected)
	));
	assert!(player.weapon.is_null(), "the failed weapon is detached");
	assert_eq!(fresh.owner, EntityHandle::INVALID.to_raw());
	assert_eq!(fresh.flags, 1, "the failed weapon is removed");

	// Attribute failures keep their cause and clean up the same way.
	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut fresh.flags).write(0);
		(&raw mut fresh.owner).write(EntityHandle::INVALID.to_raw());
	}
	assert!(matches!(
		// SAFETY: As for the first `give_with`.
		unsafe {
			inventory.give_with(
				None,
				|| Ok(fresh_ptr),
				|_| Err(AttributeError::RuntimeListFull.into()),
			)
		},
		Err(WeaponError::Attribute(AttributeError::RuntimeListFull))
	));
	assert!(player.weapon.is_null(), "the failed weapon is detached");
	assert_eq!(fresh.owner, EntityHandle::INVALID.to_raw());
	assert_eq!(fresh.flags, 1, "the failed weapon is removed");

	// `give_item_with`'s own step needs the weapon's item attributes, which
	// this weapon's datamaps lack.
	// SAFETY: No attribute is written: the fake weapon has no item
	// attributes to write to.
	let token = unsafe { trust_shipped_schema(server) };
	let set = AttributeSet::new()
		.with(&catalog::DAMAGE_BONUS, Multiplier::new(2.0).unwrap())
		.unwrap();

	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut fresh.flags).write(0);
		(&raw mut fresh.owner).write(EntityHandle::INVALID.to_raw());
	}
	assert!(matches!(
		// SAFETY: As for the first `give_with`.
		unsafe {
			inventory.give_with(
				None,
				|| Ok(fresh_ptr),
				|weapon| apply_attributes(token, weapon, &set),
			)
		},
		Err(WeaponError::Attribute(AttributeError::UnsupportedEntity))
	));
	assert!(player.weapon.is_null(), "the failed weapon is detached");
	assert_eq!(fresh.flags, 1, "the failed weapon is removed");

	// An empty set leaves the weapon as `give_item` would.
	// SAFETY: As for the player's fields above.
	unsafe {
		(&raw mut fresh.flags).write(0);
		(&raw mut fresh.owner).write(EntityHandle::INVALID.to_raw());
	}
	// SAFETY: As for the first `give_with`.
	let given = unsafe {
		inventory.give_with(
			None,
			|| Ok(fresh_ptr),
			|weapon| apply_attributes(token, weapon, &AttributeSet::new()),
		)
	}
	.unwrap();
	assert_eq!(given.entity().as_ptr(), fresh_ptr.as_ptr());
	assert_eq!(player.weapon, fresh_ptr.as_ptr());
	assert_eq!(fresh.flags, 0);
	GIVE_RESULT.set(null_mut());
	NETWORKABLE.set(null_mut());
}

unsafe extern "C" fn give(
	player: *mut sys::CTFPlayer,
	_: *const c_char,
	_: i32,
	item: *const sys::CEconItemView,
	force: bool,
) -> *mut sys::CBaseEntity {
	assert!(
		item.is_null(),
		"stock generation requires a null CEconItemView"
	);
	assert!(force, "the requested classname must not be translated");
	let weapon = GIVE_RESULT.get();
	if !weapon.is_null() {
		// SAFETY: The wrappers pass the fake player, and the test sets a fake
		// weapon to give.
		unsafe { equip(player, weapon.cast()) };
	}
	weapon
}

unsafe extern "C" fn handle(entity: *const sys::IServerUnknown) -> *const sys::CBaseHandle {
	// SAFETY: Only fake entities have this method in their vtables.
	unsafe { (&raw const (*entity.cast::<FakeEntity>()).handle).cast() }
}

unsafe extern "C" fn inventory_slot(
	entity: *const sys::CTFPlayer,
	slot: i32,
) -> *mut sys::CBaseCombatWeapon {
	// SAFETY: The wrappers pass the fake player, whose weapons are fake
	// entities or null.
	unsafe {
		let entity = entity.cast::<FakeEntity>();
		for weapon in [(*entity).second_weapon, (*entity).weapon] {
			if !weapon.is_null() && (*weapon.cast::<FakeEntity>()).slot == slot {
				return weapon.cast();
			}
		}
		null_mut()
	}
}

#[test]
fn native_inventory_slots_use_validated_classes_and_refuse_deleted_entities() {
	assert_eq!(std::mem::offset_of!(FakeEntity, flags), MOCK_EFLAGS_OFFSET);
	let base = data_map(c"CBaseEntity", Vec::from(base_entity_fields()), null_mut());
	let player_map = data_map(c"CTFPlayer", vec![], base);
	let weapon_map = weapon_map(base);
	let mut player_table = vec![std::ptr::null(); GIVE + 1];
	player_table[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
	player_table[GET_SLOT] = inventory_slot as *const ();
	let mut weapon_table = vec![std::ptr::null(); WEAPON_SLOT + 1];
	weapon_table[sdk_raw::entities::GET_DATA_DESC_MAP_SLOT] = datamap as *const ();
	weapon_table[WEAPON_SLOT] = weapon_slot as *const ();
	let mut weapon = FakeEntity {
		vtable: weapon_table.as_ptr(),
		map: weapon_map,
		weapon: null_mut(),
		slot: 2,
		padding: 0,
		flags: 0,
		owner: 1,
		handle: 2,
		owner_entity: 1,
		second_weapon: null_mut(),
		equip_calls: 0,
	};
	let raw_weapon = (&raw mut weapon).cast();
	let mut player = FakeEntity {
		vtable: player_table.as_ptr(),
		map: player_map,
		weapon: raw_weapon,
		slot: 2,
		padding: 0,
		flags: 0,
		owner: 0,
		handle: 1,
		owner_entity: 0,
		second_weapon: null_mut(),
		equip_calls: 0,
	};
	let scope = ();
	let server = mock_server(&scope);
	// SAFETY: The fake player outlives the entity's use, and its vtable
	// answers what the wrappers call of a `CTFPlayer`.
	let entity = unsafe { Entity::from_raw(NonNull::from(&mut player).cast()) };
	let inventory = PlayerWeapons::new(server, entity).unwrap();
	let found = inventory.get_slot(WeaponSlot::Melee).unwrap().unwrap();
	assert_eq!(found.entity.as_ptr(), raw_weapon);
	assert_eq!(found.slot().unwrap(), Some(WeaponSlot::Melee));
	assert!(inventory.get_slot(WeaponSlot::Primary).unwrap().is_none());
	assert!(matches!(
		PlayerWeapons::new(server, found.entity()),
		Err(WeaponError::NotTfPlayer)
	));
	assert!(matches!(
		Weapon::new(server, entity),
		Err(WeaponError::NotWeapon)
	));
	// Slots outside the named six stay reachable through raw numbers.
	// SAFETY: The field is the fake weapon's own, written in place as the
	// game would.
	unsafe { (&raw mut weapon.slot).write(7) };
	let uncommon = inventory.get_slot(7).unwrap().unwrap();
	assert_eq!(uncommon.slot_raw().unwrap(), 7);
	assert_eq!(uncommon.slot().unwrap(), None);
	assert!(inventory.get_slot(WeaponSlot::Melee).unwrap().is_none());
	// SAFETY: As for the weapon's slot.
	unsafe { (&raw mut player.flags).write(1) };
	assert!(matches!(
		inventory.get_slot(WeaponSlot::Melee),
		Err(WeaponError::MarkedForDeletion)
	));
}

unsafe extern "C" fn networkable(_: *mut sys::IServerUnknown) -> *mut sys::IServerNetworkable {
	NETWORKABLE.get()
}

unsafe extern "C" fn remove(_: *mut sys::IServerTools, entity: *mut sys::CBaseEntity) {
	// SAFETY: The wrappers only remove fake entities.
	unsafe {
		(*entity.cast::<FakeEntity>()).flags |= 1;
	}
}

fn weapon_map(base: *mut sys::datamap_t) -> *mut sys::datamap_t {
	let mut owner = field(
		c"m_hOwner",
		sys::_fieldtypes_FIELD_EHANDLE,
		offset_of!(FakeEntity, owner),
	);
	owner.fieldSize = 1;
	owner.fieldSizeInBytes = 4;
	let combat = data_map(c"CBaseCombatWeapon", vec![owner], base);

	data_map(c"CTFWeaponBase", vec![], combat)
}

unsafe extern "C" fn weapon_slot(entity: *const sys::CTFWeaponBase) -> i32 {
	// SAFETY: Only fake weapons have this method in their vtables.
	unsafe { (*entity.cast::<FakeEntity>()).slot }
}
